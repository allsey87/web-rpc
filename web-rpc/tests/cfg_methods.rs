//! A `#[cfg]` on a trait method reaches the request enum, the response enum, the client, the
//! server and the description in lockstep. `cfg(any())` is always false and `cfg(all())`
//! always true, so one build covers a stripped method and a kept one, and the method after
//! the stripped one takes its index on the wire.

#![allow(clippy::non_minimal_cfg)]

mod common;

use futures_core::Stream;
use futures_util::StreamExt;
use wasm_bindgen_test::*;
use web_rpc::wrap::Transfer;

#[web_rpc::service]
pub trait Gated {
    fn first(&self, value: u32) -> u32;

    #[cfg(any())]
    fn stripped(&self, text: &str) -> String;

    #[cfg(all())]
    fn kept(&self, text: &str) -> String;

    #[cfg(any())]
    fn stripped_stream(&self, count: u32) -> impl Stream<Item = u32>;

    #[cfg(all())]
    fn kept_stream(&self, count: u32) -> impl Stream<Item = u32>;

    #[cfg(any())]
    fn stripped_upload(&self, buffer: Transfer<js_sys::ArrayBuffer>) -> u32;

    #[cfg(all())]
    fn kept_upload(&self, buffer: Transfer<js_sys::ArrayBuffer>) -> u32;

    fn last(&self, value: u32) -> u32;
}

struct GatedImpl;
impl Gated for GatedImpl {
    fn first(&self, value: u32) -> u32 {
        value + 1
    }
    #[cfg(any())]
    fn stripped(&self, text: &str) -> String {
        unreachable!()
    }
    #[cfg(all())]
    fn kept(&self, text: &str) -> String {
        format!("got {text}")
    }
    #[cfg(any())]
    fn stripped_stream(&self, count: u32) -> impl Stream<Item = u32> {
        futures_util::stream::empty()
    }
    #[cfg(all())]
    fn kept_stream(&self, count: u32) -> impl Stream<Item = u32> {
        futures_util::stream::iter(0..count)
    }
    #[cfg(any())]
    fn stripped_upload(&self, buffer: Transfer<js_sys::ArrayBuffer>) -> u32 {
        unreachable!()
    }
    #[cfg(all())]
    fn kept_upload(&self, buffer: Transfer<js_sys::ArrayBuffer>) -> u32 {
        buffer.byte_length()
    }
    fn last(&self, value: u32) -> u32 {
        value + 2
    }
}

#[wasm_bindgen_test]
async fn methods_around_a_stripped_one_still_agree() {
    let (client, _server) = common::connect::<GatedService<_>, GatedClient>(GatedImpl).await;
    assert_eq!(client.first(41).await, 42);
    assert_eq!(client.kept("hi").await, "got hi");
    let items: Vec<u32> = client.kept_stream(3).collect().await;
    assert_eq!(items, vec![0, 1, 2]);
    let buffer = js_sys::ArrayBuffer::new(16);
    assert_eq!(client.kept_upload(Transfer(buffer.clone())).await, 16);
    assert_eq!(buffer.byte_length(), 0);
    assert_eq!(client.last(40).await, 42);
}

#[wasm_bindgen_test]
fn the_description_skips_stripped_methods() {
    let names: Vec<&str> = GATED_DESCRIPTION
        .methods
        .iter()
        .flat_map(|group| group.iter().map(|method| method.name))
        .collect();
    assert_eq!(names, ["first", "kept", "keptStream", "keptUpload", "last"]);
}
