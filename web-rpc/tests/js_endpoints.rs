//! The generated Javascript endpoint against real Rust, in both directions, plus Javascript
//! talking to Javascript over a real `Worker`.
//!
//! The endpoints are rendered into this test binary and extracted from it by
//! `tests/js/check.py` before the tests run, so they are loaded here exactly as an embedder
//! would load them. The harness in `tests/js/harness.mjs` is trait-agnostic: the scenarios
//! live here. These tests need a browser: blob modules and workers do not exist under node.

mod common;

use std::{cell::RefCell, rc::Rc, time::Duration};

use futures_core::Stream;
use futures_util::{FutureExt, StreamExt};
use postcard_schema::Schema;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;
use web_rpc::wrap::{Post, Transfer};

wasm_bindgen_test_configure!(run_in_browser);

#[derive(Serialize, Deserialize, Schema, Debug, PartialEq, Eq)]
pub struct Stats {
    pub label: String,
    pub count: u32,
}

#[derive(Serialize, Deserialize, Schema, Debug, PartialEq, Eq)]
pub enum Fault {
    NotFound,
    Bad(String),
}

/// One trait, exercised from both sides. Between them the methods reach every `Desc` kind in
/// argument and return position, both wrapper types, notifications, streams, and a stripped
/// method whose successor takes its index.
#[web_rpc::service]
pub trait Sample {
    fn add(&self, left: u32, right: u32) -> u32;
    fn wide(&self, value: u64) -> u64;
    #[cfg(any())]
    fn stripped(&self, value: u32) -> u32;
    fn blob(&self, data: Vec<u8>) -> u32;
    fn stats(&self, label: String, count: u32) -> Stats;
    fn echo(&self, value: Post<js_sys::JsString>) -> Post<js_sys::JsString>;
    fn take(&self, buffer: Transfer<js_sys::ArrayBuffer>) -> u32;
    fn give(&self, size: u32) -> Transfer<js_sys::ArrayBuffer>;
    fn maybe(&self, present: bool) -> Option<Post<js_sys::JsString>>;
    fn fallible(&self, succeed: bool) -> Result<u32, Fault>;
    fn fallible_text(&self, succeed: bool) -> Result<u32, String>;
    fn note(&self, message: String);
    fn notes(&self) -> u32;
    fn counters(&self, count: u32) -> impl Stream<Item = u32>;
    async fn slow(&self, millis: u32) -> u32;
    fn borrowed(&self, text: &str, data: &[u8]) -> String;
}

web_rpc::js::endpoint!(client = SampleClient);
web_rpc::js::endpoint!(service = SampleService);

const CLIENT_MODULE: &str = "/tests/js/generated/sample_client.mjs";
const SERVICE_MODULE: &str = "/tests/js/generated/sample_service.mjs";

/// The Javascript implementation of `Sample`, as the source of an object literal. It counts
/// what happens in `log`, which the harness hands back for `probe`.
const HANDLERS: &str = r#"{
    add: (left, right) => left + right,
    wide: (value) => value * 2n,
    blob: (data) => data.reduce((total, byte) => total + byte, 0),
    stats: (label, count) => ({ label: label.toUpperCase(), count: count + 1 }),
    echo: (value) => value + "!",
    take: (buffer) => buffer.byteLength,
    give: (size) => new ArrayBuffer(size),
    maybe: (present) => (present ? "here" : undefined),
    // `Fault` is an enum, so the thrown value has to be one; an `Error` reduces to its
    // message, which only an `Err(String)` can take.
    fallible: (succeed) => {
        if (!succeed) throw { tag: "Bad", value: "nope" };
        return 7;
    },
    fallibleText: (succeed) => {
        if (!succeed) throw new Error("bang");
        return 9;
    },
    note: () => {
        log.notes = (log.notes ?? 0) + 1;
    },
    notes: () => log.notes ?? 0,
    counters: async function* (count) {
        try {
            for (let index = 0; index < count; index += 1) {
                yield index;
                // A macrotask: an inbound `Abort` is a message event, so a generator that
                // only awaits microtasks starves the loop and cannot be interrupted.
                await new Promise((resolve) => setTimeout(resolve, 0));
            }
            log.finished = (log.finished ?? 0) + 1;
        } finally {
            if (!log.finished) log.returned = (log.returned ?? 0) + 1;
        }
    },
    slow: async (millis) => {
        await new Promise((resolve) => setTimeout(resolve, millis));
        log.slowCompletions = (log.slowCompletions ?? 0) + 1;
        return millis;
    },
    borrowed: (text, data) => `${text}:${data.length}`,
}"#;

