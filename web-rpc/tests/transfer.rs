//! `Transfer<T>` as an argument, inside `Result`, and inside `Option` as a stream item.
//! Transfer is observable from the sending side: the buffer is detached, so its byte length
//! drops to zero once the message has been posted.

mod common;

use futures_core::Stream;
use futures_util::StreamExt;
use postcard_schema::Schema;
use serde::{Deserialize, Serialize};
use wasm_bindgen_test::*;
use web_rpc::wrap::Transfer;

#[derive(Serialize, Deserialize, Schema, Debug, PartialEq, Eq)]
pub struct RenderError(String);

fn buffer(length: u32) -> js_sys::ArrayBuffer {
    js_sys::Uint8Array::new_with_length(length).buffer()
}

#[web_rpc::service]
pub trait Buffers {
    fn upload(&self, data: Transfer<js_sys::ArrayBuffer>) -> u32;
    fn render(&self, succeed: bool) -> Result<Transfer<js_sys::ArrayBuffer>, RenderError>;
    fn chunks(&self, count: u32) -> impl Stream<Item = Option<Transfer<js_sys::ArrayBuffer>>>;
}

struct BuffersImpl;
impl Buffers for BuffersImpl {
    fn upload(&self, data: Transfer<js_sys::ArrayBuffer>) -> u32 {
        data.byte_length()
    }
    fn render(&self, succeed: bool) -> Result<Transfer<js_sys::ArrayBuffer>, RenderError> {
        if succeed {
            Ok(Transfer(buffer(8)))
        } else {
            Err(RenderError("nope".into()))
        }
    }
    fn chunks(&self, count: u32) -> impl Stream<Item = Option<Transfer<js_sys::ArrayBuffer>>> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        for index in 0..count {
            let item = (index % 2 == 0).then(|| Transfer(buffer(index + 1)));
            let _ = tx.unbounded_send(item);
        }
        rx
    }
}

#[wasm_bindgen_test]
async fn transfer_argument() {
    let (client, _server) = common::connect::<BuffersService<_>, BuffersClient>(BuffersImpl).await;
    let data = buffer(16);
    assert_eq!(client.upload(Transfer(data.clone())).await, 16);
    assert_eq!(data.byte_length(), 0);
}

#[wasm_bindgen_test]
async fn transfer_inside_result() {
    let (client, _server) = common::connect::<BuffersService<_>, BuffersClient>(BuffersImpl).await;
    assert_eq!(client.render(true).await.unwrap().byte_length(), 8);
    assert_eq!(
        client.render(false).await.unwrap_err(),
        RenderError("nope".into())
    );
}

#[wasm_bindgen_test]
async fn transfer_inside_option_stream_item() {
    let (client, _server) = common::connect::<BuffersService<_>, BuffersClient>(BuffersImpl).await;
    let items: Vec<Option<Transfer<js_sys::ArrayBuffer>>> = client.chunks(4).collect().await;
    assert_eq!(items.len(), 4);
    assert_eq!(items[0].as_ref().unwrap().byte_length(), 1);
    assert!(items[1].is_none());
    assert_eq!(items[2].as_ref().unwrap().byte_length(), 3);
    assert!(items[3].is_none());
}
