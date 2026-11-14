//! `Transfer<T>` as an argument, inside `Result`, inside `Option` as a stream item, and
//! inside `Vec`, arrays and maps, alone and nested.
//! Transfer is observable from the sending side: the buffer is detached, so its byte length
//! drops to zero once the message has been posted.

mod common;

use std::collections::{BTreeMap, HashMap};

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
    fn lengths(&self, buffers: Vec<Transfer<js_sys::ArrayBuffer>>) -> Vec<u32>;
    fn pair(&self, sizes: [u32; 2]) -> [Transfer<js_sys::ArrayBuffer>; 2];
    fn named(
        &self,
        buffers: HashMap<String, Transfer<js_sys::ArrayBuffer>>,
    ) -> BTreeMap<String, u32>;
    fn allocate(&self, sizes: Option<Vec<u32>>) -> Option<Vec<Transfer<js_sys::ArrayBuffer>>>;
    fn sparse(&self, sizes: Vec<u32>) -> Vec<Option<Transfer<js_sys::ArrayBuffer>>>;
    fn grouped(
        &self,
        sizes: Vec<u32>,
    ) -> Result<BTreeMap<u32, Vec<Transfer<js_sys::ArrayBuffer>>>, RenderError>;
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
    fn lengths(&self, buffers: Vec<Transfer<js_sys::ArrayBuffer>>) -> Vec<u32> {
        buffers.iter().map(|buffer| buffer.byte_length()).collect()
    }
    fn pair(&self, sizes: [u32; 2]) -> [Transfer<js_sys::ArrayBuffer>; 2] {
        sizes.map(|size| Transfer(buffer(size)))
    }
    fn named(
        &self,
        buffers: HashMap<String, Transfer<js_sys::ArrayBuffer>>,
    ) -> BTreeMap<String, u32> {
        buffers
            .into_iter()
            .map(|(name, buffer)| (name, buffer.byte_length()))
            .collect()
    }
    fn allocate(&self, sizes: Option<Vec<u32>>) -> Option<Vec<Transfer<js_sys::ArrayBuffer>>> {
        sizes.map(|sizes| {
            sizes
                .into_iter()
                .map(|size| Transfer(buffer(size)))
                .collect()
        })
    }
    fn sparse(&self, sizes: Vec<u32>) -> Vec<Option<Transfer<js_sys::ArrayBuffer>>> {
        sizes
            .into_iter()
            .map(|size| (size > 0).then(|| Transfer(buffer(size))))
            .collect()
    }
    fn grouped(
        &self,
        sizes: Vec<u32>,
    ) -> Result<BTreeMap<u32, Vec<Transfer<js_sys::ArrayBuffer>>>, RenderError> {
        if sizes.is_empty() {
            return Err(RenderError("empty".into()));
        }
        let mut groups = BTreeMap::<u32, Vec<_>>::new();
        for size in sizes {
            groups
                .entry(size % 2)
                .or_default()
                .push(Transfer(buffer(size)));
        }
        Ok(groups)
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

fn lengths(buffers: &[Transfer<js_sys::ArrayBuffer>]) -> Vec<u32> {
    buffers.iter().map(|buffer| buffer.byte_length()).collect()
}

#[wasm_bindgen_test]
async fn transfer_inside_vec() {
    let (client, _server) = common::connect::<BuffersService<_>, BuffersClient>(BuffersImpl).await;
    let sent = [buffer(1), buffer(2), buffer(3)];
    let buffers = sent.iter().cloned().map(Transfer).collect();
    assert_eq!(client.lengths(buffers).await, vec![1, 2, 3]);
    assert!(sent.iter().all(|buffer| buffer.byte_length() == 0));
    assert_eq!(client.lengths(Vec::new()).await, Vec::<u32>::new());
}

#[wasm_bindgen_test]
async fn transfer_inside_array() {
    let (client, _server) = common::connect::<BuffersService<_>, BuffersClient>(BuffersImpl).await;
    assert_eq!(lengths(&client.pair([5, 9]).await), vec![5, 9]);
}

#[wasm_bindgen_test]
async fn transfer_inside_map() {
    let (client, _server) = common::connect::<BuffersService<_>, BuffersClient>(BuffersImpl).await;
    let buffers = HashMap::from([
        ("small".to_string(), Transfer(buffer(2))),
        ("large".to_string(), Transfer(buffer(64))),
    ]);
    assert_eq!(
        client.named(buffers).await,
        BTreeMap::from([("large".to_string(), 64), ("small".to_string(), 2)])
    );
}

#[wasm_bindgen_test]
async fn transfer_inside_nested_containers() {
    let (client, _server) = common::connect::<BuffersService<_>, BuffersClient>(BuffersImpl).await;
    assert_eq!(client.allocate(None).await, None);
    assert_eq!(
        lengths(&client.allocate(Some(vec![4, 8])).await.unwrap()),
        vec![4, 8]
    );

    let sparse = client.sparse(vec![3, 0, 7]).await;
    assert_eq!(sparse.len(), 3);
    assert_eq!(sparse[0].as_ref().unwrap().byte_length(), 3);
    assert!(sparse[1].is_none());
    assert_eq!(sparse[2].as_ref().unwrap().byte_length(), 7);

    let groups = client.grouped(vec![1, 2, 3, 4]).await.unwrap();
    assert_eq!(lengths(&groups[&0]), vec![2, 4]);
    assert_eq!(lengths(&groups[&1]), vec![1, 3]);
    assert_eq!(
        client.grouped(Vec::new()).await.unwrap_err(),
        RenderError("empty".into())
    );
}
