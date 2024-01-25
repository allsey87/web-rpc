use std::time::Duration;

use futures_util::FutureExt;
use wasm_bindgen_test::*;

#[worker_rpc::service]
pub trait Service {
    async fn sleep(duration: Duration);
}
struct ServiceServerImpl;
impl Service for ServiceServerImpl {
    async fn sleep(&self, duration: Duration) {
        gloo_timers::future::sleep(duration).await;
    }
}

async fn test_banket_impl<S: Service + 'static>(server_impl: S) -> worker_rpc::Result<()> {
    console_error_panic_hook::set_once();
    /* create channel */
    let channel = web_sys::MessageChannel::new().unwrap();
    /* create and spawn server (shuts down when _server_handle is dropped) */
    let (server, _server_handle) = worker_rpc::Builder::new(channel.port1())
        .with_server(ServiceServer::new(server_impl))
        .build().await
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    /* create client */
    let client = worker_rpc::Builder::new(channel.port2())
        .with_client::<ServiceClient>()
        .build().await;
    client.sleep(Duration::default()).await
}

#[wasm_bindgen_test]
async fn arc() {
    console_error_panic_hook::set_once();
    test_banket_impl(std::sync::Arc::new(ServiceServerImpl)).await
        .expect("RPC failure");
}

#[wasm_bindgen_test]
async fn rc() {
    console_error_panic_hook::set_once();
    test_banket_impl(std::rc::Rc::new(ServiceServerImpl)).await
        .expect("RPC failure");
}

#[wasm_bindgen_test]
async fn boxed() {
    console_error_panic_hook::set_once();
    test_banket_impl(std::boxed::Box::new(ServiceServerImpl)).await
        .expect("RPC failure");
}
