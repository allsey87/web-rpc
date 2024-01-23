use std::time::Duration;

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
    let channel = web_sys::MessageChannel::new().unwrap();
    let port1 = channel.port1();
    let port2 = channel.port2();
    let _port1_interface = worker_rpc::Interface::from(port1)
        .with_server(ServiceServer::new(server_impl))
        .connect();
    let port2_interface = worker_rpc::Interface::from(port2)
        .with_client::<ServiceClient>()
        .connect();
    port2_interface.sleep(Duration::default()).await
}

#[wasm_bindgen_test]
async fn arc() {
    test_banket_impl(std::sync::Arc::new(ServiceServerImpl)).await
        .expect("RPC failure");
}

#[wasm_bindgen_test]
async fn rc() {
    test_banket_impl(std::rc::Rc::new(ServiceServerImpl)).await
        .expect("RPC failure");
}

#[wasm_bindgen_test]
async fn boxed() {
    test_banket_impl(std::boxed::Box::new(ServiceServerImpl)).await
        .expect("RPC failure");
}
