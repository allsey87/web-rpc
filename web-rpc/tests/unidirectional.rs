use futures_util::FutureExt;
use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait Calculator {
    fn add(left: u32, right: u32) -> u32;
}
struct CalculatorServiceImpl;
impl Calculator for CalculatorServiceImpl {
    fn add(&self, left: u32, right: u32) -> u32 {
        left + right
    }
}

#[wasm_bindgen_test]
async fn unidirectional() {
    console_error_panic_hook::set_once();
    /* create channel */
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    ).await;
    /* create and spawn server (shuts down when _server_handle is dropped) */
    let (server, _server_handle) = web_rpc::Builder::new(server_interface)
        .with_service::<CalculatorService<_>>(CalculatorServiceImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    /* create client */
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<CalculatorClient>()
        .build();
    /* run test */
    assert_eq!(client.add(41, 1).await, 42);
}

