use std::sync::OnceLock;

use wasm_bindgen_test::*;
use worker_rpc::ConnectedInterface;

#[worker_rpc::service]
pub trait FortyTwoService {
    fn forty_two() -> u32;
}
struct FortyTwoServiceImpl;
impl FortyTwoService for FortyTwoServiceImpl {
    fn forty_two(&self) -> u32 {
        42
    }
}

#[worker_rpc::service]
pub trait Service {
    #[post(transfer(return))]
    fn start() -> web_sys::MessagePort;
}
#[derive(Default)]
struct ServiceServerImpl {
    interface: OnceLock<ConnectedInterface<worker_rpc::client::None>>
}
impl Service for ServiceServerImpl {
    fn start(&self) -> web_sys::MessagePort {
        let channel = web_sys::MessageChannel::new().unwrap();
        let port1 = channel.port1();
        let port2 = channel.port2();
        /* create the forty two service server with port1 and return port2 */
        let server = FortyTwoServiceServer::new(FortyTwoServiceImpl);
        let interface = worker_rpc::Interface::from(port1)
            .with_server(server)
            .connect();
        /* store the interface inside the impl so that it is not dropped */
        self.interface.set(interface)
            .map_err(|_| "OnceLock already set")
            .unwrap();
        port2
    }
}

#[wasm_bindgen_test]
async fn inception() {
    let channel = web_sys::MessageChannel::new().unwrap();
    let port1 = channel.port1();
    let port2 = channel.port2();
    let server = ServiceServer::new(ServiceServerImpl::default());
    let _port1_interface = worker_rpc::Interface::from(port1)
        .with_server(server)
        .connect();
    let port2_interface = worker_rpc::Interface::from(port2)
        .with_client::<ServiceClient>()
        .connect();

    let port3 = port2_interface.start().await.expect("RPC failure");
    let port3_interface = worker_rpc::Interface::from(port3)
        .with_client::<FortyTwoServiceClient>()
        .connect();
    assert_eq!(port3_interface.forty_two().await.expect("RPC failure"), 42);
}

