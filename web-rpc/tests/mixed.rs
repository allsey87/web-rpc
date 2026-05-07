//! Tests for routing cases that the old `#[post(...)]` macro couldn't express:
//!  - `Result<JsT, RustErr>` (mixed JS/serial variants)
//!  - `Result<Option<JsT>, RustErr>` (nested wrappers)
//!
//! Each variant routes through its own path automatically — no attribute required.

use futures_core::Stream;
use futures_util::{FutureExt, StreamExt};
use serde::{Deserialize, Serialize};
use wasm_bindgen_test::*;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub enum MyRustError {
    NotFound,
    InvalidInput(String),
}

// --- Result<JsT, RustErr> — Ok routes JS, Err routes serial ---

#[web_rpc::service]
pub trait MixedFallible {
    fn try_string(&self, succeed: bool) -> Result<js_sys::JsString, MyRustError>;
}

struct MixedFallibleImpl;
impl MixedFallible for MixedFallibleImpl {
    fn try_string(&self, succeed: bool) -> Result<js_sys::JsString, MyRustError> {
        if succeed {
            Ok(js_sys::JsString::from("ok"))
        } else {
            Err(MyRustError::InvalidInput("bad".into()))
        }
    }
}

#[wasm_bindgen_test]
async fn mixed_result_ok() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<MixedFallibleService<_>>(MixedFallibleImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<MixedFallibleClient>()
        .build();

    let result = client.try_string(true).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), js_sys::JsString::from("ok"));
}

#[wasm_bindgen_test]
async fn mixed_result_err() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<MixedFallibleService<_>>(MixedFallibleImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<MixedFallibleClient>()
        .build();

    let result = client.try_string(false).await;
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        MyRustError::InvalidInput("bad".into())
    );
}

// --- Nested wrapper: Result<Option<JsT>, RustErr> ---

#[web_rpc::service]
pub trait Nested {
    fn lookup(&self, key: u32) -> Result<Option<js_sys::JsString>, MyRustError>;
}

struct NestedImpl;
impl Nested for NestedImpl {
    fn lookup(&self, key: u32) -> Result<Option<js_sys::JsString>, MyRustError> {
        match key {
            0 => Err(MyRustError::NotFound),
            1 => Ok(None),
            _ => Ok(Some(js_sys::JsString::from(format!("key-{key}")))),
        }
    }
}

#[wasm_bindgen_test]
async fn nested_ok_some() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<NestedService<_>>(NestedImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<NestedClient>()
        .build();

    let r = client.lookup(42).await;
    assert_eq!(r.unwrap(), Some(js_sys::JsString::from("key-42")));
}

#[wasm_bindgen_test]
async fn nested_ok_none() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<NestedService<_>>(NestedImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<NestedClient>()
        .build();

    let r = client.lookup(1).await;
    assert_eq!(r.unwrap(), None);
}

#[wasm_bindgen_test]
async fn nested_err() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<NestedService<_>>(NestedImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<NestedClient>()
        .build();

    let r = client.lookup(0).await;
    assert_eq!(r.unwrap_err(), MyRustError::NotFound);
}

// --- Streaming Result<JsT, RustErr> ---

#[web_rpc::service]
pub trait StreamMixed {
    fn stream_results(
        &self,
        count: u32,
    ) -> impl Stream<Item = Result<js_sys::JsString, MyRustError>>;
}

struct StreamMixedImpl;
impl StreamMixed for StreamMixedImpl {
    fn stream_results(
        &self,
        count: u32,
    ) -> impl Stream<Item = Result<js_sys::JsString, MyRustError>> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        for i in 0..count {
            if i % 2 == 0 {
                let _ = tx.unbounded_send(Ok(js_sys::JsString::from(format!("ok-{i}"))));
            } else {
                let _ = tx.unbounded_send(Err(MyRustError::InvalidInput(format!("err-{i}"))));
            }
        }
        rx
    }
}

#[wasm_bindgen_test]
async fn stream_mixed() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<StreamMixedService<_>>(StreamMixedImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<StreamMixedClient>()
        .build();

    let items: Vec<Result<js_sys::JsString, MyRustError>> =
        client.stream_results(4).collect().await;
    assert_eq!(items.len(), 4);
    assert_eq!(items[0].as_ref().unwrap(), &js_sys::JsString::from("ok-0"));
    assert_eq!(
        items[1].as_ref().unwrap_err(),
        &MyRustError::InvalidInput("err-1".into())
    );
    assert_eq!(items[2].as_ref().unwrap(), &js_sys::JsString::from("ok-2"));
    assert_eq!(
        items[3].as_ref().unwrap_err(),
        &MyRustError::InvalidInput("err-3".into())
    );
}
