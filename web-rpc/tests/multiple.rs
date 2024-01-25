use futures_util::FutureExt;
use wasm_bindgen_test::*;

#[web_rpc::service]
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
    console_error_panic_hook::set_once();
    /* create channel */
    let channel = web_sys::MessageChannel::new().unwrap();
    /* create and spawn server (shuts down when _server_handle is dropped) */
    let (server, _server_handle) = web_rpc::Builder::new(channel.port1())
        .with_server(ServiceServer::new(ServiceServerImpl))
        .build().await
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    /* create client */
    let client = web_rpc::Builder::new(channel.port2())
        .with_client::<ServiceClient>()
        .build().await;
    let add_response = client.add(41, 1).await
        .expect("RPC failure");
    let is_forty_two_response = client.is_forty_two(add_response).await
        .expect("RPC failure");
    assert!(is_forty_two_response);
}