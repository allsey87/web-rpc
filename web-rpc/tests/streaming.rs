mod common;

use std::{cell::RefCell, rc::Rc, time::Duration};

use futures_core::Stream;
use futures_util::StreamExt;
use wasm_bindgen_test::*;
use web_rpc::wrap::Post;

fn stream_of<T: 'static>(items: impl IntoIterator<Item = T>) -> impl Stream<Item = T> {
    let (tx, rx) = futures_channel::mpsc::unbounded();
    for item in items {
        let _ = tx.unbounded_send(item);
    }
    rx
}

#[web_rpc::service]
pub trait DataSource {
    fn stream_data(&self, count: u32) -> impl Stream<Item = u32>;
}

struct DataSourceImpl;
impl DataSource for DataSourceImpl {
    fn stream_data(&self, count: u32) -> impl Stream<Item = u32> {
        stream_of(0..count)
    }
}

#[wasm_bindgen_test]
async fn basic_streaming() {
    let (client, _server) =
        common::connect::<DataSourceService<_>, DataSourceClient>(DataSourceImpl).await;
    let items: Vec<u32> = client.stream_data(5).collect().await;
    assert_eq!(items, vec![0, 1, 2, 3, 4]);
}

#[wasm_bindgen_test]
async fn empty_stream() {
    let (client, _server) =
        common::connect::<DataSourceService<_>, DataSourceClient>(DataSourceImpl).await;
    let items: Vec<u32> = client.stream_data(0).collect().await;
    assert_eq!(items, Vec::<u32>::new());
}

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
        stream_of(start..end)
    }
}

#[wasm_bindgen_test]
async fn mixed_methods() {
    let (client, _server) = common::connect::<MixedService<_>, MixedClient>(MixedImpl).await;
    assert_eq!(client.add(10, 20).await, 30);
    let items: Vec<u32> = client.stream_range(3, 7).collect().await;
    assert_eq!(items, vec![3, 4, 5, 6]);
}

#[web_rpc::service]
pub trait SlowStream {
    async fn slow_count(&self, target: u32, interval_ms: u32) -> impl Stream<Item = u32>;
}

struct SlowStreamImpl {
    produced: Rc<RefCell<u32>>,
}

impl SlowStream for SlowStreamImpl {
    async fn slow_count(&self, target: u32, interval_ms: u32) -> impl Stream<Item = u32> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        let produced = self.produced.clone();
        let interval = Duration::from_millis(interval_ms as u64);
        wasm_bindgen_futures::spawn_local(async move {
            for index in 0..target {
                gloo_timers::future::sleep(interval).await;
                *produced.borrow_mut() += 1;
                if tx.unbounded_send(index).is_err() {
                    break;
                }
            }
        });
        rx
    }
}

#[wasm_bindgen_test]
async fn abort_via_drop() {
    let produced = Rc::new(RefCell::new(0u32));
    let service = SlowStreamImpl {
        produced: produced.clone(),
    };
    let (client, _server) =
        common::connect::<SlowStreamService<_>, SlowStreamClient>(service).await;
    let mut stream = client.slow_count(100, 50);
    assert_eq!(stream.next().await, Some(0));
    assert_eq!(stream.next().await, Some(1));
    std::mem::drop(stream);
    gloo_timers::future::sleep(Duration::from_millis(300)).await;
    assert!(
        *produced.borrow() < 10,
        "server produced {} items",
        produced.borrow()
    );
}

#[wasm_bindgen_test]
async fn close_and_drain() {
    let produced = Rc::new(RefCell::new(0u32));
    let service = SlowStreamImpl {
        produced: produced.clone(),
    };
    let (client, _server) =
        common::connect::<SlowStreamService<_>, SlowStreamClient>(service).await;
    let mut stream = client.slow_count(100, 50);
    assert_eq!(stream.next().await, Some(0));
    assert_eq!(stream.next().await, Some(1));
    stream.close();
    // `collect` returning is the assertion: the stream ends after the abort.
    let _remaining: Vec<u32> = stream.collect().await;
}

#[web_rpc::service]
pub trait BorrowedStream {
    fn stream_prefixed(&self, prefix: &str) -> impl Stream<Item = String>;
}

struct BorrowedStreamImpl;
impl BorrowedStream for BorrowedStreamImpl {
    fn stream_prefixed(&self, prefix: &str) -> impl Stream<Item = String> {
        stream_of(
            (0..3)
                .map(|index| format!("{prefix}-{index}"))
                .collect::<Vec<_>>(),
        )
    }
}

#[wasm_bindgen_test]
async fn streaming_with_borrowed_args() {
    let (client, _server) =
        common::connect::<BorrowedStreamService<_>, BorrowedStreamClient>(BorrowedStreamImpl).await;
    let items: Vec<String> = client.stream_prefixed("hello").collect().await;
    assert_eq!(items, vec!["hello-0", "hello-1", "hello-2"]);
}

#[web_rpc::service]
pub trait PostStream {
    fn stream_js_strings(&self, count: u32) -> impl Stream<Item = Post<js_sys::JsString>>;
}

struct PostStreamImpl;
impl PostStream for PostStreamImpl {
    fn stream_js_strings(&self, count: u32) -> impl Stream<Item = Post<js_sys::JsString>> {
        stream_of((0..count).map(|index| Post(js_sys::JsString::from(format!("item-{index}")))))
    }
}

#[wasm_bindgen_test]
async fn streaming_post_return() {
    let (client, _server) =
        common::connect::<PostStreamService<_>, PostStreamClient>(PostStreamImpl).await;
    let items: Vec<Post<js_sys::JsString>> = client.stream_js_strings(3).collect().await;
    assert_eq!(items.len(), 3);
    assert_eq!(*items[0], "item-0");
    assert_eq!(*items[1], "item-1");
    assert_eq!(*items[2], "item-2");
}
