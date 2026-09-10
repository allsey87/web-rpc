mod common;

use futures_util::{future::join, FutureExt};
use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait Calculator {
    fn add(&self, left: u32, right: u32) -> u32;
}

struct CalculatorImpl;
impl Calculator for CalculatorImpl {
    fn add(&self, left: u32, right: u32) -> u32 {
        left + right
    }
}

#[wasm_bindgen_test]
async fn bidirectional() {
    console_error_panic_hook::set_once();
    let channel = common::channel();
    let (interface1, interface2) = join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (client1, server1) = web_rpc::Builder::new(interface1)
        .with_service::<CalculatorService<_>>(CalculatorImpl)
        .with_client::<CalculatorClient>()
        .build();
    let (client2, server2) = web_rpc::Builder::new(interface2)
        .with_service::<CalculatorService<_>>(CalculatorImpl)
        .with_client::<CalculatorClient>()
        .build();
    let (server1, _handle1) = server1.remote_handle();
    let (server2, _handle2) = server2.remote_handle();
    wasm_bindgen_futures::spawn_local(server1);
    wasm_bindgen_futures::spawn_local(server2);
    assert_eq!(join(client1.add(1, 2), client2.add(3, 4)).await, (3, 7));
}
