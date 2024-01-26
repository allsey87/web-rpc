use std::{sync::Arc, time::Duration};

use futures_util::FutureExt;
use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait CountSlowly {
    async fn count_slowly(to: u32, interval: Duration);
}

#[derive(Default)]
struct CountSlowlyServiceImpl(std::sync::Mutex<Vec<u32>>);

impl CountSlowly for CountSlowlyServiceImpl {
    async fn count_slowly(&self, to: u32, interval: Duration) {
        let mut count: u32 = Default::default();
        while count < to {
            self.0.lock().unwrap().push(count);
            count += 1;
            gloo_timers::future::sleep(interval).await;
        }
    }
}
#[wasm_bindgen_test]
async fn abort_via_drop() {
    console_error_panic_hook::set_once();
    /* create channel */
    let channel = web_sys::MessageChannel::new().unwrap();
    /* create and spawn server (shuts down when _server_handle is dropped) */
    let service_impl: Arc<CountSlowlyServiceImpl> = Default::default();
    let (server, _server_handle) = web_rpc::Builder::new(channel.port1())
        .with_service(CountSlowlyService::new(service_impl.clone()))
        .build().await
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    /* create client */
    let client = web_rpc::Builder::new(channel.port2())
        .with_client::<CountSlowlyClient>()
        .build().await;
    /* run test */
    client.count_slowly(5, Duration::from_millis(100));
    gloo_timers::future::sleep(Duration::from_millis(250)).await;
    let server_impl_vec = service_impl.0.lock().unwrap();
    assert_eq!(*server_impl_vec, vec![0, 1, 2])
    
}
