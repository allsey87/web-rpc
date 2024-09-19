use std::{cell::RefCell, marker::PhantomData, pin::Pin, rc::Rc, task::{Context, Poll}};

use futures_channel::mpsc;
use futures_core::{future::LocalBoxFuture, Future};
use futures_util::{FutureExt, StreamExt};
use gloo_events::EventListener;
use js_sys::{Uint8Array, ArrayBuffer};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use wasm_bindgen::JsCast;

pub use bincode;
pub use futures_channel;
pub use futures_core;
pub use futures_util;
pub use gloo_events;
pub use js_sys;
pub use pin_utils;
pub use serde;
pub use wasm_bindgen;
pub use web_rpc_macro::service;

pub mod client;
pub mod service;
pub mod interface;
pub mod port;

pub use interface::Interface;

#[derive(Serialize, Deserialize)]
pub enum Message<Request, Response> {
    Request(usize, Request),
    Abort(usize),
    Response(usize, Response),
}

pub struct Builder<C, S> {
    client: PhantomData<C>,
    service: S,
    interface: Interface,
}

impl Builder<(), ()> {
    pub fn new(interface: Interface) -> Self {
        Self {
            interface,
            client: PhantomData::<()>,
            service: (),
        }
    }
}

impl<C> Builder<C, ()> {
    pub fn with_service<S: service::Service>(
        self,
        implementation: impl Into<S>
    ) -> Builder<C, S> {
        let service = implementation.into();
        let Builder { interface, client, .. } = self;
        Builder { interface, client, service }
    }
}

impl<S> Builder<(), S> {
    pub fn with_client<C: client::Client>(
        self,
    ) -> Builder<C, S> {
        let Builder { interface, service, .. } = self;
        Builder { interface, client: PhantomData::<C>, service }
    }
}

pub struct Server {
    _listener: Rc<EventListener>,
    task: LocalBoxFuture<'static, ()>,
}

impl Future for Server {
    type Output = ();

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>
    ) -> Poll<Self::Output> {
        self.task.poll_unpin(cx)
    }
}

impl<C> Builder<C, ()> where
    C: client::Client + From<client::Configuration<C::Request, C::Response>> + 'static,
    <C as client::Client>::Response: DeserializeOwned,
    <C as client::Client>::Request: Serialize {

    pub fn build(self) -> C {
        let Builder { interface: Interface { port, listener, mut messages_rx }, ..} = self;
        let client_callback_map: Rc<RefCell<client::CallbackMap<C::Response>>> = Default::default();
        let client_callback_map_cloned = client_callback_map.clone();
        let dispatcher = async move {
            while let Some(array) = messages_rx.next().await {
                let message = Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap())
                    .to_vec();
                match bincode::deserialize::<Message<(), C::Response>>(&message).unwrap() {
                    Message::Response(seq_id, response) => {
                        if let Some(callback_tx) = client_callback_map_cloned.borrow_mut().remove(&seq_id) {
                            let _ = callback_tx.send((response, array));
                        }
                    },
                    _ => panic!("client received a server message"),
                }
            }
        }.boxed_local().shared();
        let port_cloned = port.clone();
        let abort_sender = move |seq_id: usize| {
            let abort = Message::<C::Request, ()>::Abort(seq_id);
            let abort = bincode::serialize(&abort).unwrap();
            let buffer = js_sys::Uint8Array::from(&abort[..]).buffer();
            let post_args = js_sys::Array::of1(&buffer);
            let transfer_args = js_sys::Array::of1(&buffer);
            port_cloned.post_message(&post_args, &transfer_args).unwrap();
        };
        let request_serializer = |seq_id: usize, request: C::Request| {
            let request = Message::<C::Request, ()>::Request(seq_id, request);
            bincode::serialize(&request).unwrap()
        };
        C::from((
            client_callback_map,
            port,
            Rc::new(listener),
            dispatcher,
            Rc::new(request_serializer),
            Rc::new(abort_sender)
        ))
    }
}

