//! Bidirectional RPC for browsing contexts, web workers, and message channels.
//!
//! This crate allows you to define a service as a trait and annotate it with
//! [`#[web_rpc::service]`](macro@service). The macro then produces a `*Client`, a `*Service`,
//! and a forwarding trait that you can implement on the server side.
//!
//! Routing is inferred from each type: anything implementing
//! [`AsRef<JsValue>`](https://docs.rs/wasm-bindgen/latest/wasm_bindgen/struct.JsValue.html) is
//! posted through `postMessage` directly and everything that is serializable is first encoded
//! via bincode. There is special support for `Option<T>` and `Result<T, E>` to allow Javascript
//! types to be embedded within these types. This behaviour is recursive.
//!
//! # Quickstart
//! ```rust
//! #[web_rpc::service]
//! pub trait Calculator {
//!     fn add(&self, left: u32, right: u32) -> u32;
//! }
//! struct Calc;
//! impl Calculator for Calc {
//!     fn add(&self, left: u32, right: u32) -> u32 { left + right }
//! }
//! ```
//! Wire up over a `MessageChannel`, [`Worker`](https://docs.rs/web-sys/latest/web_sys/struct.Worker.html),
//! or any [`MessagePort`](https://docs.rs/web-sys/latest/web_sys/struct.MessagePort.html). Each
//! Each call to [`Interface::new`] is async because temporary listeners need to detect when
//! both ends are ready.
//! ```rust,no_run
//! # #[web_rpc::service]
//! # pub trait Calculator { fn add(&self, l: u32, r: u32) -> u32; }
//! # struct Calc;
//! # impl Calculator for Calc { fn add(&self, l: u32, r: u32) -> u32 { l + r } }
//! # async fn run() {
//! let channel = web_sys::MessageChannel::new().unwrap();
//! let (server_iface, client_iface) = futures_util::future::join(
//!     web_rpc::Interface::new(channel.port1()),
//!     web_rpc::Interface::new(channel.port2()),
//! ).await;
//!
//! let server = web_rpc::Builder::new(server_iface)
//!     .with_service::<CalculatorService<_>>(Calc)
//!     .build();
//! wasm_bindgen_futures::spawn_local(server);
//!
//! let client = web_rpc::Builder::new(client_iface)
//!     .with_client::<CalculatorClient>()
//!     .build();
//! assert_eq!(client.add(41, 1).await, 42);
//! # }
//! ```
//!
//! # Routing
//! ```rust
//! #[web_rpc::service]
//! pub trait Routing {
//!     // Plain types that implement Serialize go through bincode.
//!     fn add(&self, l: u32, r: u32) -> u32;
//!     // Anything `AsRef<JsValue>` is posted through the JS array.
//!     fn echo(&self, s: js_sys::JsString) -> js_sys::JsString;
//!     // `Option`/`Result` recurse: Ok(Some(_)) is posted, Ok(None) is one byte,
//!     // Err carries a bincoded `String`.
//!     fn lookup(&self, k: u32) -> Result<Option<js_sys::JsString>, String>;
//!     // `&str` / `&[u8]` deserialize zero-copy on the server.
//!     fn count(&self, data: &[u8]) -> usize;
//!     // References to JS types are accepted too and are decoded via JsCast::dyn_ref.
//!     fn len(&self, s: &js_sys::JsString) -> u32;
//! }
//! ```
//!
//! # Async, notifications, streaming
//! ```rust
//! use futures_core::Stream;
//!
//! #[web_rpc::service]
//! pub trait Misc {
//!     // `async` here makes the server impl async; the client side is also async because we return a u32.
//!     async fn slow(&self, ms: u32) -> u32;
//!     // No return type means the method is a notification.
//!     fn fire(&self, msg: String);
//!     // `impl Stream<Item = T>` makes the method a streaming RPC.
//!     fn items(&self, n: u32) -> impl Stream<Item = u32>;
//! }
//! ```
//! On the client side, RPC methods that have a return type are async and yield a
//! [`client::RequestFuture<T>`] which you await for the response. Methods without a return type
//! are sync and act as fire-and-forget notifications. This is independent of whether the trait
//! method itself is marked `async`, which only affects the server implementation. Dropping the
//! `RequestFuture` cancels the request, so notifications cannot be cancelled.
//!
//! Streaming methods return a [`client::StreamReceiver<T>`] that yields each item the server
//! produces. Dropping the receiver aborts the stream on the server, while
//! [`close`](client::StreamReceiver::close) lets buffered items finish arriving instead.
//! Streaming methods can also be `async` and the items they yield can be wrapper types like
//! `Result<JsT, E>`.
//!
//! # Transfer
//! Anything that should be transferred to the other side rather than copied with the structured
//! clone algorithm can be specified inside a `#[transfer(...)]` attribute as a comma-separated
//! list. The simplest case is to list the parameter that holds the transferable value, but if
//! that value is wrapped or derived from a parameter, you can use a parameter-name expression
//! (`name => expr`, evaluated with `name` in scope), a closure with a refutable pattern
//! (`name => |pat| body`), or a match-block (`name => match { arm, ... }`). The same forms also
//! work for the return value via `return`.
//! ```rust
//! # use wasm_bindgen::JsCast;
//! #[web_rpc::service]
//! pub trait Transfer {
//!     // Bare param + derived expression + return closure.
//!     #[transfer(
//!         canvas,
//!         data => data.buffer(),
//!         return => |Ok(buf)| buf.buffer(),
//!     )]
//!     fn render(
//!         &self,
//!         canvas: web_sys::OffscreenCanvas,
//!         data: js_sys::Uint8Array,
//!     ) -> Result<js_sys::Uint8Array, String>;
//!
//!     // Match-block: useful when several variants need transferring.
//!     #[transfer(return => match { Some(buf) => buf.buffer(), })]
//!     fn maybe(&self) -> Option<js_sys::Uint8Array>;
//! }
//! ```
//!
//! # Conditional methods
//! Methods can be gated with `#[cfg(...)]` or `#[cfg_attr(...)]`. The macro propagates these
//! attributes to every generated artifact for that method, so rustc strips them in lockstep.
//! ```rust
//! #[web_rpc::service]
//! pub trait Conditional {
//!     fn always_on(&self, x: u32) -> u32;
//!     #[cfg(feature = "extra")]
//!     fn extra(&self, s: &str) -> String;
//! }
//! ```
//! Bincode encodes enum variants by their positional discriminant, so the set of methods
//! that survive cfg evaluation must match on both ends of a channel. If one side has a gated
//! method enabled and the other does not, the wire format will silently desync.
//!
//! # Bi-directional
//! Both sides of a channel can be set up to act as both client and server at the same time. To
//! do this, stack [`with_service`](Builder::with_service) and
//! [`with_client`](Builder::with_client) on the same [`Builder`] before calling `build()`, which
//! then returns a `(C, Server)` tuple instead of one or the other.
//! ```rust,no_run
//! # #[web_rpc::service]
//! # pub trait Calculator { fn add(&self, l: u32, r: u32) -> u32; }
//! # struct Calc;
//! # impl Calculator for Calc { fn add(&self, l: u32, r: u32) -> u32 { l + r } }
//! # async fn run() {
//! # let channel = web_sys::MessageChannel::new().unwrap();
//! # let (iface, _) = futures_util::future::join(
//! #     web_rpc::Interface::new(channel.port1()),
//! #     web_rpc::Interface::new(channel.port2()),
//! # ).await;
//! let (client, server) = web_rpc::Builder::new(iface)
//!     .with_service::<CalculatorService<_>>(Calc)
//!     .with_client::<CalculatorClient>()
//!     .build();
//! # }
//! ```

