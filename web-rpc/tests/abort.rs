mod common;

use std::{cell::RefCell, rc::Rc, time::Duration};

use futures_util::FutureExt;
use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait CountSlowly {
    async fn count_slowly(&self, target: u32, interval: Duration) -> u32;
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
    let counter: Rc<RefCell<u32>> = Default::default();
    let (client, _server) =
        common::connect::<CountSlowlyService<_>, CountSlowlyClient>(counter.clone()).await;
    let mut count = client.count_slowly(10, Duration::from_millis(100)).fuse();
    let mut timeout = gloo_timers::future::sleep(Duration::from_millis(250)).fuse();
    futures_util::select! {
        _ = &mut count => panic!("`count` completed"),
        _ = &mut timeout => std::mem::drop(count)
    };
    gloo_timers::future::sleep(Duration::from_millis(250)).await;
    assert_eq!(*counter.borrow(), 3);
}

#[wasm_bindgen_test]
async fn a_request_outlives_its_client() {
    let counter: Rc<RefCell<u32>> = Default::default();
    let (client, _server) =
        common::connect::<CountSlowlyService<_>, CountSlowlyClient>(counter.clone()).await;
    let pending = client.count_slowly(2, Duration::from_millis(10));
    std::mem::drop(client);
    assert_eq!(pending.await, 2);
}
