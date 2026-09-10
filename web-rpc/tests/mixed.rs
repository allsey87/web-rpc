//! `Result` and `Option` wrappers whose variants route differently: one a Javascript value,
//! the other postcard bytes.

mod common;

use futures_core::Stream;
use futures_util::StreamExt;
use postcard_schema::Schema;
use serde::{Deserialize, Serialize};
use wasm_bindgen_test::*;
use web_rpc::wrap::Post;

#[derive(Serialize, Deserialize, Schema, Debug, PartialEq, Eq)]
pub enum Fault {
    NotFound,
    InvalidInput(String),
}

#[web_rpc::service]
pub trait Lookup {
    fn try_string(&self, succeed: bool) -> Result<Post<js_sys::JsString>, Fault>;
    fn lookup(&self, key: u32) -> Result<Option<Post<js_sys::JsString>>, Fault>;
    fn stream_results(
        &self,
        count: u32,
    ) -> impl Stream<Item = Result<Post<js_sys::JsString>, Fault>>;
}

struct LookupImpl;
impl Lookup for LookupImpl {
    fn try_string(&self, succeed: bool) -> Result<Post<js_sys::JsString>, Fault> {
        if succeed {
            Ok(Post(js_sys::JsString::from("ok")))
        } else {
            Err(Fault::InvalidInput("bad".into()))
        }
    }
    fn lookup(&self, key: u32) -> Result<Option<Post<js_sys::JsString>>, Fault> {
        match key {
            0 => Err(Fault::NotFound),
            1 => Ok(None),
            _ => Ok(Some(Post(js_sys::JsString::from(format!("key-{key}"))))),
        }
    }
    fn stream_results(
        &self,
        count: u32,
    ) -> impl Stream<Item = Result<Post<js_sys::JsString>, Fault>> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        for index in 0..count {
            let item = if index % 2 == 0 {
                Ok(Post(js_sys::JsString::from(format!("ok-{index}"))))
            } else {
                Err(Fault::InvalidInput(format!("err-{index}")))
            };
            let _ = tx.unbounded_send(item);
        }
        rx
    }
}

#[wasm_bindgen_test]
async fn result_of_post_and_error() {
    let (client, _server) = common::connect::<LookupService<_>, LookupClient>(LookupImpl).await;
    assert_eq!(
        *client.try_string(true).await.unwrap(),
        js_sys::JsString::from("ok")
    );
    assert_eq!(
        client.try_string(false).await.unwrap_err(),
        Fault::InvalidInput("bad".into())
    );
}

#[wasm_bindgen_test]
async fn result_of_option_of_post() {
    let (client, _server) = common::connect::<LookupService<_>, LookupClient>(LookupImpl).await;
    assert_eq!(
        client.lookup(42).await.unwrap(),
        Some(Post(js_sys::JsString::from("key-42")))
    );
    assert_eq!(client.lookup(1).await.unwrap(), None);
    assert_eq!(client.lookup(0).await.unwrap_err(), Fault::NotFound);
}

#[wasm_bindgen_test]
async fn stream_of_results() {
    let (client, _server) = common::connect::<LookupService<_>, LookupClient>(LookupImpl).await;
    let items: Vec<Result<Post<js_sys::JsString>, Fault>> =
        client.stream_results(4).collect().await;
    assert_eq!(items.len(), 4);
    assert_eq!(**items[0].as_ref().unwrap(), js_sys::JsString::from("ok-0"));
    assert_eq!(
        items[1].as_ref().unwrap_err(),
        &Fault::InvalidInput("err-1".into())
    );
    assert_eq!(**items[2].as_ref().unwrap(), js_sys::JsString::from("ok-2"));
    assert_eq!(
        items[3].as_ref().unwrap_err(),
        &Fault::InvalidInput("err-3".into())
    );
}
