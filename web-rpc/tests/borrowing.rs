mod common;

use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait Greeter {
    fn greet(&self, name: &str, greeting: &str) -> String;
    fn count_bytes(&self, data: &[u8]) -> u32;
    fn mixed(&self, name: &str, id: u32) -> String;
    fn notify(&self, message: &str);
}

struct GreeterImpl;
impl Greeter for GreeterImpl {
    fn greet(&self, name: &str, greeting: &str) -> String {
        format!("{greeting}, {name}!")
    }
    fn count_bytes(&self, data: &[u8]) -> u32 {
        data.len() as u32
    }
    fn mixed(&self, name: &str, id: u32) -> String {
        format!("{name}#{id}")
    }
    fn notify(&self, _message: &str) {}
}

#[wasm_bindgen_test]
async fn borrowing() {
    let (client, _server) = common::connect::<GreeterService<_>, GreeterClient>(GreeterImpl).await;
    assert_eq!(client.greet("World", "Hello").await, "Hello, World!");
    assert_eq!(client.count_bytes(&[1, 2, 3, 4, 5]).await, 5);
    assert_eq!(client.mixed("Alice", 42).await, "Alice#42");
    client.notify("test");
}
