use futures_util::FutureExt;
use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait Greeter {
    fn greet(name: &str, greeting: &str) -> String;
    fn count_bytes(data: &[u8]) -> usize;
    fn mixed(name: &str, id: u32) -> String;
    fn notify(message: &str);
}

struct GreeterServiceImpl;
impl Greeter for GreeterServiceImpl {
    fn greet(&self, name: &str, greeting: &str) -> String {
        format!("{greeting}, {name}!")
    }
    fn count_bytes(&self, data: &[u8]) -> usize {
        data.len()
    }
    fn mixed(&self, name: &str, id: u32) -> String {
        format!("{name}#{id}")
    }
    fn notify(&self, _message: &str) {}
}

#[wasm_bindgen_test]
async fn borrowing() {
    console_error_panic_hook::set_once();
    /* create channel */
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    /* create and spawn server */
    let (server, _server_handle) = web_rpc::Builder::new(server_interface)
        .with_service::<GreeterService<_>>(GreeterServiceImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    /* create client */
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<GreeterClient>()
        .build();
    /* test &str parameters */
    assert_eq!(client.greet("World", "Hello").await, "Hello, World!");
    /* test &[u8] parameters */
    assert_eq!(client.count_bytes(&[1, 2, 3, 4, 5]).await, 5);
    /* test mixed owned and borrowed parameters */
    assert_eq!(client.mixed("Alice", 42).await, "Alice#42");
    /* test notification with &str (fire and forget) */
    client.notify("test");
}
