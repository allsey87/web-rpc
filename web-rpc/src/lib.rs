//! The `web-rpc` create is a library for performing RPCs (remote proceedure calls) between
//! browsing contexts, web workers, and channels. It allows you to define an RPC using a trait
//! similar to Google's [tarpc](https://github.com/google/tarpc) and will transparently
//! handle the serialization and deserialization of the arguments. Moreover, it can post
//! anything that implements [`AsRef<JsValue>`](https://docs.rs/wasm-bindgen/latest/wasm_bindgen/struct.JsValue.html) and also supports transferring ownership.
//! ## Quick start
//! To get started define a trait for your RPC service as follows. Annnotate this trait with the
//! `service` procedural macro that is exported by this crate:
//! ```rust
//! #[web_rpc::service]
//! pub trait Calculator {
//!     fn add(left: u32, right: u32) -> u32;
//! }
//! ```
//! This macro will generate the structs `CalculatorClient`, `CalculatorService`, and a new trait
//! `Calculator` that you can use to implement the service as follows:
//! ```rust
//! struct CalculatorServiceImpl;
//! 
//! impl Calculator for CalculatorServiceImpl {
//!     fn add(&self, left: u32, right: u32) -> u32 {
//!         left + right
//!     }
//! }
//! ```
//! Note that the version of the trait emitted from the macro adds a `&self` receiver. Although not
//! used in this example, this is useful when we want the RPC to modify some state (via interior
//! mutability). Now that we have defined our RPC, let's create a client and server for it! In this
//! example, we will use [`MessageChannel`](https://docs.rs/web-sys/latest/web_sys/struct.MessageChannel.html)
//! since it is easy to construct and test, however, a more common case would be to construct the
//! channel from a [`Worker`](https://docs.rs/web-sys/latest/web_sys/struct.Worker.html) or a 
//! [`DedicatedWorkerGlobalScope`](https://docs.rs/web-sys/latest/web_sys/struct.DedicatedWorkerGlobalScope.html).
//! Let's start by defining the server:
//! ```rust
//! // create a MessageChannel
//! let channel = web_sys::MessageChannel::new();
//! // Create two interfaces from the ports. web_rpc::Interface::new is an async method that
//! // will return once the other end is ready, hence we need to poll both at the same time
//! let (server_interface, client_interface) = futures_util::future::join(
//!     web_rpc::Interface::new(channel.port1()),
//!     web_rpc::Interface::new(channel.port2()),
//! ).await;
//! // create a server with the first port
//! let server = web_rpc::Builder::new(server_interface)
//!     .with_service::<CalculatorService<_>>(CalculatorServiceImpl)
//!     .build();
//! // spawn the server
//! wasm_bindgen_futures::spawn_local(server);
//! ```
//! [`Interface::new`] is async since there is no way to synchronously check whether a channel or
//! a worker is ready to receive messages. To workaround this, temporary listeners are attached to
//! determine when a channel is ready for communication. The output of this method is a future that can 
//! be added to the browser's event loop using [wasm_bindgen_futures::spawn_local], however, this will
//! run the server indefinitely. For more control, consider wrapping the server with [futures_util::FutureExt::remote_handle]
//! before spawning it, which will shutdown the server once the handle has been dropped. Moving onto the
//! client:
//! ```rust
//! // create a client using the second port
//! let client = web_rpc::Builder::new(client_interface)
//!     .with_client::<CalculatorClient>()
//!     .build();
//! /* call `add` */
//! assert_eq!(client.add(41, 1).await, 42);
//! ```
//! That is it! Underneath the hood, the client will serialize its arguments and transfer the bytes to
//! server. The server will deserialize those arguments and run `<CalculatorServiceImpl as Calculator>::add`
//! before returning the result to the client. Note that we are only awaiting the response of the call to `add`,
//! the request itself is sent synchronously before we await anything.
//! 
//! Now that we have the basic idea of how define an RPC trait and set up a server and client, let's dive into
//! some of the more advanced features of this library!
//! 
//! ### Synchronous and asynchronous RPC methods
//! Server methods can be asynchronous! That is, you can define the following RPC trait and service
//! implementation:
//! 
//! ```rust
//! #[web_rpc::service]
//! pub trait Sleep {
//!     async fn sleep(interval: Duration);
//! }
//! 
//! struct SleepServiceImpl;
//! impl Sleep for SleepServiceImpl {
//!     async fn sleep(&self, interval: Duration) -> bool {
//!         gloo_timers::future::sleep(interval).await;
//!         // sleep completed (was not cancelled)
//!         true
//!     }
//! }
//! ```
//! Asynchronous RPC methods are run concurrently on the server and also support cancellation if the
//! future on the client side is dropped. However, such a future is only returned from a client
//! method if the RPC returns a value. Otherwise the RPC is considered a notification.
//! 
//! ### Notifications
//! Notifications are RPCs that do not return anything. On the client side, the method is completely
//! synchronous and also returns nothing. This setup is useful if you need to communicate with another
//! part of your application but cannot yield to the event loop.
//! 
//! The implication of this, however, is that even if the server method is asynchronous, we are unable
//! to cancel it from the client side since we do not have a future that can be dropped.
//! 
//! ### Posting and transferring Javascript types
//! In the example above, we discussed how the client serializes its arguments before sending them to 
//! the server. This approach is convenient, but how do send web types such as a `WebAssembly.Module`
//! or an `OffscreenCanvas` that have no serializable representation? Well, we are in luck since this
//! happens to be one of the key features of this crate. Consider the following RPC trait:
//! ```rust
//! #[web_rpc::service]
//! pub trait Concat {
//!     #[post(left, right, return)]
//!     fn concat_with_space(
//!         left: js_sys::JsString,
//!         right: js_sys::JsString
//!     ) -> js_sys::JsString;
//! }
//! ```
//! All we have done is added the `post` attribute to the method and listed the arguments that we would
//! like to be posted to the other side. Under the hood, the implementation of the client will then skip
//! these arguments during serialization and just append them after the serialized message to the array
//! that will be posted. As shown above, this also works for the return type by just specifying `return`
//! in the post attribute. For web types that need to be transferred, we simply wrap them in `transfer`
//! as follows:
//! ```rust
//! #[web_rpc::service]
//! pub trait GameEngine {
//!     #[post(transfer(canvas))]
//!     fn send_canvas(
//!         canvas: js_sys::OffscreenCanvas,
//!     );
//! }
//! ```
//! ### Bi-directional RPC
//! In the original example, we created a server on the first port of the message channel and a client on
//! the second port. However, it is possible to define both a client and a server on each side, enabling
//! bi-directional RPC. This is particularly useful if we want to send and receive messages from a worker
//! without sending it a seperate channel for the bi-directional communication. Our original example can
//! be extended as follows:
//! ```rust
//! /* create channel */
//! let channel = web_sys::MessageChannel::new().unwrap();
//! let (interface1, interface2) = futures_util::future::join(
//!     web_rpc::Interface::new(channel.port1()),
//!     web_rpc::Interface::new(channel.port2()),
//! ).await;
//! /* create server1 and client1 */
//! let (client1, server1) = web_rpc::Builder::new(interface1)
//!     .with_service::<CalculatorService<_>>(CalculatorServiceImpl)
//!     .with_client::<CalculatorClient>()
//!     .build();
//! /* create server2 and client2 */
//! let (client2, server2) = web_rpc::Builder::new(interface2)
//!     .with_service::<CalculatorService<_>>(CalculatorServiceImpl)
//!     .with_client::<CalculatorClient>()
//!     .build();
//! ```

