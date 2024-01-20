use wasm_bindgen_test::*;

/* define the service */
#[worker_rpc::service]
pub trait Service {
    fn add(left: u32, right: u32) -> u32;
}
/* implement the server */
struct ServiceServerImpl;
impl Service for ServiceServerImpl {
    fn add(&self, left: u32, right: u32) -> u32 {
        left + right
    }
}

#[wasm_bindgen_test]
async fn bidirectional() {
    /* create a message channel and an interface for each port */
    let channel = web_sys::MessageChannel::new().unwrap();
    let port1 = channel.port1();
    let port2 = channel.port2();
    let server = ServiceServer::new(ServiceServerImpl);
    let port1_interface = worker_rpc::Interface::from(port1)
        .with_client::<ServiceClient>()
        .with_server(server)
        .connect();
    let server = ServiceServer::new(ServiceServerImpl);
    let port2_interface = worker_rpc::Interface::from(port2)
        .with_client::<ServiceClient>()
        .with_server(server)
        .connect();
    
    let (port1_result, port2_result) =
        futures_util::join!(port1_interface.add(1, 2), port2_interface.add(3, 4));

    assert_eq!(port1_result.expect("RPC failure"), 3);
    assert_eq!(port2_result.expect("RPC failure"), 7);
}