impl<S> Builder<(), S> where
    S: service::Service + 'static,
    <S as service::Service>::Request: DeserializeOwned,
    <S as service::Service>::Response: Serialize {

    pub fn build(self) -> Server {
        let Builder { service, interface: Interface { port, listener, mut messages_rx }, .. } = self;
        let (server_requests_tx, server_requests_rx) = mpsc::unbounded();
        let (abort_requests_tx, abort_requests_rx) = mpsc::unbounded();
        let dispatcher = async move {
            while let Some(array) = messages_rx.next().await {
                let message = Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap())
                    .to_vec();
                match bincode::deserialize::<Message<S::Request, ()>>(&message).unwrap() {
                    Message::Request(seq_id, request) =>
                        server_requests_tx.unbounded_send((seq_id, request, array)).unwrap(),
                    Message::Abort(seq_id) =>
                        abort_requests_tx.unbounded_send(seq_id).unwrap(),
                    _ => panic!("server received a client message"),
                }
            }
        }.boxed_local().shared();
        Server {
            _listener: Rc::new(listener),
            task: service::task::<S, ()>(
                service,
                port,
                dispatcher,
                server_requests_rx,
                abort_requests_rx
            ).boxed_local()
        }
    }
}

impl<C, S> Builder<C, S> where
    C: client::Client + From<client::Configuration<C::Request, C::Response>> + 'static,
    S: service::Service + 'static,
    <S as service::Service>::Request: DeserializeOwned,
    <S as service::Service>::Response: Serialize,
    <C as client::Client>::Request: Serialize,
    <C as client::Client>::Response: DeserializeOwned {
    pub fn build(self) -> (C, Server) {
        let Builder { service: server, interface: Interface { port, listener, mut messages_rx }, .. } = self;
        let client_callback_map: Rc<RefCell<client::CallbackMap<C::Response>>> = Default::default();
        let (server_requests_tx, server_requests_rx) = mpsc::unbounded();
        let (abort_requests_tx, abort_requests_rx) = mpsc::unbounded();
        let client_callback_map_cloned = client_callback_map.clone();
        let dispatcher = async move {
            while let Some(array) = messages_rx.next().await {
                let message = array.shift().dyn_into::<ArrayBuffer>().unwrap();
                let message = Uint8Array::new(&message).to_vec();
                match bincode::deserialize::<Message<S::Request, C::Response>>(&message).unwrap() {
                    Message::Response(seq_id, response) => {
                        if let Some(callback_tx) = client_callback_map_cloned.borrow_mut().remove(&seq_id) {
                            let _ = callback_tx.send((response, array));
                        }
                    },
                    Message::Request(seq_id, request) =>
                        server_requests_tx.unbounded_send((seq_id, request, array)).unwrap(),
                    Message::Abort(seq_id) =>
                        abort_requests_tx.unbounded_send(seq_id).unwrap(),
                }
            }
        }.boxed_local().shared();
        let port_cloned = port.clone();
        let abort_sender = move |seq_id: usize| {
            let abort = Message::<C::Request, S::Response>::Abort(seq_id);
            let abort = bincode::serialize(&abort).unwrap();
            let buffer = js_sys::Uint8Array::from(&abort[..]).buffer();
            let post_args = js_sys::Array::of1(&buffer);
            let transfer_args = js_sys::Array::of1(&buffer);
            port_cloned.post_message(&post_args, &transfer_args).unwrap();
        };
        let request_serializer = |seq_id: usize, request: C::Request| {
            let request = Message::<C::Request, S::Response>::Request(seq_id, request);
            bincode::serialize(&request).unwrap()
        };
        let listener = Rc::new(listener);
        let client = C::from((
            client_callback_map,
            port.clone(),
            listener.clone(),
            dispatcher.clone(),
            Rc::new(request_serializer),
            Rc::new(abort_sender),
        ));
        let server = Server {
            _listener: listener,
            task: service::task::<S, C::Request>(
                server,
                port,
                dispatcher,
                server_requests_rx,
                abort_requests_rx
            ).boxed_local()
        };
        (client, server)
    }
}