use std::{cell::RefCell, marker::PhantomData, pin::Pin, rc::Rc, task::{Context, Poll}};

use futures_channel::mpsc;
use futures_core::{future::LocalBoxFuture, Future};
use futures_util::{FutureExt, StreamExt};
use gloo_events::EventListener;
use js_sys::{Uint8Array, ArrayBuffer};
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
pub mod service;
pub mod interface;
pub mod port;

pub use interface::Interface;

#[doc(hidden)]
#[derive(Serialize, Deserialize)]
pub enum Message<Request, Response> {
    Request(usize, Request),
    Abort(usize),
    Response(usize, Response),
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
    /// ```
    /// struct CalculatorServiceImpl;
    /// impl Calculator for CalculatorServiceImpl { /* add Calculator's methods */}
    /// let server = Builder::new(some_interface)
    ///     .with_service<CalculatorService<_>>(CalculatorServiceImpl)
    ///     .build();
    /// ```
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
    /// Configure the RPC interface with a client that allows you to execute RPCs on the
    /// server. The builder will automatically instansiate the client for you, you just
    /// need to provide the type which is generated via the [`macro@service`] attribute
    /// macro. For example, if you had a trait `Calculator` to which you applied the 
    /// [`macro@service`] attribute macro, the macro would have generated a `CalculatorClient`
    /// struct which you can use as the `C` in this function.
    pub fn with_client<C: client::Client>(
        self,
    ) -> Builder<C, S> {
        let Builder { interface, service, .. } = self;
        Builder { interface, client: PhantomData::<C>, service }
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

    /// Build function for client-only RPC interfaces.
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

    /// Build function for server-only RPC interfaces.
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
    
    /// Build function for client-server RPC interfaces.
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
