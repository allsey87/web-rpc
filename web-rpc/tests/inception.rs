use std::sync::OnceLock;

use futures_util::{future::RemoteHandle, FutureExt};
use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait FortyTwo {
    fn forty_two() -> u32;
}
struct FortyTwoServiceImpl;
impl FortyTwo for FortyTwoServiceImpl {
    fn forty_two(&self) -> u32 {
        42
    }
}

#[web_rpc::service]
pub trait Channel {
    #[post(transfer(return))]
    async fn start() -> web_sys::MessagePort;
}

#[derive(Default)]
struct ChannelServiceImpl {
    server_handle: OnceLock<RemoteHandle<()>>
}

impl Channel for ChannelServiceImpl {
    async fn start(&self) -> web_sys::MessagePort {
        /* create channel */
        let channel = web_sys::MessageChannel::new().unwrap();
        /* create and spawn server (shuts down when server_handle is dropped) */
        let (server, server_handle) = web_rpc::Builder::new(channel.port1())
            .with_service::<FortyTwoService<_>>(FortyTwoServiceImpl)
            .build().await
            .remote_handle();
        wasm_bindgen_futures::spawn_local(server);
        /* store the server_handle inside the struct so that it is not dropped */
        self.server_handle.set(server_handle)
            .map_err(|_| "OnceLock already set")
            .unwrap();
        /* return the second port */
        channel.port2()
    }
}

#[wasm_bindgen_test]
async fn inception() {
    console_error_panic_hook::set_once();
    /* create channel */
    let channel = web_sys::MessageChannel::new().unwrap();
    /* create and spawn server (shuts down when _server_handle is dropped) */
    let (server, _server_handle) = web_rpc::Builder::new(channel.port1())
        .with_service::<ChannelService<_>>(ChannelServiceImpl::default())
        .build().await
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    /* create client */
    let client = web_rpc::Builder::new(channel.port2())
        .with_client::<ChannelClient>()
        .build().await;
    /* run test */
    let port3 = client.start().await;
    let client = web_rpc::Builder::new(port3)
        .with_client::<FortyTwoClient>()
        .build().await;
    assert_eq!(client.forty_two().await, 42);
}

