mod common;

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
async fn unidirectional() {
    let (client, _server) =
        common::connect::<CalculatorService<_>, CalculatorClient>(CalculatorImpl).await;
    assert_eq!(client.add(41, 1).await, 42);
}