#[derive(Default)]
struct SampleImpl {
    notes: RefCell<u32>,
    slow_completions: RefCell<u32>,
}

impl Sample for SampleImpl {
    fn add(&self, left: u32, right: u32) -> u32 {
        left + right
    }
    fn wide(&self, value: u64) -> u64 {
        value * 2
    }
    #[cfg(any())]
    fn stripped(&self, value: u32) -> u32 {
        unreachable!()
    }
    fn blob(&self, data: Vec<u8>) -> u32 {
        data.iter().map(|byte| *byte as u32).sum()
    }
    fn stats(&self, label: String, count: u32) -> Stats {
        Stats {
            label: label.to_uppercase(),
            count: count + 1,
        }
    }
    fn echo(&self, value: Post<js_sys::JsString>) -> Post<js_sys::JsString> {
        Post(value.concat(&"!".into()))
    }
    fn take(&self, buffer: Transfer<js_sys::ArrayBuffer>) -> u32 {
        buffer.byte_length()
    }
    fn give(&self, size: u32) -> Transfer<js_sys::ArrayBuffer> {
        Transfer(js_sys::Uint8Array::new_with_length(size).buffer())
    }
    fn maybe(&self, present: bool) -> Option<Post<js_sys::JsString>> {
        present.then(|| Post(js_sys::JsString::from("here")))
    }
    fn fallible(&self, succeed: bool) -> Result<u32, Fault> {
        if succeed {
            Ok(7)
        } else {
            Err(Fault::Bad("nope".into()))
        }
    }
    fn fallible_text(&self, succeed: bool) -> Result<u32, String> {
        if succeed {
            Ok(9)
        } else {
            Err("bang".into())
        }
    }
    fn note(&self, _message: String) {
        *self.notes.borrow_mut() += 1;
    }
    fn notes(&self) -> u32 {
        *self.notes.borrow()
    }
    fn counters(&self, count: u32) -> impl Stream<Item = u32> {
        futures_util::stream::iter(0..count)
    }
    async fn slow(&self, millis: u32) -> u32 {
        gloo_timers::future::sleep(Duration::from_millis(millis as u64)).await;
        *self.slow_completions.borrow_mut() += 1;
        millis
    }
    fn borrowed(&self, text: &str, data: &[u8]) -> String {
        format!("{text}:{}", data.len())
    }
}

// ---------------------------------------------------------------------------
// Reaching the harness
// ---------------------------------------------------------------------------

