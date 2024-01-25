use std::{marker::PhantomData, rc::Rc, sync::{atomic::AtomicUsize, Arc}};

use futures_channel::mpsc;
use futures_core::{future::LocalBoxFuture, Future};
use futures_util::FutureExt;
use gloo_events::EventListener;
use js_sys::{Uint8Array, Array, ArrayBuffer};
use serde::{Serialize, Deserialize};
use wasm_bindgen::JsCast;
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
pub mod interface;

#[derive(Debug)]
pub enum Error {
    EncodeDecode(bincode::ErrorKind),
    Aborted,
}
pub type Result<T> = std::result::Result<T, Error>;

impl<I: interface::Interface> Builder<client::None, server::None, I> {
    pub fn new(interface: I) -> Self {
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

pub struct Builder<C, S, I> {
    client: PhantomData<C>,
    server: S,
    interface: I,
}

impl<C, I> Builder<C, server::None, I> {
    pub fn with_server<S: server::Server>(
        self,
        server: S
    ) -> Builder<C, server::Some<S>, I> {
        let Builder { interface, client, .. } = self;
        Builder { interface, client, server: server::Some(server) }
    }
}

impl<S, I> Builder<client::None, S, I> {
    pub fn with_client<C: client::Client>(
        self,
    ) -> Builder<client::Some<C>, S, I> {
        let Builder { interface, server, .. } = self;
        Builder { interface, client: PhantomData::<client::Some<C>>, server }
    }
}

pub struct Server {
    listener: Rc<EventListener>,
    task: LocalBoxFuture<'static, ()>,
}

impl Future for Server {
    type Output = ();

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        self.task.poll_unpin(cx)
    }
}

#[derive(Clone)]
pub struct Client<C> {
    listener: Rc<EventListener>,
    last_seq_id: Arc<AtomicUsize>,
    client_impl: C
}

impl<C> std::ops::Deref for Client<C> {
    type Target = C;
    fn deref(&self) -> &Self::Target {
        &self.client_impl
    }
}

impl<C, I> Builder<client::Some<C>, server::None, I> where
    C: client::Client + From<client::RequestSender<C>> + 'static,
    I: interface::Interface + 'static,
    <C as client::Client>::Response: 'static {

    pub async fn build(self) -> Client<C> {
        let Builder { interface, ..} = self;
        let (client_requests_tx, client_requests_rx) = mpsc::unbounded();
        let (client_responses_tx, client_responses_rx) = mpsc::unbounded();
                
        interface.pre_attach().await;
        let listener = EventListener::new(interface.as_ref(), "message", move |event| {
            let array = event.unchecked_ref::<MessageEvent>()
                .data()
                .dyn_into::<Array>()
                .unwrap();
            let message = Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap())
                .to_vec();
            match bincode::deserialize::<Message<(), C::Response>>(&message).unwrap() {
                Message::Response(seq_id, response) =>
                    client_responses_tx.unbounded_send((seq_id, response, array)).unwrap(),
                _ => panic!("client received a server message"),
            }
        });
        interface.post_attach().await;
        
        // WORK IN PROGRESS: do not rely on task so that messages can be sync
        let temp_client_task = client::task::<C, I, ()>(
            interface.clone(),
            client_requests_rx,
            client_responses_rx
        );
        wasm_bindgen_futures::spawn_local(temp_client_task);
        // WORK IN PROGRESS: do not rely on task so that messages can be sync
        
        Client {
            listener: Rc::new(listener),
            last_seq_id: Default::default(),
            client_impl: C::from(client_requests_tx)
        }
    }
}

impl<S, I> Builder<client::None, server::Some<S>, I> where 
    S: server::Server + 'static,
    I: interface::Interface + 'static,
    <S as server::Server>::Request: 'static {

    pub async fn build(self) -> Server {
        let Builder { server: server::Some(server_impl), interface, .. } = self;
        
        let (server_requests_tx, server_requests_rx) = mpsc::unbounded();
        let (abort_requests_tx, abort_requests_rx) = mpsc::unbounded();
        
        interface.pre_attach().await;
        let listener = EventListener::new(interface.as_ref(), "message", move |event| {
            let array = event.unchecked_ref::<MessageEvent>()
                .data()
                .dyn_into::<Array>()
                .unwrap();
            let message = Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap())
                .to_vec();
            match bincode::deserialize::<Message<S::Request, ()>>(&message).unwrap() {
                // rx messages for the server
                Message::Request(seq_id, request) =>
                    server_requests_tx.unbounded_send((seq_id, request, array)).unwrap(),
                Message::Abort(seq_id) =>
                    abort_requests_tx.unbounded_send(seq_id).unwrap(),
                _ => panic!("server received a client message"),
            }
        });
        interface.post_attach().await;
        Server {
            listener: Rc::new(listener),
            task: server::task::<S, I, ()>(
                server_impl,
                interface,
                server_requests_rx,
                abort_requests_rx
            ).boxed_local()
        }
    }
}

impl<C, S, I> Builder<client::Some<C>, server::Some<S>, I> where
    C: client::Client + From<client::RequestSender<C>> + 'static,
    S: server::Server + 'static,
    I: interface::Interface + 'static,
    <S as server::Server>::Request: 'static,
    <C as client::Client>::Response: 'static {
    
    pub async fn build(self) -> (Client<C>, Server) {
        let Builder { server: server::Some(server_impl), interface, .. } = self;
        let (client_requests_tx, client_requests_rx) = mpsc::unbounded();
        let (client_responses_tx, client_responses_rx) = mpsc::unbounded();
        
        let (server_requests_tx, server_requests_rx) = mpsc::unbounded();
        let (abort_requests_tx, abort_requests_rx) = mpsc::unbounded();
        
        interface.pre_attach().await;
        let listener = EventListener::new(interface.as_ref(), "message", move |event| {
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
        interface.post_attach().await;
        let listener = Rc::new(listener);
        
        // WORK IN PROGRESS: do not rely on task so that messages can be sync
        let temp_client_task = client::task::<C, I, S::Response>(
            interface.clone(),
            client_requests_rx,
            client_responses_rx
        );
        wasm_bindgen_futures::spawn_local(temp_client_task);
        // WORK IN PROGRESS: do not rely on task so that messages can be sync

        let client = Client {
            listener: listener.clone(),
            last_seq_id: Default::default(),
            client_impl: C::from(client_requests_tx)
        };

        let server = Server {
            listener,
            task: server::task::<S, I, C::Request>(
                server_impl,
                interface,
                server_requests_rx,
                abort_requests_rx
            ).boxed_local()
        };

        (client, server)
    }
}
