//! A `MessagePort` transferred through one connection carries a second connection.

mod common;

use std::sync::OnceLock;

use futures_util::{future::RemoteHandle, FutureExt};
use wasm_bindgen_test::*;
use web_rpc::wrap::Transfer;

#[web_rpc::service]
pub trait FortyTwo {
    fn forty_two(&self) -> u32;
}

struct FortyTwoImpl;
impl FortyTwo for FortyTwoImpl {
    fn forty_two(&self) -> u32 {
        42
    }
}

#[web_rpc::service]
pub trait Channel {
    fn start(&self) -> Transfer<web_sys::MessagePort>;
}

#[derive(Default)]
struct ChannelImpl {
    server: OnceLock<RemoteHandle<()>>,
}

impl Channel for ChannelImpl {
    fn start(&self) -> Transfer<web_sys::MessagePort> {
        let channel = common::channel();
        // The handshake completes only once the client has the other port, so the server is
        // spawned rather than awaited here.
        let (server, handle) = web_rpc::Interface::new(channel.port1())
            .then(|interface| {
                web_rpc::Builder::new(interface)
                    .with_service::<FortyTwoService<_>>(FortyTwoImpl)
                    .build()
            })
            .remote_handle();
        wasm_bindgen_futures::spawn_local(server);
        assert!(self.server.set(handle).is_ok(), "started twice");
        Transfer(channel.port2())
    }
}

#[wasm_bindgen_test]
async fn inception() {
    let (client, _server) =
        common::connect::<ChannelService<_>, ChannelClient>(ChannelImpl::default()).await;
    let port = client.start().await.into_inner();
    // The transferred port is ours, so starting it is ours too.
    port.start();
    let inner = web_rpc::Builder::new(web_rpc::Interface::new(port).await)
        .with_client::<FortyTwoClient>()
        .build();
    assert_eq!(inner.forty_two().await, 42);
}
