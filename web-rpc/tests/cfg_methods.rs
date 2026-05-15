#[cfg(feature = "extra")]
use futures_core::Stream;
use futures_util::FutureExt;
#[cfg(feature = "extra")]
use futures_util::StreamExt;
use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait Demo {
    fn always_on(&self, x: u32) -> u32;

    #[cfg(feature = "extra")]
    fn extra(&self, s: &str) -> String;

    #[cfg(feature = "extra")]
    fn extra_stream(&self, n: u32) -> impl Stream<Item = u32>;
}

struct DemoImpl;

impl Demo for DemoImpl {
    fn always_on(&self, x: u32) -> u32 {
        x + 1
    }

    #[cfg(feature = "extra")]
    fn extra(&self, s: &str) -> String {
        format!("got {s}")
    }

    #[cfg(feature = "extra")]
    fn extra_stream(&self, n: u32) -> impl Stream<Item = u32> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        for i in 0..n {
            let _ = tx.unbounded_send(i);
        }
        rx
    }
}

async fn build() -> (DemoClient, futures_util::future::RemoteHandle<()>) {
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, handle) = web_rpc::Builder::new(server_interface)
        .with_service::<DemoService<_>>(DemoImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<DemoClient>()
        .build();
    (client, handle)
}

#[wasm_bindgen_test]
async fn always_on_method_works() {
    console_error_panic_hook::set_once();
    let (client, _handle) = build().await;
    assert_eq!(client.always_on(41).await, 42);
}

#[cfg(feature = "extra")]
#[wasm_bindgen_test]
async fn gated_method_works_when_feature_on() {
    console_error_panic_hook::set_once();
    let (client, _handle) = build().await;
    assert_eq!(client.extra("hi").await, "got hi");
}

#[cfg(feature = "extra")]
#[wasm_bindgen_test]
async fn gated_streaming_method_works_when_feature_on() {
    console_error_panic_hook::set_once();
    let (client, _handle) = build().await;
    let items: Vec<u32> = client.extra_stream(3).collect().await;
    assert_eq!(items, vec![0, 1, 2]);
}

// Method-level cfg combined with #[transfer].

#[web_rpc::service]
pub trait Uploader {
    fn ping(&self) -> u32;

    #[cfg(feature = "extra")]
    #[transfer(buffer)]
    fn upload(&self, buffer: js_sys::ArrayBuffer) -> u32;
}

struct UploaderImpl;

impl Uploader for UploaderImpl {
    fn ping(&self) -> u32 {
        7
    }

    #[cfg(feature = "extra")]
    fn upload(&self, buffer: js_sys::ArrayBuffer) -> u32 {
        buffer.byte_length()
    }
}

async fn build_uploader() -> (UploaderClient, futures_util::future::RemoteHandle<()>) {
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, handle) = web_rpc::Builder::new(server_interface)
        .with_service::<UploaderService<_>>(UploaderImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<UploaderClient>()
        .build();
    (client, handle)
}

#[wasm_bindgen_test]
async fn uploader_ping_works() {
    console_error_panic_hook::set_once();
    let (client, _handle) = build_uploader().await;
    assert_eq!(client.ping().await, 7);
}

#[cfg(feature = "extra")]
#[wasm_bindgen_test]
async fn gated_transfer_param_works() {
    console_error_panic_hook::set_once();
    let (client, _handle) = build_uploader().await;
    let buffer = js_sys::ArrayBuffer::new(16);
    assert_eq!(buffer.byte_length(), 16);
    assert_eq!(client.upload(buffer.clone()).await, 16);
    // Detached on the sender after transfer.
    assert_eq!(buffer.byte_length(), 0);
}

