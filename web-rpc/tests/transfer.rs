//! Tests for the new `#[transfer(...)]` attribute forms:
//!  - bare param transfer (`canvas`)
//!  - param-expression transfer (`data => data.buffer()`)
//!  - return closure with refutable pattern (`return => |Ok(o)| o.buffer()`)
//!  - return match-block with multiple arms

use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
use wasm_bindgen_test::*;

/// Returns the byteLength of the underlying `ArrayBuffer` of a `Uint8Array` —
/// 0 if the buffer has been detached (transferred).
fn buffer_byte_length(arr: &js_sys::Uint8Array) -> u32 {
    arr.buffer().byte_length()
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct RenderError(String);

// --- Param-expression transfer: `data => data.buffer()` ---

#[web_rpc::service]
pub trait Upload {
    #[transfer(data => data.buffer())]
    fn upload(&self, data: js_sys::Uint8Array) -> u32;
}

struct UploadImpl;
impl Upload for UploadImpl {
    fn upload(&self, data: js_sys::Uint8Array) -> u32 {
        data.length()
    }
}

#[wasm_bindgen_test]
async fn transfer_param_expr() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<UploadService<_>>(UploadImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<UploadClient>()
        .build();

    let data = js_sys::Uint8Array::new_with_length(16);
    data.fill(0xAB, 0, 16);
    assert_eq!(buffer_byte_length(&data), 16);

    let len = client.upload(data.clone()).await;
    assert_eq!(len, 16);
    // After transfer the original buffer is detached on the sending side.
    assert_eq!(buffer_byte_length(&data), 0);
}

// --- Return closure with refutable pattern: `return => |Ok(o)| o.buffer()` ---

#[web_rpc::service]
pub trait Render {
    #[transfer(return => |Ok(buf)| buf.buffer())]
    fn render(&self, succeed: bool) -> Result<js_sys::Uint8Array, RenderError>;
}

struct RenderImpl;
impl Render for RenderImpl {
    fn render(&self, succeed: bool) -> Result<js_sys::Uint8Array, RenderError> {
        if succeed {
            let arr = js_sys::Uint8Array::new_with_length(8);
            arr.fill(7, 0, 8);
            Ok(arr)
        } else {
            Err(RenderError("nope".into()))
        }
    }
}

#[wasm_bindgen_test]
async fn transfer_return_ok_only() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<RenderService<_>>(RenderImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<RenderClient>()
        .build();

    let r = client.render(true).await.unwrap();
    assert_eq!(r.length(), 8);
    let r_err = client.render(false).await;
    assert_eq!(r_err.unwrap_err(), RenderError("nope".into()));
}

// --- Return match-block: handle multiple variants with one clause ---

#[web_rpc::service]
pub trait RenderBoth {
    #[transfer(return => match {
        Ok(buf) => buf.buffer(),
    })]
    fn render(&self) -> Result<js_sys::Uint8Array, RenderError>;
}

struct RenderBothImpl;
impl RenderBoth for RenderBothImpl {
    fn render(&self) -> Result<js_sys::Uint8Array, RenderError> {
        let arr = js_sys::Uint8Array::new_with_length(4);
        Ok(arr)
    }
}

#[wasm_bindgen_test]
async fn transfer_return_match_block() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<RenderBothService<_>>(RenderBothImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<RenderBothClient>()
        .build();

    let r = client.render().await.unwrap();
    assert_eq!(r.length(), 4);
}
