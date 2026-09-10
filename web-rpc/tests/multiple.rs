mod common;

use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait Calculator {
    fn add(&self, left: u32, right: u32) -> u32;
    fn is_forty_two(&self, value: u32) -> bool;
}

struct CalculatorImpl;
impl Calculator for CalculatorImpl {
    fn add(&self, left: u32, right: u32) -> u32 {
        left + right
    }
    fn is_forty_two(&self, value: u32) -> bool {
        value == 42
    }
}

#[wasm_bindgen_test]
async fn multiple() {
    let (client, _server) =
        common::connect::<CalculatorService<_>, CalculatorClient>(CalculatorImpl).await;
    let sum = client.add(41, 1).await;
    assert!(client.is_forty_two(sum).await);
}
