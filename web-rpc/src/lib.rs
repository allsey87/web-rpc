//! Bidirectional RPC for browsing contexts, web workers, and message channels.
//!
//! This crate allows you to define a service as a trait and annotate it with
//! [`#[web_rpc::service]`](macro@service). The macro then produces a `*Client`, a `*Service`,
//! a forwarding trait that you can implement on the server side, and a compile-time
//! [description](describe::Service) of the trait from which
//! [`js::endpoint!`](macro@js::endpoint) can render a typed Javascript endpoint.
//!
//! Routing is explicit. A value wrapped in [`Post`](wrap::Post) or [`Transfer`](wrap::Transfer)
//! crosses the channel as a Javascript value through `postMessage`; anything else is encoded
//! with [postcard](https://docs.rs/postcard) and must implement
//! [`postcard_schema::Schema`]. There is special support for `Option<T>` and `Result<T, E>`
//! so that Javascript values can be embedded within them, and this behaviour is recursive.
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
//! or any [`MessagePort`](https://docs.rs/web-sys/latest/web_sys/struct.MessagePort.html).
//! Each call to [`Interface::new`] is async because temporary listeners need to detect when
//! both ends are ready.
//! ```rust,no_run
//! # #[web_rpc::service]
//! # pub trait Calculator { fn add(&self, l: u32, r: u32) -> u32; }
//! # struct Calc;
//! # impl Calculator for Calc { fn add(&self, l: u32, r: u32) -> u32 { l + r } }
//! # async fn run() {
//! let channel = web_sys::MessageChannel::new().unwrap();
//! channel.port1().start();
//! channel.port2().start();
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
//! # Transports are borrowed, never owned
//! web-rpc uses the transport it is handed and never manages its lifecycle. Dropping a
//! [`Port`](port::Port), an [`Interface`] or a client does not terminate a
//! [`Worker`](web_sys::Worker): whoever created the worker terminates it. Likewise a
//! [`MessagePort`](web_sys::MessagePort) is **not** started for you, on either the Rust or the
//! Javascript side. Call [`start`](web_sys::MessagePort::start) on it before handing it over,
//! as in the example above; an unstarted port delivers nothing to the listener that
//! [`Interface::new`] installs, so the symptom is a handshake that spins forever rather than an
//! error.
//!
//! # Routing
//! ```rust
//! use web_rpc::wrap::{Post, Transfer};
//!
//! #[web_rpc::service]
//! pub trait Routing {
//!     // Plain types implementing `Serialize` and `Schema` go through postcard.
//!     fn add(&self, l: u32, r: u32) -> u32;
//!     // `Post<T>` crosses as a Javascript value, copied by structured clone.
//!     fn echo(&self, s: Post<js_sys::JsString>) -> Post<js_sys::JsString>;
//!     // `Transfer<T>` crosses as a Javascript value and is moved, not copied.
//!     fn upload(&self, buffer: Transfer<js_sys::ArrayBuffer>) -> u32;
//!     // `Option`/`Result` recurse: each variant routes independently.
//!     fn lookup(&self, k: u32) -> Result<Option<Post<js_sys::JsString>>, String>;
//!     // `&str` / `&[u8]` deserialize zero-copy on the server.
//!     fn count(&self, data: &[u8]) -> u32;
//! }
//! ```
//! A bare Javascript type in a signature is a compile error, because it implements neither
//! [`serde::Serialize`] nor [`postcard_schema::Schema`]. Note that a typed array is not a
//! transferable object: send `Transfer<ArrayBuffer>` and rebuild the view on the other side.
//!
//! Every type in a signature must implement [`postcard_schema::Schema`], which for your own
//! payload types means `#[derive(Schema)]` alongside the serde derives. The trait description,
//! and therefore the generated Javascript, is built from it. postcard-schema implements `Schema`
//! for neither `usize` nor `isize`, since serde widens both to 64 bits, so use a fixed-width
//! integer in a signature; and a foreign type with no upstream `Schema` impl needs a local
//! mirror type.
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
//! `Result<Post<JsT>, E>`.
//!
//! # Conditional methods
//! Methods can be gated with `#[cfg(...)]` or `#[cfg_attr(...)]`. The macro propagates these
//! attributes to every generated artifact for that method, so rustc strips them in lockstep.
//! ```rust
//! #[web_rpc::service]
//! pub trait Conditional {
//!     fn always_on(&self, x: u32) -> u32;
//!     #[cfg(feature = "admin")]
//!     fn extra(&self, s: &str) -> String;
//! }
//! ```
//! Postcard encodes enum variants by their positional discriminant, so the set of methods
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
//!
//! # Javascript endpoints
//! [`js::endpoint!`](macro@js::endpoint) renders a Javascript class and a `.d.ts` for the other
//! end of a connection, from the same traits, into two custom sections of the wasm binary. See
//! the [`js`] module.

