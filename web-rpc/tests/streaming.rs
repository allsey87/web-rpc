use std::time::Duration;

use futures_core::Stream;
use futures_util::{FutureExt, StreamExt};
use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait DataSource {
    fn stream_data(&self, count: u32) -> impl Stream<Item = u32>;
}

struct DataSourceImpl;
impl DataSource for DataSourceImpl {
    fn stream_data(&self, count: u32) -> impl Stream<Item = u32> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        for i in 0..count {
            let _ = tx.unbounded_send(i);
        }
        rx
    }
}

/// Basic test: server pushes N items, client collects all
#[wasm_bindgen_test]
async fn basic_streaming() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _server_handle) = web_rpc::Builder::new(server_interface)
        .with_service::<DataSourceService<_>>(DataSourceImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<DataSourceClient>()
        .build();

    let items: Vec<u32> = client.stream_data(5).collect().await;
    assert_eq!(items, vec![0, 1, 2, 3, 4]);
}

/// Empty stream: server returns immediately without sending items
#[wasm_bindgen_test]
async fn empty_stream() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _server_handle) = web_rpc::Builder::new(server_interface)
        .with_service::<DataSourceService<_>>(DataSourceImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<DataSourceClient>()
        .build();

    let items: Vec<u32> = client.stream_data(0).collect().await;
    assert_eq!(items, Vec::<u32>::new());
}

/// Mixed methods: service with both unary and streaming methods
#[web_rpc::service]
pub trait Mixed {
    fn add(&self, left: u32, right: u32) -> u32;
    fn stream_range(&self, start: u32, end: u32) -> impl Stream<Item = u32>;
}

struct MixedImpl;
impl Mixed for MixedImpl {
    fn add(&self, left: u32, right: u32) -> u32 {
        left + right
    }
    fn stream_range(&self, start: u32, end: u32) -> impl Stream<Item = u32> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        for i in start..end {
            let _ = tx.unbounded_send(i);
        }
        rx
    }
}

#[wasm_bindgen_test]
async fn mixed_methods() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _server_handle) = web_rpc::Builder::new(server_interface)
        .with_service::<MixedService<_>>(MixedImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<MixedClient>()
        .build();

    // Unary method works
    assert_eq!(client.add(10, 20).await, 30);

    // Streaming method works
    let items: Vec<u32> = client.stream_range(3, 7).collect().await;
    assert_eq!(items, vec![3, 4, 5, 6]);
}

/// Slow streaming service for testing abort
#[web_rpc::service]
pub trait SlowStream {
    async fn slow_count(&self, target: u32, interval_ms: u32) -> impl Stream<Item = u32>;
}

use std::{cell::RefCell, rc::Rc};

struct SlowStreamImpl {
    count: Rc<RefCell<u32>>,
}

impl SlowStream for SlowStreamImpl {
    async fn slow_count(
        &self,
        target: u32,
        interval_ms: u32,
    ) -> impl Stream<Item = u32> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        let count = self.count.clone();
        let interval = Duration::from_millis(interval_ms as u64);
        wasm_bindgen_futures::spawn_local(async move {
            for i in 0..target {
                gloo_timers::future::sleep(interval).await;
                *count.borrow_mut() += 1;
                if tx.unbounded_send(i).is_err() {
                    break;
                }
            }
        });
        rx
    }
}

/// Abort via drop: client drops receiver midway, server stops producing
#[wasm_bindgen_test]
async fn abort_via_drop() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let count = Rc::new(RefCell::new(0u32));
    let service = SlowStreamImpl {
        count: count.clone(),
    };
    let (server, _server_handle) = web_rpc::Builder::new(server_interface)
        .with_service::<SlowStreamService<_>>(service)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<SlowStreamClient>()
        .build();

    // Start a stream that would produce 100 items at 50ms intervals
    let mut stream = client.slow_count(100, 50);

    // Read a few items
    let first = stream.next().await;
    assert_eq!(first, Some(0));
    let second = stream.next().await;
    assert_eq!(second, Some(1));

    // Drop the stream
    std::mem::drop(stream);

    // Wait to ensure the server stops
    gloo_timers::future::sleep(Duration::from_millis(300)).await;

    // Server should have stopped early (not reached 100)
    let final_count = *count.borrow();
    assert!(
        final_count < 10,
        "server sent {} items, expected < 10",
        final_count
    );
}

/// Close + drain: client calls close(), drains remaining buffered items
#[wasm_bindgen_test]
async fn close_and_drain() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let count = Rc::new(RefCell::new(0u32));
    let service = SlowStreamImpl {
        count: count.clone(),
    };
    let (server, _server_handle) = web_rpc::Builder::new(server_interface)
        .with_service::<SlowStreamService<_>>(service)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<SlowStreamClient>()
        .build();

    let mut stream = client.slow_count(100, 50);

    // Read a couple items
    assert_eq!(stream.next().await, Some(0));
    assert_eq!(stream.next().await, Some(1));

    // Close the stream (sends abort) but keep draining
    stream.close();

    // Drain remaining buffered items until the stream ends
    let remaining: Vec<u32> = stream.collect().await;

    // The stream terminated (this is the key assertion — collect returns)
    let _ = remaining;
}

/// Streaming with borrowed args
#[web_rpc::service]
pub trait BorrowedStream {
    fn stream_prefixed(&self, prefix: &str) -> impl Stream<Item = String>;
}

struct BorrowedStreamImpl;
impl BorrowedStream for BorrowedStreamImpl {
    fn stream_prefixed(&self, prefix: &str) -> impl Stream<Item = String> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        for i in 0..3 {
            let _ = tx.unbounded_send(format!("{}-{}", prefix, i));
        }
        rx
    }
}

#[wasm_bindgen_test]
async fn streaming_with_borrowed_args() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _server_handle) = web_rpc::Builder::new(server_interface)
        .with_service::<BorrowedStreamService<_>>(BorrowedStreamImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<BorrowedStreamClient>()
        .build();

    let items: Vec<String> = client.stream_prefixed("hello").collect().await;
    assert_eq!(items, vec!["hello-0", "hello-1", "hello-2"]);
}

/// Streaming with #[post(return)] — JS values bypass serialization
#[web_rpc::service]
pub trait PostStream {
    #[post(return)]
    fn stream_js_strings(&self, count: u32) -> impl Stream<Item = js_sys::JsString>;
}

struct PostStreamImpl;
impl PostStream for PostStreamImpl {
    fn stream_js_strings(
        &self,
        count: u32,
    ) -> impl Stream<Item = js_sys::JsString> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        for i in 0..count {
            let _ = tx.unbounded_send(js_sys::JsString::from(format!("item-{}", i)));
        }
        rx
    }
}

#[wasm_bindgen_test]
async fn streaming_post_return() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _server_handle) = web_rpc::Builder::new(server_interface)
        .with_service::<PostStreamService<_>>(PostStreamImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<PostStreamClient>()
        .build();

    let items: Vec<js_sys::JsString> = client.stream_js_strings(3).collect().await;
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], "item-0");
    assert_eq!(items[1], "item-1");
    assert_eq!(items[2], "item-2");
}
