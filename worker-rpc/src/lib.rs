use std::{collections::HashMap, marker::PhantomData};

use futures_channel::{oneshot, mpsc};
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered, future::RemoteHandle};
use gloo_events::EventListener;
use js_sys::{Uint8Array, Array, ArrayBuffer};
use serde::{Serialize, Deserialize};
use wasm_bindgen::{JsValue, JsCast};
use web_sys::MessageEvent;

pub use worker_rpc_macro::service;
pub use futures_channel;
pub use futures_util;
pub use serde;
pub use js_sys;
pub use wasm_bindgen;
pub use pin_utils;

pub mod client;
pub mod server;

#[derive(Debug)]
pub enum Error {
    EncodeDecode(bincode::ErrorKind),
    Aborted,
}
pub type Result<T> = std::result::Result<T, Error>;

impl<I: SendRecv> From<I> for Interface<client::None, server::None, I> {
    fn from(interface: I) -> Self {
        Self {
            interface,
            client: PhantomData::<client::None>,
            server: server::None,
        }
    }
}

#[derive(Serialize, Deserialize)]
enum Message<Request, Response> {
    Request(u32, Request),
    Abort(u32),
    Response(u32, Response),
}

pub struct Interface<C, S, I> {
    client: PhantomData<C>,
    server: S,
    interface: I,
}

impl<C, I> Interface<C, server::None, I> {
    pub fn with_server<S: server::Server>(
        self,
        server: S
    ) -> Interface<C, S, I> {
        let Interface { interface, client, .. } = self;
        Interface { interface, client, server }
    }
}

impl<S, I> Interface<client::None, S, I> {
    pub fn with_client<C: client::Client>(
        self,
    ) -> Interface<C, S, I> {
        let Interface { interface, server, .. } = self;
        Interface { interface, client: PhantomData::<C>, server }
    }
}

impl<C, S, I> Interface<C, S, I> where
    C: client::Client + From<client::RequestSender<C>> + 'static,
    S: server::Server + 'static,
    I: SendRecv + 'static {

    pub fn connect(self) -> ConnectedInterface<C> {
        let (client_requests_tx, client_requests_rx) = mpsc::unbounded();
        let Interface { server, interface, .. } = self;
        let (task, _task_handle) = run::<C, S, I>(client_requests_rx, server, interface)
            .remote_handle();
        wasm_bindgen_futures::spawn_local(task);
        ConnectedInterface {
            _task_handle,
            client: C::from(client_requests_tx),
        }
    }
}

pub struct ConnectedInterface<C> {
    _task_handle: RemoteHandle<()>,
    client: C,
}