use std::{
    cell::RefCell,
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use futures_channel::mpsc;
use futures_core::{future::LocalBoxFuture, Future};
use futures_util::{FutureExt, StreamExt};
use gloo_events::EventListener;
use js_sys::{ArrayBuffer, Uint8Array};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use wasm_bindgen::JsCast;

#[doc(hidden)]
pub use bincode;
#[doc(hidden)]
pub use futures_channel;
#[doc(hidden)]
pub use futures_core;
#[doc(hidden)]
pub use futures_util;
#[doc(hidden)]
pub use gloo_events;
#[doc(hidden)]
pub use js_sys;
#[doc(hidden)]
pub use pin_utils;
#[doc(hidden)]
pub use serde;
#[doc(hidden)]
pub use wasm_bindgen;

pub use web_rpc_macro::service;

pub mod client;
#[doc(hidden)]
pub mod codec;
pub mod interface;
pub mod port;
#[doc(hidden)]
pub mod service;

pub use interface::Interface;

#[doc(hidden)]
#[derive(Serialize, Deserialize)]
pub enum MessageHeader {
    Request(usize),
    Abort(usize),
    Response(usize),
    StreamItem(usize),
    StreamEnd(usize),
}

/// This struct allows one to configure the RPC interface prior to creating it.
/// To get an instance of this struct, call [`Builder<C, S>::new`] with
/// an [`Interface`].
pub struct Builder<C, S> {
    client: PhantomData<C>,
    service: S,
    interface: Interface,
}

impl Builder<(), ()> {
    /// Create a new builder from an [`Interface`]
    pub fn new(interface: Interface) -> Self {
        Self {
            interface,
            client: PhantomData::<()>,
            service: (),
        }
    }
}

impl<C> Builder<C, ()> {
    /// Configure the RPC interface with a service that implements methods
    /// that can be called from the other side of the channel. To use this method,
    /// you need to specify the type `S` which is the service type generated by the
    /// attribute macro [`macro@service`]. The implementation parameter is then an
    /// instance of something that implements the trait to which to applied the
    /// [`macro@service`] macro. For example, if you have a trait `Calculator` to
    /// which you have applied [`macro@service`], you would use this method as follows:
    /// ```rust,no_run
    /// # #[web_rpc::service]
    /// # pub trait Calculator {
    /// #     fn add(&self, left: u32, right: u32) -> u32;
    /// # }
    /// # struct CalculatorServiceImpl;
    /// # impl Calculator for CalculatorServiceImpl {
    /// #     fn add(&self, left: u32, right: u32) -> u32 { left + right }
    /// # }
    /// # fn example(some_interface: web_rpc::Interface) {
    /// let server = web_rpc::Builder::new(some_interface)
    ///     .with_service::<CalculatorService<_>>(CalculatorServiceImpl)
    ///     .build();
    /// # }
    /// ```
    pub fn with_service<S: service::Service>(self, implementation: impl Into<S>) -> Builder<C, S> {
        let service = implementation.into();
        let Builder {
            interface, client, ..
        } = self;
        Builder {
            interface,
            client,
            service,
        }
    }
}

impl<S> Builder<(), S> {
    /// Configure the RPC interface with a client that allows you to execute RPCs on the
    /// server. The builder will automatically instansiate the client for you, you just
    /// need to provide the type which is generated via the [`macro@service`] attribute
    /// macro. For example, if you had a trait `Calculator` to which you applied the
    /// [`macro@service`] attribute macro, the macro would have generated a `CalculatorClient`
    /// struct which you can use as the `C` in this function.
    pub fn with_client<C: client::Client>(self) -> Builder<C, S> {
        let Builder {
            interface, service, ..
        } = self;
        Builder {
            interface,
            client: PhantomData::<C>,
            service,
        }
    }
}

/// `Server` is the server that is returned from the [`Builder::build`] method given
/// you configured the RPC interface with a service. Note that `Server` implements future and needs
/// to be polled in order to execute and respond to inbound RPC requests.
#[must_use = "Server must be polled in order for RPC requests to be executed"]
pub struct Server {
    _listener: Rc<EventListener>,
    task: LocalBoxFuture<'static, ()>,
}

impl Future for Server {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.task.poll_unpin(cx)
    }
}

