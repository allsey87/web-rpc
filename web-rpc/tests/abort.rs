use std::{cell::RefCell, rc::Rc, time::Duration};

use futures_util::FutureExt;
use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait CountSlowly {
    async fn count_slowly(target: u32, interval: Duration) -> u32;
}

impl CountSlowly for RefCell<u32> {
    async fn count_slowly(&self, target: u32, interval: Duration) -> u32 {
        loop {
            if self.replace_with(|value| value.wrapping_add(1)) == target {
                break target;
            }
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
    let service_impl: Rc<RefCell<u32>> = Default::default();
    let (server, _server_handle) = web_rpc::Builder::new(channel.port1())
        .with_service::<CountSlowlyService<_>>(service_impl.clone())
        .build().await
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    /* create client */
    let client = web_rpc::Builder::new(channel.port2())
        .with_client::<CountSlowlyClient>()
        .build().await;
    /* run test */
    let mut count = client.count_slowly(10, Duration::from_millis(100)).fuse();
    let mut timeout = gloo_timers::future::sleep(Duration::from_millis(250)).fuse();
    futures_util::select! {
        _ = &mut count => panic!("`count` completed"),
        _ = &mut timeout => std::mem::drop(count)
    };
    gloo_timers::future::sleep(Duration::from_millis(250)).await;
    assert_eq!(*service_impl.borrow(), 3);
}
