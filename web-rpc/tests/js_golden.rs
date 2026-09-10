//! Fixture traits whose rendered endpoints are compared against `tests/js/expected/`.
//!
//! Between them the traits reach every rule of the Javascript conventions: each primitive,
//! every container shape, both wrapper types, `Option` and `Result` in every position,
//! borrowed arguments, notifications, streams, a trailing optional argument, a reserved word, and a `#[cfg]`
//! that strips a method so that the one after it takes its index. There are no test
//! functions here: building this file renders the endpoints into the test binary, and
//! `tests/js/check.py` extracts them with objcopy and diffs them.

use postcard_schema::Schema;
use serde::{Deserialize, Serialize};
use web_rpc::wrap::{Post, Transfer};

#[derive(Serialize, Deserialize, Schema)]
pub struct Point {
    pub x: i32,
    pub y: f64,
}

#[derive(Serialize, Deserialize, Schema)]
pub enum Colour {
    Red,
    Green,
}

#[derive(Serialize, Deserialize, Schema)]
pub enum Shape {
    Empty,
    Circle(u32),
    Segment(u32, u32),
    Label { text: String, at: Point },
}

#[derive(Serialize, Deserialize, Schema)]
pub struct Everything {
    pub flag: bool,
    pub small: u8,
    pub signed_small: i8,
    pub short: u16,
    pub signed_short: i16,
    pub wide: u64,
    pub signed: i64,
    pub huge: u128,
    pub signed_huge: i128,
    pub ratio: f32,
    pub letter: char,
    pub name: String,
    pub blob: Vec<u8>,
    pub points: Vec<Point>,
    pub pair: (u32, String),
    pub lookup: std::collections::BTreeMap<String, u32>,
    pub maybe: Option<Colour>,
    pub outcome: Result<Point, String>,
    pub nothing: (),
}

/// The trait a Javascript endpoint calls.
#[web_rpc::service]
pub trait Calculator {
    fn add(&self, left: u32, right: u32) -> u32;
    fn note(&self, message: String);
    fn describe(&self, shape: Shape) -> Everything;
    fn echo(&self, value: Post<js_sys::JsString>) -> Post<js_sys::JsString>;
    fn upload(&self, buffer: Transfer<js_sys::ArrayBuffer>) -> Result<u32, String>;
    fn lookup(&self, key: u32) -> Result<Option<Post<js_sys::JsString>>, Colour>;
    fn counters(&self, count: u32) -> impl futures_core::Stream<Item = u32>;
    #[cfg(any())]
    fn stripped(&self, value: u32) -> u32;
    fn tail(&self, delete: u32, extra: Option<u32>) -> u32;
    fn nothing(&self);
    fn borrowed(&self, text: &str, data: &[u8]) -> u32;
}

/// The trait a Javascript endpoint implements.
#[web_rpc::service]
pub trait Display {
    fn ping(&self) -> u32;
    fn colours(&self, count: u32) -> impl futures_core::Stream<Item = Colour>;
    fn delete(&self, point: Point) -> Result<Point, String>;
}

/// A trait for an endpoint that only calls.
#[web_rpc::service]
pub trait Clock {
    fn now(&self) -> u64;
}

/// A trait for an endpoint that only serves.
#[web_rpc::service]
pub trait Logger {
    fn log(&self, line: String);
}

web_rpc::js::endpoint!(service = DisplayService, client = CalculatorClient);
web_rpc::js::endpoint!(client = ClockClient);
web_rpc::js::endpoint!(service = LoggerService);