impl<C> Builder<C, ()>
where
    C: client::Client + From<client::Configuration<C::Response>> + 'static,
    <C as client::Client>::Response: DeserializeOwned,
{
    /// Build function for client-only RPC interfaces.
    pub fn build(self) -> C {
        let Builder {
            interface:
                Interface {
                    port,
                    listener,
                    mut messages_rx,
                },
            ..
        } = self;
        let client_callback_map: Rc<RefCell<client::CallbackMap<C::Response>>> = Default::default();
        let client_callback_map_cloned = client_callback_map.clone();
        let stream_callback_map: Rc<RefCell<client::StreamCallbackMap<C::Response>>> =
            Default::default();
        let stream_callback_map_cloned = stream_callback_map.clone();
        let dispatcher = async move {
            while let Some(array) = messages_rx.next().await {
                let header_bytes =
                    Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap()).to_vec();
                let header: MessageHeader = bincode::deserialize(&header_bytes).unwrap();
                match header {
                    MessageHeader::Response(seq_id) => {
                        let payload_bytes =
                            Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap())
                                .to_vec();
                        let response: C::Response = bincode::deserialize(&payload_bytes).unwrap();
                        if let Some(callback_tx) =
                            client_callback_map_cloned.borrow_mut().remove(&seq_id)
                        {
                            let _ = callback_tx.send((response, array));
                        }
                    }
                    MessageHeader::StreamItem(seq_id) => {
                        let payload_bytes =
                            Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap())
                                .to_vec();
                        let response: C::Response = bincode::deserialize(&payload_bytes).unwrap();
                        if let Some(tx) = stream_callback_map_cloned.borrow().get(&seq_id) {
                            let _ = tx.unbounded_send((response, array));
                        }
                    }
                    MessageHeader::StreamEnd(seq_id) => {
                        stream_callback_map_cloned.borrow_mut().remove(&seq_id);
                    }
                    _ => panic!("client received a server message"),
                }
            }
        }
        .boxed_local()
        .shared();
        let port_cloned = port.clone();
        let abort_sender = move |seq_id: usize| {
            let header = MessageHeader::Abort(seq_id);
            let header_bytes = bincode::serialize(&header).unwrap();
            let buffer = js_sys::Uint8Array::from(&header_bytes[..]).buffer();
            let post_args = js_sys::Array::of1(&buffer);
            let transfer_args = js_sys::Array::of1(&buffer);
            port_cloned
                .post_message(&post_args, &transfer_args)
                .unwrap();
        };
        C::from((
            client_callback_map,
            stream_callback_map,
            port,
            Rc::new(listener),
            dispatcher,
            Rc::new(abort_sender),
        ))
    }
}

