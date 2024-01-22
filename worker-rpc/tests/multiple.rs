use wasm_bindgen_test::*;

#[worker_rpc::service]
pub trait Service {
    fn add(left: u32, right: u32) -> u32;
    fn is_forty_two(value: u32) -> bool;
}
struct ServiceServerImpl;
impl Service for ServiceServerImpl {
    fn add(&self, left: u32, right: u32) -> u32 {
        left + right
    }
    fn is_forty_two(&self, value: u32) -> bool {
        value == 42
    }
}

#[wasm_bindgen_test]
async fn post() {
    let channel = web_sys::MessageChannel::new().unwrap();
    let port1 = channel.port1();
    let port2 = channel.port2();
    let server = ServiceServer::new(ServiceServerImpl);
    let _server_interface = worker_rpc::Interface::from(port1)
        .with_server(server)
        .connect();
    let client_interface = worker_rpc::Interface::from(port2)
        .with_client::<ServiceClient>()
        .connect();
    let add_response = client_interface.add(41, 1).await
        .expect("RPC failure");
    let is_forty_two_response = client_interface.is_forty_two(add_response).await
        .expect("RPC failure");
    assert!(is_forty_two_response);
}