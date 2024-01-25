use std::{sync::Arc, time::Duration};

use futures_util::{future::{self, Either}, FutureExt};
use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait Service {
    async fn count_slowly(to: u32, interval: Duration);
}

#[derive(Default)]
struct ServiceServerImpl(std::sync::Mutex<Vec<u32>>);

impl Service for ServiceServerImpl {
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
    let server_impl: Arc<ServiceServerImpl> = Default::default();
    let (server, _server_handle) = web_rpc::Builder::new(channel.port1())
        .with_server(ServiceServer::new(server_impl.clone()))
        .build().await
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    /* create client */
    let client = web_rpc::Builder::new(channel.port2())
        .with_client::<ServiceClient>()
        .build().await;
    /* run test */
    let count = client.count_slowly(5, Duration::from_millis(100));
    let timeout = gloo_timers::future::sleep(Duration::from_millis(250));
    if let Either::Left(_) = future::select(count, timeout).await {
        panic!("`count` should not have completed");
    }
    gloo_timers::future::sleep(Duration::from_millis(100)).await;
    let server_impl_vec = server_impl.0.lock().unwrap();
    assert_eq!(*server_impl_vec, vec![0, 1, 2])
    
}
