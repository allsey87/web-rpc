use std::sync::OnceLock;

use futures_util::{future::RemoteHandle, FutureExt};
use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait FortyTwo {
    fn forty_two(&self) -> u32;
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
    fn start(&self) -> web_sys::MessagePort;
}

#[derive(Default)]
struct ChannelServiceImpl {
    server_handle: OnceLock<RemoteHandle<()>>,
}

impl Channel for ChannelServiceImpl {
    fn start(&self) -> web_sys::MessagePort {
        /* create channel */
        let channel = web_sys::MessageChannel::new().unwrap();
        /* web_rpc::Interface::new will not complete until the same operation
        is performed on port2, hence we combine these futures and spawn it
        on the event loop, before returning port2 to the client */
        let (server, server_handle) = web_rpc::Interface::new(channel.port1())
            .then(|interface| {
                web_rpc::Builder::new(interface)
                    .with_service::<FortyTwoService<_>>(FortyTwoServiceImpl)
                    .build()
            })
            .remote_handle();
        wasm_bindgen_futures::spawn_local(server);
        /* store the server_handle inside the struct so that it is not dropped */
        self.server_handle
            .set(server_handle)
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
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    /* create and spawn server (shuts down when _server_handle is dropped) */
    let (server, _server_handle) = web_rpc::Builder::new(server_interface)
        .with_service::<ChannelService<_>>(ChannelServiceImpl::default())
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    /* create client */
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<ChannelClient>()
        .build();
    /* run test */
    let remote_client = client
        .start()
        .then(web_rpc::Interface::new)
        .map(|interface| {
            web_rpc::Builder::new(interface)
                .with_client::<FortyTwoClient>()
                .build()
        })
        .await;
    assert_eq!(remote_client.forty_two().await, 42);
}
