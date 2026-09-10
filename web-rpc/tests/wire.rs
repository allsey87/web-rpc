//! The byte strings in `WIRE.md`, asserted against the types they were derived from.
//!
//! A Javascript implementation of the protocol is written against `WIRE.md`, so these fixtures
//! are what stop the document and the Rust enums from drifting apart.

use postcard_schema::Schema;
use serde::{Deserialize, Serialize};
use wasm_bindgen_test::*;
use web_rpc::{codec::WireArg, wrap::Post, MessageHeader};

fn bytes<T: Serialize>(value: &T) -> Vec<u8> {
    postcard::to_allocvec(value).unwrap()
}

#[wasm_bindgen_test]
fn message_headers() {
    assert_eq!(bytes(&MessageHeader::Request(0)), [0x00, 0x00]);
    assert_eq!(bytes(&MessageHeader::Abort(1)), [0x01, 0x01]);
    assert_eq!(bytes(&MessageHeader::Response(3)), [0x02, 0x03]);
    // A sequence number is a varint, so it grows a byte at a time.
    assert_eq!(bytes(&MessageHeader::StreamItem(300)), [0x03, 0xac, 0x02]);
    assert_eq!(bytes(&MessageHeader::StreamEnd(127)), [0x04, 0x7f]);
    assert_eq!(bytes(&MessageHeader::StreamEnd(128)), [0x04, 0x80, 0x01]);
}

#[wasm_bindgen_test]
fn wire_arguments() {
    assert_eq!(bytes(&WireArg::Js), [0x00]);
    assert_eq!(bytes(&WireArg::Bytes(vec![1, 2, 3])), [0x01, 0x03, 1, 2, 3]);
    assert_eq!(bytes(&WireArg::None), [0x02]);
    assert_eq!(bytes(&WireArg::Some(Box::new(WireArg::Js))), [0x03, 0x00]);
    // `Result<Option<Post<_>>, E>` round-trips as Ok(Some(Js)) or Err(Bytes(..)).
    assert_eq!(
        bytes(&WireArg::Ok(Box::new(WireArg::Some(Box::new(WireArg::Js))))),
        [0x04, 0x03, 0x00]
    );
    assert_eq!(
        bytes(&WireArg::Err(Box::new(WireArg::Bytes(vec![7])))),
        [0x05, 0x01, 0x01, 7]
    );
}

#[derive(Serialize, Deserialize, Schema, Debug, PartialEq, Eq)]
pub struct Pair {
    pub left: u32,
    pub right: String,
}

#[web_rpc::service]
pub trait Fixture {
    fn first(&self, value: u32) -> u32;
    fn second(&self, pair: Pair, tag: Post<js_sys::JsString>) -> bool;
    fn third(&self, text: &str);
}

#[wasm_bindgen_test]
fn request_payloads() {
    // varint(method index) then one WireArg per argument. A `u32` argument is postcard bytes
    // nested inside the WireArg's own length prefix.
    assert_eq!(
        bytes(&FixtureRequest::First {
            value: WireArg::Bytes(bytes(&1u32)),
        }),
        [0x00, 0x01, 0x01, 0x01]
    );
    // The second method: index 1, a postcard payload, then a Javascript slot.
    assert_eq!(
        bytes(&FixtureRequest::Second {
            pair: WireArg::Bytes(bytes(&Pair {
                left: 2,
                right: "hi".into()
            })),
            tag: WireArg::Js,
        }),
        [0x01, 0x01, 0x04, 0x02, 0x02, b'h', b'i', 0x00]
    );
    // `&str` keeps serde's borrowing path, so it is written inline rather than as a WireArg.
    assert_eq!(
        bytes(&FixtureRequest::Third { text: "hi" }),
        [0x02, 0x02, b'h', b'i']
    );
}

#[wasm_bindgen_test]
fn response_payloads() {
    assert_eq!(
        bytes(&FixtureResponse::First(WireArg::Bytes(bytes(&42u32)))),
        [0x00, 0x01, 0x01, 42]
    );
    assert_eq!(
        bytes(&FixtureResponse::Second(WireArg::Bytes(bytes(&true)))),
        [0x01, 0x01, 0x01, 0x01]
    );
}

#[wasm_bindgen_test]
fn postcard_primitives() {
    assert_eq!(bytes(&true), [0x01]);
    assert_eq!(bytes(&false), [0x00]);
    assert_eq!(bytes(&-1i8), [0xff]);
    assert_eq!(bytes(&300u32), [0xac, 0x02]);
    // Signed integers zigzag before the varint.
    assert_eq!(bytes(&-1i32), [0x01]);
    assert_eq!(bytes(&1i32), [0x02]);
    assert_eq!(bytes(&-150i32), [0xab, 0x02]);
    assert_eq!(bytes(&1.0f32), [0x00, 0x00, 0x80, 0x3f]);
    assert_eq!(bytes(&"hi"), [0x02, b'h', b'i']);
    assert_eq!(bytes(&'a'), [0x01, b'a']);
    assert_eq!(bytes(&Option::<u32>::None), [0x00]);
    assert_eq!(bytes(&Some(1u32)), [0x01, 0x01]);
    assert_eq!(bytes(&vec![1u32, 2]), [0x02, 0x01, 0x02]);
    // Structs and tuples are their fields back to back, with no framing.
    assert_eq!(
        bytes(&Pair {
            left: 1,
            right: "x".into()
        }),
        [0x01, 0x01, b'x']
    );
    assert_eq!(bytes(&(1u32, 2u32)), [0x01, 0x02]);
    assert_eq!(bytes(&()), [] as [u8; 0]);
    // serde has no `usize`, so it encodes exactly as a `u64` does.
    assert_eq!(bytes(&(u32::MAX as usize)), bytes(&(u64::from(u32::MAX))));
    assert_eq!(bytes(&(-1isize)), bytes(&(-1i64)));
    // A sequence number is a u32, so a header is at most six bytes.
    assert_eq!(
        bytes(&MessageHeader::Request(u32::MAX)),
        [0x00, 0xff, 0xff, 0xff, 0xff, 0x0f]
    );
}
