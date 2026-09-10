//! A connected service and client over a `MessageChannel`, which is what nearly every test
//! starts from.

#![allow(dead_code)]

use futures_util::{future::RemoteHandle, FutureExt};
use serde::{de::DeserializeOwned, Serialize};
use web_rpc::{client, service, Builder, Interface};

/// Start a `MessageChannel` and hand back its ports. web-rpc starts nothing itself.
pub fn channel() -> web_sys::MessageChannel {
    let channel = web_sys::MessageChannel::new().unwrap();
    channel.port1().start();
    channel.port2().start();
    channel
}

/// Serve `implementation` on one end of a fresh channel and return a client for the other.
/// The server runs until the returned handle is dropped.
pub async fn connect<S, C>(implementation: impl Into<S>) -> (C, RemoteHandle<()>)
where
    S: service::Service + 'static,
    S::Response: Serialize,
    C: client::Client + From<client::State<C::Response>> + 'static,
    C::Response: DeserializeOwned,
{
    console_error_panic_hook::set_once();
    let channel = channel();
    let (server_interface, client_interface) = futures_util::future::join(
        Interface::new(channel.port1()),
        Interface::new(channel.port2()),
    )
    .await;
    let (server, handle) = Builder::new(server_interface)
        .with_service::<S>(implementation)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = Builder::new(client_interface).with_client::<C>().build();
    (client, handle)
}
