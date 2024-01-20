use std::{collections::HashMap, marker::PhantomData};

use futures_channel::{oneshot, mpsc};
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered, future::RemoteHandle};
use gloo_events::EventListener;
use js_sys::{Uint8Array, Array, ArrayBuffer};
use serde::{Serialize, Deserialize, de::DeserializeOwned};
use wasm_bindgen::{JsValue, JsCast};
use web_sys::MessageEvent;

pub use worker_rpc_macro::service;
pub use futures_channel;
pub use serde;
pub use js_sys;
pub use wasm_bindgen;

// use wasm_bindgen::prelude::wasm_bindgen;
// #[wasm_bindgen]
// extern "C" {
//     #[wasm_bindgen(js_namespace = console)]
//     fn log(s: &str);

//     #[wasm_bindgen(js_namespace = console, js_name = log)]
//     fn log_jsvalue(v: &JsValue);
// }

#[derive(Debug)]
pub enum Error {
    EncodeDecode(bincode::ErrorKind),
    BadResponse,
}
pub type Result<T> = std::result::Result<T, Error>;

impl<I: SendRecv> From<I> for Interface<WithoutClient, WithoutServer, I> {
    fn from(interface: I) -> Self {
        Self {
            interface,
            client: PhantomData::<WithoutClient>,
            server: WithoutServer,
        }
    }
}

pub trait Server {
    type Request: DeserializeOwned + Serialize;
    type Response: DeserializeOwned + Serialize;

    fn execute(
        &self,
        request: Self::Request,
        js_args: Array
    ) -> impl futures_core::Future<Output = (Self::Response, Array, Array)>;
}

impl Server for WithoutServer {
    type Request = ();
    type Response = ();

    async fn execute(
        &self,
        _: Self::Request,
        _: Array
    ) -> (Self::Response, Array, Array) {
        unreachable!()
    }
}

pub trait Client {
    type Request: DeserializeOwned + Serialize;
    type Response: DeserializeOwned + Serialize;
}

pub type ClientRequestSender<C> = mpsc::UnboundedSender<(
    <C as Client>::Request,
    Array,
    Array,
    oneshot::Sender<(<C as Client>::Response, Array)>
)>;

impl Client for WithoutClient {
    type Request = ();
    type Response = ();
}

impl From<ClientRequestSender<WithoutClient>> for WithoutClient {
    fn from(_: ClientRequestSender<WithoutClient>) -> Self {
        Self
    }
}

#[derive(Serialize, Deserialize)]
enum Message<Request, Response>  {
    Request(u32, Request),
    Response(u32, Response),
}

pub struct WithoutClient;
pub struct WithoutServer;

pub struct Interface<C, S, I> {
    client: PhantomData<C>,
    server: S,
    interface: I,
}

impl<C, I> Interface<C, WithoutServer, I> {
    pub fn with_server<S: Server>(
        self,
        server: S
    ) -> Interface<C, S, I> {
        let Interface { interface, client, .. } = self;
        Interface { interface, client, server }
    }
}

impl<S, I> Interface<WithoutClient, S, I> {
    pub fn with_client<C: Client>(
        self,
    ) -> Interface<C, S, I> {
        let Interface { interface, server, .. } = self;
        Interface { interface, client: PhantomData::<C>, server }
    }
}

impl<C, S, I> Interface<C, S, I> where
    C: Client + From<ClientRequestSender<C>> + 'static,
    S: Server + 'static,
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

// TODO what is the best approach to cleaning up?
// custom drop impl, use futures' now_or_never?
// try finish remaining server/client requests?
// ignore new server requests?
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

#[allow(clippy::type_complexity)]
async fn run<C, S, I>(
    mut client_requests_rx: mpsc::UnboundedReceiver<(
        C::Request,
        Array,
        Array,
        oneshot::Sender<(C::Response, Array)>
    )>,
    server: S,
    interface: I
) where
    C: Client,
    S: Server,
    I: SendRecv,
    <S as Server>::Request: 'static,
    <C as Client>::Response: 'static {
    /* set up the event listener */
    let (client_responses_tx, mut client_responses_rx) = mpsc::unbounded();
    let (server_requests_tx, mut server_requests_rx) = mpsc::unbounded();
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
                server_requests_tx.unbounded_send((seq_id, request, array)).unwrap()
        }
    });
    interface.prepare();

    let mut last_seq_id: u32 = 0;   
    let mut client_tasks: HashMap<u32, oneshot::Sender<_>> = Default::default();
    let mut server_responses_rx: FuturesUnordered<_> = Default::default();

    loop {
        futures_util::select! {
            server_request = server_requests_rx.next() => {
                let (seq_id, request, post_args) = server_request.unwrap();
                let task = server.execute(request, post_args)
                    .map(move |(response, post_args, transfer_args)|
                        (response, post_args, transfer_args, seq_id));
                server_responses_rx.push(task);
            },
            server_response = server_responses_rx.next() => {
                if let Some((response, post_args, transfer_args, seq_id)) = server_response {
                    let response = Message::<C::Request, S::Response>::Response(seq_id, response);
                    let response = bincode::serialize(&response).unwrap();
                    let buffer = Uint8Array::from(&response[..]).buffer();
                    post_args.unshift(&buffer);
                    transfer_args.unshift(&buffer);
                    interface.post_message(&post_args, &transfer_args).unwrap();
                }
            },
            client_request = client_requests_rx.next() => {
                if let Some((request, post_args, transfer_args, callback)) = client_request {
                    let seq_id = last_seq_id;
                    last_seq_id = last_seq_id.wrapping_add(1);
                    client_tasks.insert(seq_id, callback);
                    let request = Message::<C::Request, S::Response>::Request(seq_id, request);
                    let request = bincode::serialize(&request).unwrap();
                    let buffer = js_sys::Uint8Array::from(&request[..]).buffer();
                    post_args.unshift(&buffer);
                    transfer_args.unshift(&buffer);
                    interface.post_message(&post_args, &transfer_args).unwrap();
                }
            },
            client_response = client_responses_rx.next() => {
                let (seq_id, response, js_args) = client_response.unwrap();
                let _ = client_tasks
                    .remove(&seq_id)
                    .expect("unknown response")
                    .send((response, js_args));
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