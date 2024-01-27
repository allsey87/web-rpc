use std::time::Duration;

use futures_util::FutureExt;
use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait Sleep {
    async fn sleep(duration: Duration);
}
struct SleepServiceImpl;
impl Sleep for SleepServiceImpl {
    async fn sleep(&self, duration: Duration) {
        gloo_timers::future::sleep(duration).await;
    }
}

async fn test_banket_impl<S: Sleep + 'static>(server_impl: S) {
    console_error_panic_hook::set_once();
    /* create channel */
    let channel = web_sys::MessageChannel::new().unwrap();
    /* create and spawn server (shuts down when _server_handle is dropped) */
    let (server, _server_handle) = web_rpc::Builder::new(channel.port1())
        .with_service::<SleepService<_>>(server_impl)
        .build().await
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    /* create client */
    let client = web_rpc::Builder::new(channel.port2())
        .with_client::<SleepClient>()
        .build().await;
    client.sleep(Duration::default())
}

#[wasm_bindgen_test]
async fn arc() {
    console_error_panic_hook::set_once();
    test_banket_impl(std::sync::Arc::new(SleepServiceImpl)).await;
}

#[wasm_bindgen_test]
async fn rc() {
    console_error_panic_hook::set_once();
    test_banket_impl(std::rc::Rc::new(SleepServiceImpl)).await;
}

#[wasm_bindgen_test]
async fn boxed() {
    console_error_panic_hook::set_once();
    test_banket_impl(std::boxed::Box::new(SleepServiceImpl)).await;
}
