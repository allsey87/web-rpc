use std::time::Duration;

use futures_util::FutureExt;
use wasm_bindgen_test::*;

#[worker_rpc::service]
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
    let channel = web_sys::MessageChannel::new().unwrap();
    let port1 = channel.port1();
    let port2 = channel.port2();
    let server_impl = std::sync::Arc::new(ServiceServerImpl::default());
    let server = ServiceServer::new(server_impl.clone());
    let _port1_interface = worker_rpc::Interface::from(port1)
        .with_server(server)
        .connect();
    let port2_interface = worker_rpc::Interface::from(port2)
        .with_client::<ServiceClient>()
        .connect();
    let mut count = port2_interface.count_slowly(5, Duration::from_millis(100)).fuse();
    let mut timeout = gloo_timers::future::sleep(Duration::from_millis(250)).fuse();
    futures_util::select! {
        _ = &mut count => panic!("`count` should not have completed"),
        _ = &mut timeout => std::mem::drop(count)
    }
    gloo_timers::future::sleep(Duration::from_millis(100)).await;
    let server_impl_vec = server_impl.0.lock().unwrap();
    assert_eq!(*server_impl_vec, vec![0, 1, 2])
}