#[wasm_bindgen(inline_js = "\
let harness = null; \
export async function invoke(name, args) { \
  if (!harness) { \
    const source = await (await fetch('/tests/js/harness.mjs')).text(); \
    const url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' })); \
    harness = await import(url); \
  } \
  return await harness[name](...args); \
}")]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn invoke(name: &str, args: js_sys::Array) -> Result<JsValue, JsValue>;
}

fn args<const COUNT: usize>(values: [JsValue; COUNT]) -> js_sys::Array {
    values.into_iter().collect()
}

async fn harness(name: &str, values: js_sys::Array) -> Result<JsValue, JsValue> {
    invoke(name, values).await
}

async fn open(
    module: &str,
    class: &str,
    endpoint: &JsValue,
    handlers: Option<&str>,
) -> Result<JsValue, JsValue> {
    let handlers = handlers.map_or(JsValue::UNDEFINED, JsValue::from_str);
    harness(
        "open",
        args([module.into(), class.into(), endpoint.clone(), handlers]),
    )
    .await
}

async fn call(handle: &JsValue, method: &str, values: js_sys::Array) -> Result<JsValue, JsValue> {
    harness("call", args([handle.clone(), method.into(), values.into()])).await
}

async fn subscribe(handle: &JsValue, method: &str, values: js_sys::Array) -> Vec<JsValue> {
    let items = harness(
        "subscribe",
        args([handle.clone(), method.into(), values.into()]),
    )
    .await
    .unwrap();
    js_sys::Array::from(&items).iter().collect()
}

async fn probe(handle: &JsValue) -> JsValue {
    harness("probe", args([handle.clone()])).await.unwrap()
}

fn field(object: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(object, &JsValue::from_str(key)).unwrap()
}

fn number(value: &JsValue) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("not a number: {value:?}"))
}

fn string(value: &JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| panic!("not a string: {value:?}"))
}

fn json(value: &JsValue) -> String {
    js_sys::JSON::stringify(value).unwrap().into()
}