use std::{
    cell::RefCell,
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use futures_channel::mpsc;
use futures_core::{future::LocalBoxFuture, Future};
use futures_util::{future::Shared, FutureExt, StreamExt};
use gloo_events::EventListener;
use js_sys::{Array, ArrayBuffer, Uint8Array};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use wasm_bindgen::JsCast;

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
pub use postcard;
#[doc(hidden)]
pub use postcard_schema;
#[doc(hidden)]
pub use serde;
#[doc(hidden)]
pub use wasm_bindgen;
#[doc(hidden)]
pub use web_sys;

pub use web_rpc_macro::service;

pub mod client;
#[doc(hidden)]
pub mod codec;
pub mod describe;
pub mod interface;
pub mod js;
pub mod port;
#[doc(hidden)]
pub mod service;
pub mod wrap;

pub use interface::Interface;
use port::Port;

/// The first element of every message. The sequence number is allocated by whoever sends
/// the request, and identifies a message only within the direction it travels.
#[doc(hidden)]
#[derive(Serialize, Deserialize)]
pub enum MessageHeader {
    Request(u32),
    Abort(u32),
    Response(u32),
    StreamItem(u32),
    StreamEnd(u32),
}

/// The future that turns inbound messages into responses, stream items and requests. It is
/// shared by every client and server on one interface and driven by whichever of them is
/// polled, and it completes only when the listener that feeds it is dropped.
#[doc(hidden)]
pub type Dispatcher = Shared<LocalBoxFuture<'static, ()>>;

fn to_buffer(bytes: &[u8]) -> ArrayBuffer {
    Uint8Array::from(bytes).buffer()
}

/// Take the `ArrayBuffer` at the front of a message and copy it out.
#[doc(hidden)]
pub fn take_bytes(message: &Array) -> Vec<u8> {
    let buffer = message
        .shift()
        .dyn_into::<ArrayBuffer>()
        .expect("web_rpc: a message must start with an ArrayBuffer");
    Uint8Array::new(&buffer).to_vec()
}

/// Post a message that is only a header.
#[doc(hidden)]
pub fn post_header(port: &Port, header: MessageHeader) {
    let header = to_buffer(&postcard::to_allocvec(&header).unwrap());
    let message = Array::of1(&header);
    port.post_message(&message, &message).unwrap();
}

/// Post `[header, payload, ...post_args]`, transferring the buffers and `transfer_args`.
#[doc(hidden)]
pub fn post_message(
    port: &Port,
    header: MessageHeader,
    payload: &impl Serialize,
    post_args: &Array,
    transfer_args: &Array,
) {
    let header = to_buffer(&postcard::to_allocvec(&header).unwrap());
    let payload = to_buffer(&postcard::to_allocvec(payload).unwrap());
    post_args.unshift(&payload);
    post_args.unshift(&header);
    transfer_args.unshift(&payload);
    transfer_args.unshift(&header);
    port.post_message(post_args, transfer_args).unwrap();
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
            client: PhantomData,
            service: (),
        }
    }
}

impl<C> Builder<C, ()> {
    /// Configure the RPC interface with a service that implements methods
    /// that can be called from the other side of the channel. To use this method,
    /// you need to specify the type `S` which is the service type generated by the
    /// attribute macro [`macro@service`]. The implementation parameter is then an
    /// instance of something that implements the trait to which you applied the
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
        Builder {
            interface: self.interface,
            client: self.client,
            service: implementation.into(),
        }
    }
}