impl<S> Builder<(), S>
where
    S: service::Service + 'static,
    <S as service::Service>::Response: Serialize,
{
    /// Build function for server-only RPC interfaces.
    pub fn build(self) -> Server {
        let Builder {
            service,
            interface:
                Interface {
                    port,
                    listener,
                    mut messages_rx,
                },
            ..
        } = self;
        let (server_requests_tx, server_requests_rx) = mpsc::unbounded();
        let (abort_requests_tx, abort_requests_rx) = mpsc::unbounded();
        let dispatcher = async move {
            while let Some(array) = messages_rx.next().await {
                let header_bytes =
                    Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap()).to_vec();
                let header: MessageHeader = bincode::deserialize(&header_bytes).unwrap();
                match header {
                    MessageHeader::Request(seq_id) => {
                        let payload =
                            Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap())
                                .to_vec();
                        server_requests_tx
                            .unbounded_send((seq_id, payload, array))
                            .unwrap();
                    }
                    MessageHeader::Abort(seq_id) => {
                        abort_requests_tx.unbounded_send(seq_id).unwrap();
                    }
                    _ => panic!("server received a client message"),
                }
            }
        }
        .boxed_local()
        .shared();
        Server {
            _listener: Rc::new(listener),
            task: service::task::<S>(
                service,
                port,
                dispatcher,
                server_requests_rx,
                abort_requests_rx,
            )
            .boxed_local(),
        }
    }
}

