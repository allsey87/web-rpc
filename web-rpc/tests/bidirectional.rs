use futures_util::{future::join, FutureExt};
use wasm_bindgen_test::*;

/* define the service */
#[web_rpc::service]
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

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

#[wasm_bindgen_test]
async fn bidirectional() {
    console_error_panic_hook::set_once();
    /* create channel */
    let channel = web_sys::MessageChannel::new().unwrap();
    /* create server1 and client1 */
    let (client1, server1) = web_rpc::Builder::new(channel.port1())
        .with_server(ServiceServer::new(ServiceServerImpl))
        .with_client::<ServiceClient>()
        .build().await;
    /* create server2 and client2 */
    let (client2, server2) = web_rpc::Builder::new(channel.port2())
        .with_server(ServiceServer::new(ServiceServerImpl))
        .with_client::<ServiceClient>()
        .build().await;
    /* spawn the servers */
    let (server1, _server_handle1) = server1.remote_handle();
    let (server2, _server_handle2) = server2.remote_handle();
    wasm_bindgen_futures::spawn_local(server1);
    wasm_bindgen_futures::spawn_local(server2);
    /* run test */
    match join(client1.add(1, 2), client2.add(3, 4)).await {
        (Ok(result1), Ok(result2)) => match (result1, result2) {
            (3, 7) => {}
            _ => panic!("incorrect result")
        }
        _ => panic!("RPC error")
    }
}