impl<C> std::ops::Deref for ConnectedInterface<C> {
    type Target = C;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

async fn run<C, S, I>(
    mut client_requests_rx: client::RequestReceiver<C>,
    server: S,
    interface: I
) where
    C: client::Client,
    S: server::Server,
    I: SendRecv,
    <S as server::Server>::Request: 'static,
    <C as client::Client>::Response: 'static {
    /* set up the event listener */
    let (client_responses_tx, mut client_responses_rx) = mpsc::unbounded();
    let (server_requests_tx, mut server_requests_rx) = mpsc::unbounded();
    let (abort_requests_tx, mut abort_requests_rx) = mpsc::unbounded();
    let _listener = EventListener::new(interface.event_target(), "message", move |event| {
        let array = event.unchecked_ref::<MessageEvent>()
            .data()
            .dyn_into::<Array>()
            .unwrap();
        let message = Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap())
            .to_vec();
        match bincode::deserialize::<Message<S::Request, C::Response>>(&message).unwrap() {
            Message::Response(seq_id, response) =>
                client_responses_tx.unbounded_send((seq_id, response, array)).unwrap(),
            Message::Request(seq_id, request) =>
                server_requests_tx.unbounded_send((seq_id, request, array)).unwrap(),
            Message::Abort(seq_id) =>
                abort_requests_tx.unbounded_send(seq_id).unwrap(),
        }
    });
    interface.prepare();

    let mut last_seq_id: u32 = 0;   
    let mut client_tasks: HashMap<u32, oneshot::Sender<_>> = Default::default();
    let mut server_tasks: HashMap<u32, oneshot::Sender<_>> = Default::default();
    let mut server_responses_rx: FuturesUnordered<_> = Default::default();
    let mut abort_requests: FuturesUnordered<_> = Default::default();

    loop {
        futures_util::select! {
            // SERVER SIDE LOGIC
            server_request = server_requests_rx.next() => {
                let (seq_id, request, post_args) = server_request.unwrap();
                let (abort_tx, abort_rx) = oneshot::channel::<()>();
                server_tasks.insert(seq_id, abort_tx);
                server_responses_rx.push(server.execute(seq_id, abort_rx, request, post_args));
            },
            abort_request = abort_requests_rx.next() => {
                if let Some(seq_id) = abort_request {
                    if let Some(abort_tx) = server_tasks.remove(&seq_id) {
                        /* this causes S::execute to terminate with None */
                        let _ = abort_tx.send(());
                    }
                }
            }
            server_response = server_responses_rx.next() => {
                if let Some((seq_id, response)) = server_response {
                    if server_tasks.remove(&seq_id).is_some() {
                        if let Some((response, post_args, transfer_args)) = response {
                            let response = Message::<C::Request, S::Response>::Response(seq_id, response);
                            let response = bincode::serialize(&response).unwrap();
                            let buffer = Uint8Array::from(&response[..]).buffer();
                            post_args.unshift(&buffer);
                            transfer_args.unshift(&buffer);
                            interface.post_message(&post_args, &transfer_args).unwrap();
                        }
                    }
                }
            },

            // CLIENT SIDE LOGIC
            client_request = client_requests_rx.next() => {
                if let Some((request, post_args, transfer_args, response_tx, abort_rx)) = client_request {
                    let seq_id = last_seq_id;
                    last_seq_id = last_seq_id.wrapping_add(1);
                    /* abort_rx will complete once RequestFuture is dropped */
                    abort_requests.push(abort_rx.map(move |_| seq_id));
                    client_tasks.insert(seq_id, response_tx);
                    let request = Message::<C::Request, S::Response>::Request(seq_id, request);
                    let request = bincode::serialize(&request).unwrap();
                    let buffer = js_sys::Uint8Array::from(&request[..]).buffer();
                    post_args.unshift(&buffer);
                    transfer_args.unshift(&buffer);
                    interface.post_message(&post_args, &transfer_args).unwrap();
                }
            },
            abort_request = abort_requests.next() => {
                if let Some(seq_id) = abort_request {
                    if client_tasks.remove(&seq_id).is_some() {
                        let abort_request = Message::<C::Request, S::Response>::Abort(seq_id);
                        let abort_request = bincode::serialize(&abort_request).unwrap();
                        let buffer = js_sys::Uint8Array::from(&abort_request[..]).buffer();
                        let post_args = js_sys::Array::of1(&buffer);
                        let transfer_args = js_sys::Array::of1(&buffer);
                        interface.post_message(&post_args, &transfer_args).unwrap();
                    }
                }
            },
            client_response = client_responses_rx.next() => {
                let (seq_id, response, js_args) = client_response.unwrap();
                if let Some(client_task) = client_tasks.remove(&seq_id) {
                    let _ = client_task.send((response, js_args));
                }
            }
        }
    }
}

pub trait SendRecv {
    fn event_target(&self) -> &web_sys::EventTarget;
    fn post_message(
        &self,
        message: &JsValue,
        transfer: &JsValue
    ) -> std::result::Result<(), JsValue>;
    fn prepare(&self) {}
}

impl SendRecv for web_sys::DedicatedWorkerGlobalScope {
    fn event_target(&self) -> &web_sys::EventTarget {
        self.as_ref()
    }
    fn post_message(
        &self,
        message: &JsValue,
        transfer: &JsValue
    ) -> std::result::Result<(), JsValue> {
        self.post_message_with_transfer(message, transfer)
    }
}

impl SendRecv for web_sys::Worker {
    fn event_target(&self) -> &web_sys::EventTarget {
        self.as_ref()
    }
    fn post_message(
        &self,
        message: &JsValue,
        transfer: &JsValue
    ) -> std::result::Result<(), JsValue> {
        self.post_message_with_transfer(message, transfer)
    }
    // TODO wait in prepare until worker has started?
}

impl SendRecv for web_sys::MessagePort {
    fn event_target(&self) -> &web_sys::EventTarget {
        self.as_ref()
    }
    fn post_message(
        &self,
        message: &JsValue,
        transfer: &JsValue
    ) -> std::result::Result<(), JsValue> {
        self.post_message_with_transferable(message, transfer)
    }
    fn prepare(&self) {
        self.start();
    }
}