impl<S> Builder<(), S> {
    /// Configure the RPC interface with a client that allows you to execute RPCs on the
    /// server. The builder instantiates the client for you, you just
    /// need to provide the type which is generated via the [`macro@service`] attribute
    /// macro. For example, if you had a trait `Calculator` to which you applied the
    /// [`macro@service`] attribute macro, the macro would have generated a `CalculatorClient`
    /// struct which you can use as the `C` in this function.
    pub fn with_client<C: client::Client>(self) -> Builder<C, S> {
        Builder {
            interface: self.interface,
            client: PhantomData,
            service: self.service,
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

/// The client half of an interface with no client: it receives nothing and is never handed
/// out.
struct NoClient;

impl client::Client for NoClient {
    type Response = ();
}

impl From<client::State<()>> for NoClient {
    fn from(_: client::State<()>) -> Self {
        NoClient
    }
}

/// The service half of an interface with no service: its server is dropped unpolled.
struct NoService;

impl service::Service for NoService {
    type Response = ();

    async fn execute(
        &self,
        _: u32,
        _: futures_channel::oneshot::Receiver<()>,
        _: Vec<u8>,
        _: Array,
        _: mpsc::UnboundedSender<service::StreamMessage<()>>,
    ) -> (u32, service::ExecuteResult<()>) {
        unreachable!("web_rpc: a request reached an interface with no service")
    }
}

/// Build both halves of an interface. Whichever half the caller did not ask for is built
/// from its `No*` stand-in and dropped.
fn assemble<C, S>(interface: Interface, service: S) -> (C, Server)
where
    C: client::Client + From<client::State<C::Response>> + 'static,
    C::Response: DeserializeOwned,
    S: service::Service + 'static,
    S::Response: Serialize,
{
    let Interface {
        port,
        listener,
        mut messages_rx,
    } = interface;
    let callbacks: Rc<RefCell<client::CallbackMap<C::Response>>> = Default::default();
    let stream_callbacks: Rc<RefCell<client::StreamCallbackMap<C::Response>>> = Default::default();
    let (requests_tx, requests_rx) = mpsc::unbounded();
    let (aborts_tx, aborts_rx) = mpsc::unbounded();
    let dispatcher: Dispatcher = {
        let callbacks = callbacks.clone();
        let stream_callbacks = stream_callbacks.clone();
        async move {
            while let Some(message) = messages_rx.next().await {
                let header: MessageHeader = postcard::from_bytes(&take_bytes(&message)).unwrap();
                match header {
                    MessageHeader::Request(sequence) => {
                        let payload = take_bytes(&message);
                        requests_tx
                            .unbounded_send((sequence, payload, message))
                            .expect("web_rpc: a request arrived but the server has been dropped");
                    }
                    MessageHeader::Abort(sequence) => {
                        let _ = aborts_tx.unbounded_send(sequence);
                    }
                    MessageHeader::Response(sequence) => {
                        let response = postcard::from_bytes(&take_bytes(&message)).unwrap();
                        if let Some(callback) = callbacks.borrow_mut().remove(&sequence) {
                            let _ = callback.send((response, message));
                        }
                    }
                    MessageHeader::StreamItem(sequence) => {
                        let item = postcard::from_bytes(&take_bytes(&message)).unwrap();
                        if let Some(items) = stream_callbacks.borrow().get(&sequence) {
                            let _ = items.unbounded_send((item, message));
                        }
                    }
                    MessageHeader::StreamEnd(sequence) => {
                        stream_callbacks.borrow_mut().remove(&sequence);
                    }
                }
            }
        }
        .boxed_local()
        .shared()
    };
    let listener = Rc::new(listener);
    let client = C::from(client::State {
        callbacks,
        stream_callbacks,
        port: port.clone(),
        listener: listener.clone(),
        dispatcher: dispatcher.clone(),
        sequence: Default::default(),
    });
    let server = Server {
        _listener: listener,
        task: service::task::<S>(service, port, dispatcher, requests_rx, aborts_rx).boxed_local(),
    };
    (client, server)
}

impl<C> Builder<C, ()>
where
    C: client::Client + From<client::State<C::Response>> + 'static,
    C::Response: DeserializeOwned,
{
    /// Build function for client-only RPC interfaces.
    pub fn build(self) -> C {
        assemble::<C, NoService>(self.interface, NoService).0
    }
}

impl<S> Builder<(), S>
where
    S: service::Service + 'static,
    S::Response: Serialize,
{
    /// Build function for server-only RPC interfaces.
    pub fn build(self) -> Server {
        assemble::<NoClient, S>(self.interface, self.service).1
    }
}

impl<C, S> Builder<C, S>
where
    C: client::Client + From<client::State<C::Response>> + 'static,
    C::Response: DeserializeOwned,
    S: service::Service + 'static,
    S::Response: Serialize,
{
    /// Build function for client-server RPC interfaces.
    pub fn build(self) -> (C, Server) {
        assemble::<C, S>(self.interface, self.service)
    }
}