impl<C, S> Builder<C, S>
where
    C: client::Client + From<client::Configuration<C::Response>> + 'static,
    S: service::Service + 'static,
    <S as service::Service>::Response: Serialize,
    <C as client::Client>::Response: DeserializeOwned,
{
    /// Build function for client-server RPC interfaces.
    pub fn build(self) -> (C, Server) {
        let Builder {
            service: server,
            interface:
                Interface {
                    port,
                    listener,
                    mut messages_rx,
                },
            ..
        } = self;
        let client_callback_map: Rc<RefCell<client::CallbackMap<C::Response>>> = Default::default();
        let stream_callback_map: Rc<RefCell<client::StreamCallbackMap<C::Response>>> =
            Default::default();
        let (server_requests_tx, server_requests_rx) = mpsc::unbounded();
        let (abort_requests_tx, abort_requests_rx) = mpsc::unbounded();
        let client_callback_map_cloned = client_callback_map.clone();
        let stream_callback_map_cloned = stream_callback_map.clone();
        let dispatcher = async move {
            while let Some(array) = messages_rx.next().await {
                let header_bytes =
                    Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap()).to_vec();
                let header: MessageHeader = bincode::deserialize(&header_bytes).unwrap();
                match header {
                    MessageHeader::Response(seq_id) => {
                        let payload_bytes =
                            Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap())
                                .to_vec();
                        let response: C::Response = bincode::deserialize(&payload_bytes).unwrap();
                        if let Some(callback_tx) =
                            client_callback_map_cloned.borrow_mut().remove(&seq_id)
                        {
                            let _ = callback_tx.send((response, array));
                        }
                    }
                    MessageHeader::StreamItem(seq_id) => {
                        let payload_bytes =
                            Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap())
                                .to_vec();
                        let response: C::Response = bincode::deserialize(&payload_bytes).unwrap();
                        if let Some(tx) = stream_callback_map_cloned.borrow().get(&seq_id) {
                            let _ = tx.unbounded_send((response, array));
                        }
                    }
                    MessageHeader::StreamEnd(seq_id) => {
                        stream_callback_map_cloned.borrow_mut().remove(&seq_id);
                    }
                    MessageHeader::Request(seq_id) => {
                        let payload =
                            Uint8Array::new(&array.shift().dyn_into::<ArrayBuffer>().unwrap())
                                .to_vec();
                        server_requests_tx
                            .unbounded_send((seq_id, payload, array))
                            .unwrap();
                    }
                    MessageHeader::Abort(seq_id) => {
                        abort_requests_tx.unbounded_send(seq_id).unwrap();
                    }
                }
            }
        }
        .boxed_local()
        .shared();
        let port_cloned = port.clone();
        let abort_sender = move |seq_id: usize| {
            let header = MessageHeader::Abort(seq_id);
            let header_bytes = bincode::serialize(&header).unwrap();
            let buffer = js_sys::Uint8Array::from(&header_bytes[..]).buffer();
            let post_args = js_sys::Array::of1(&buffer);
            let transfer_args = js_sys::Array::of1(&buffer);
            port_cloned
                .post_message(&post_args, &transfer_args)
                .unwrap();
        };
        let listener = Rc::new(listener);
        let client = C::from((
            client_callback_map,
            stream_callback_map,
            port.clone(),
            listener.clone(),
            dispatcher.clone(),
            Rc::new(abort_sender),
        ));
        let server = Server {
            _listener: listener,
            task: service::task::<S>(
                server,
                port,
                dispatcher,
                server_requests_rx,
                abort_requests_rx,
            )
            .boxed_local(),
        };
        (client, server)
    }
}
