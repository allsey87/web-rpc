use wasm_bindgen_test::*;

#[worker_rpc::service]
pub trait Service {
    fn add(left: u32, right: u32) -> u32;
}
struct ServiceServerImpl;
impl Service for ServiceServerImpl {
    fn add(&self, left: u32, right: u32) -> u32 {
        left + right
    }
}

#[wasm_bindgen_test]
async fn unidirectional() {
    let channel = web_sys::MessageChannel::new().unwrap();
    let port1 = channel.port1();
    let port2 = channel.port2();
    let server = ServiceServer::new(ServiceServerImpl);
    let _port1_interface = worker_rpc::Interface::from(port1)
        .with_server(server)
        .connect();
    let port2_interface = worker_rpc::Interface::from(port2)
        .with_client::<ServiceClient>()
        .connect();
    assert_eq!(port2_interface.add(41, 1).await.expect("RPC failure"), 42);
}