// ---------------------------------------------------------------------------
// Rust serves, Javascript calls
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
async fn rust_serves_js_calls() {
    console_error_panic_hook::set_once();
    let channel = common::channel();
    let service = Rc::new(SampleImpl::default());
    // The Rust half cannot be awaited before the Javascript half exists: the handshake only
    // completes once both ends are listening.
    let (server, _handle) = web_rpc::Interface::new(channel.port1())
        .then({
            let service = service.clone();
            |interface| {
                web_rpc::Builder::new(interface)
                    .with_service::<SampleService<_>>(service)
                    .build()
            }
        })
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = open(CLIENT_MODULE, "SampleClient", &channel.port2().into(), None)
        .await
        .unwrap();

    assert_eq!(
        number(
            &call(&client, "add", args([41.into(), 1.into()]))
                .await
                .unwrap()
        ),
        42.0
    );
    let wide = call(
        &client,
        "wide",
        args([js_sys::BigInt::from(1u64 << 40).into()]),
    )
    .await
    .unwrap();
    assert_eq!(wide, JsValue::from(js_sys::BigInt::from(2u64 << 40)));
    let blob = js_sys::Uint8Array::from(&[1u8, 2, 3, 4, 5][..]);
    assert_eq!(
        number(&call(&client, "blob", args([blob.into()])).await.unwrap()),
        15.0
    );
    let stats = call(&client, "stats", args(["hello".into(), 3.into()]))
        .await
        .unwrap();
    assert_eq!(string(&field(&stats, "label")), "HELLO");
    assert_eq!(number(&field(&stats, "count")), 4.0);
    assert_eq!(
        string(&call(&client, "echo", args(["hi".into()])).await.unwrap()),
        "hi!"
    );

    let buffer = js_sys::ArrayBuffer::new(16);
    assert_eq!(
        number(
            &call(&client, "take", args([buffer.clone().into()]))
                .await
                .unwrap()
        ),
        16.0
    );
    // The buffer was transferred out of the calling context.
    assert_eq!(buffer.byte_length(), 0);
    let given: js_sys::ArrayBuffer = call(&client, "give", args([8.into()]))
        .await
        .unwrap()
        .into();
    assert_eq!(given.byte_length(), 8);

    assert_eq!(
        string(&call(&client, "maybe", args([true.into()])).await.unwrap()),
        "here"
    );
    assert!(call(&client, "maybe", args([false.into()]))
        .await
        .unwrap()
        .is_undefined());

    assert_eq!(
        number(
            &call(&client, "fallible", args([true.into()]))
                .await
                .unwrap()
        ),
        7.0
    );
    // A `Result` rejects with the decoded `Err`, not with an `Error`.
    let err = call(&client, "fallible", args([false.into()]))
        .await
        .unwrap_err();
    assert_eq!(json(&err), r#"{"tag":"Bad","value":"nope"}"#);
    let err = call(&client, "fallibleText", args([false.into()]))
        .await
        .unwrap_err();
    assert_eq!(string(&err), "bang");

    assert!(call(&client, "note", args(["noted".into()]))
        .await
        .unwrap()
        .is_undefined());
    assert_eq!(
        number(&call(&client, "notes", args([])).await.unwrap()),
        1.0
    );

    let items: Vec<f64> = subscribe(&client, "counters", args([5.into()]))
        .await
        .iter()
        .map(number)
        .collect();
    assert_eq!(items, vec![0.0, 1.0, 2.0, 3.0, 4.0]);

    let data = js_sys::Uint8Array::from(&[7u8, 8, 9][..]);
    let borrowed = call(&client, "borrowed", args(["ab".into(), data.into()]))
        .await
        .unwrap();
    assert_eq!(string(&borrowed), "ab:3");

    // `Request.abort()` rejects locally and stops the service.
    let aborted = harness(
        "abort",
        args([client.clone(), "slow".into(), args([5000.into()]).into()]),
    )
    .await
    .unwrap();
    assert_eq!(string(&aborted), "AbortError");
    gloo_timers::future::sleep(Duration::from_millis(150)).await;
    assert_eq!(*service.slow_completions.borrow(), 0);

    harness("close", args([client])).await.unwrap();
}

#[wasm_bindgen_test]
async fn every_handler_is_required() {
    console_error_panic_hook::set_once();
    let channel = common::channel();
    let error = open(
        SERVICE_MODULE,
        "SampleService",
        &channel.port2().into(),
        Some("{ add: () => 0 }"),
    )
    .await
    .unwrap_err();
    let error: js_sys::Error = error.into();
    assert_eq!(error.name(), "TypeError");
    let message = String::from(error.message());
    assert!(message.contains("missing handlers"), "{message}");
    // The one handler that was supplied is not in the list; every other method is.
    assert!(!message.contains("add"), "{message}");
    assert!(message.contains("wide"), "{message}");
    assert!(message.contains("counters"), "{message}");
}

// ---------------------------------------------------------------------------
// Javascript serves, Rust calls
// ---------------------------------------------------------------------------

async fn js_backed_client() -> (SampleClient, JsValue) {
    console_error_panic_hook::set_once();
    let channel = common::channel();
    // The Javascript endpoint starts polling as soon as it is constructed, so it can be built
    // first and the Rust handshake awaited after.
    let handle = open(
        SERVICE_MODULE,
        "SampleService",
        &channel.port2().into(),
        Some(HANDLERS),
    )
    .await
    .unwrap();
    let client = web_rpc::Builder::new(web_rpc::Interface::new(channel.port1()).await)
        .with_client::<SampleClient>()
        .build();
    (client, handle)
}

#[wasm_bindgen_test]
async fn js_serves_rust_calls() {
    let (client, handle) = js_backed_client().await;

    assert_eq!(client.add(41, 1).await, 42);
    assert_eq!(client.wide(1 << 40).await, 2 << 40);
    assert_eq!(client.blob(vec![1, 2, 3, 4, 5]).await, 15);
    assert_eq!(
        client.stats("hello".into(), 3).await,
        Stats {
            label: "HELLO".into(),
            count: 4
        }
    );
    assert_eq!(*client.echo(Post("hi".into())).await, "hi!");
    assert_eq!(
        client.maybe(true).await.map(|value| String::from(&*value)),
        Some("here".into())
    );
    assert_eq!(client.maybe(false).await, None);
    assert_eq!(client.fallible(true).await, Ok(7));
    // A handler that throws produces the `Err` variant.
    assert_eq!(client.fallible(false).await, Err(Fault::Bad("nope".into())));
    assert_eq!(client.fallible_text(true).await, Ok(9));
    assert_eq!(client.fallible_text(false).await, Err("bang".into()));

    let buffer = js_sys::Uint8Array::new_with_length(16).buffer();
    assert_eq!(client.take(Transfer(buffer.clone())).await, 16);
    assert_eq!(buffer.byte_length(), 0);
    assert_eq!(client.give(8).await.byte_length(), 8);

    client.note("noted".into());
    assert_eq!(client.notes().await, 1);

    let items: Vec<u32> = client.counters(5).collect().await;
    assert_eq!(items, vec![0, 1, 2, 3, 4]);
    assert_eq!(number(&field(&probe(&handle).await, "finished")), 1.0);

    assert_eq!(client.borrowed("ab", &[7, 8, 9]).await, "ab:3");

    assert_eq!(client.slow(20).await, 20);
}

#[wasm_bindgen_test]
async fn rust_dropping_a_receiver_returns_the_js_generator() {
    let (client, handle) = js_backed_client().await;
    let mut stream = client.counters(10_000);
    assert_eq!(stream.next().await, Some(0));
    std::mem::drop(stream);
    gloo_timers::future::sleep(Duration::from_millis(150)).await;
    assert_eq!(number(&field(&probe(&handle).await, "returned")), 1.0);
}

#[wasm_bindgen_test]
async fn rust_dropping_a_request_abandons_the_response() {
    let (client, handle) = js_backed_client().await;
    let mut pending = client.slow(80).fuse();
    let mut timeout = gloo_timers::future::sleep(Duration::from_millis(20)).fuse();
    futures_util::select! {
        _ = pending => panic!("the request completed"),
        _ = timeout => std::mem::drop(pending),
    };
    gloo_timers::future::sleep(Duration::from_millis(150)).await;
    // The handler ran to completion; the connection still works.
    assert_eq!(
        number(&field(&probe(&handle).await, "slowCompletions")),
        1.0
    );
    assert_eq!(client.add(1, 2).await, 3);
}

// ---------------------------------------------------------------------------
// Javascript to Javascript over a real Worker
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
async fn js_to_js_over_a_worker() {
    console_error_panic_hook::set_once();
    let client = harness(
        "openWorker",
        args([
            CLIENT_MODULE.into(),
            "SampleClient".into(),
            SERVICE_MODULE.into(),
            "SampleService".into(),
            HANDLERS.into(),
        ]),
    )
    .await
    .unwrap();

    assert_eq!(
        number(
            &call(&client, "add", args([41.into(), 1.into()]))
                .await
                .unwrap()
        ),
        42.0
    );
    assert_eq!(
        string(&call(&client, "echo", args(["hi".into()])).await.unwrap()),
        "hi!"
    );
    let items: Vec<f64> = subscribe(&client, "counters", args([3.into()]))
        .await
        .iter()
        .map(number)
        .collect();
    assert_eq!(items, vec![0.0, 1.0, 2.0]);

    // Closing rejects what is still pending.
    let rejection = harness(
        "callThenClose",
        args([
            client.clone(),
            "add".into(),
            args([1.into(), 1.into()]).into(),
        ]),
    )
    .await
    .unwrap();
    assert_eq!(string(&rejection), "AbortError");
    harness("terminate", args([client])).await.unwrap();
}

#[wasm_bindgen_test]
async fn a_worker_that_fails_to_start_rejects_rather_than_hangs() {
    console_error_panic_hook::set_once();
    let client = harness(
        "openWorker",
        args([
            CLIENT_MODULE.into(),
            "SampleClient".into(),
            JsValue::NULL,
            JsValue::NULL,
            JsValue::NULL,
        ]),
    )
    .await
    .unwrap();
    let mut pending = std::pin::pin!(call(&client, "add", args([1.into(), 2.into()])).fuse());
    let mut timeout = gloo_timers::future::sleep(Duration::from_millis(2000)).fuse();
    futures_util::select! {
        outcome = pending => assert!(outcome.is_err()),
        _ = timeout => panic!("the request neither resolved nor rejected"),
    };
    harness("close", args([client.clone()])).await.unwrap();
    harness("terminate", args([client.clone()])).await.unwrap();
}
