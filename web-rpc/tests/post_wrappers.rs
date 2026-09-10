//! `Option` and `Result` around `Post` values, in argument, return and stream item position.

mod common;

use futures_core::Stream;
use futures_util::StreamExt;
use wasm_bindgen_test::*;
use web_rpc::wrap::Post;

#[web_rpc::service]
pub trait Wrapped {
    fn maybe_string(&self, return_some: bool) -> Option<Post<js_sys::JsString>>;
    fn try_string(&self, succeed: bool) -> Result<Post<js_sys::JsString>, Post<js_sys::Error>>;
    fn string_length(&self, value: Option<Post<js_sys::JsString>>) -> u32;
    fn stream_maybe(&self, count: u32) -> impl Stream<Item = Option<Post<js_sys::JsString>>>;
    fn stream_results(
        &self,
        count: u32,
    ) -> impl Stream<Item = Result<Post<js_sys::JsString>, Post<js_sys::Error>>>;
}

struct WrappedImpl;
impl Wrapped for WrappedImpl {
    fn maybe_string(&self, return_some: bool) -> Option<Post<js_sys::JsString>> {
        return_some.then(|| Post(js_sys::JsString::from("hello")))
    }
    fn try_string(&self, succeed: bool) -> Result<Post<js_sys::JsString>, Post<js_sys::Error>> {
        if succeed {
            Ok(Post(js_sys::JsString::from("success")))
        } else {
            Err(Post(js_sys::Error::new("something went wrong")))
        }
    }
    fn string_length(&self, value: Option<Post<js_sys::JsString>>) -> u32 {
        value.map(|string| string.length()).unwrap_or(0)
    }
    fn stream_maybe(&self, count: u32) -> impl Stream<Item = Option<Post<js_sys::JsString>>> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        for index in 0..count {
            let item =
                (index % 2 == 0).then(|| Post(js_sys::JsString::from(format!("item-{index}"))));
            let _ = tx.unbounded_send(item);
        }
        rx
    }
    fn stream_results(
        &self,
        count: u32,
    ) -> impl Stream<Item = Result<Post<js_sys::JsString>, Post<js_sys::Error>>> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        for index in 0..count {
            let item = if index % 2 == 0 {
                Ok(Post(js_sys::JsString::from(format!("ok-{index}"))))
            } else {
                Err(Post(js_sys::Error::new(&format!("err-{index}"))))
            };
            let _ = tx.unbounded_send(item);
        }
        rx
    }
}

#[wasm_bindgen_test]
async fn option_return() {
    let (client, _server) = common::connect::<WrappedService<_>, WrappedClient>(WrappedImpl).await;
    assert_eq!(
        client.maybe_string(true).await,
        Some(Post(js_sys::JsString::from("hello")))
    );
    assert_eq!(client.maybe_string(false).await, None);
}

#[wasm_bindgen_test]
async fn result_return() {
    let (client, _server) = common::connect::<WrappedService<_>, WrappedClient>(WrappedImpl).await;
    assert_eq!(
        *client.try_string(true).await.unwrap(),
        js_sys::JsString::from("success")
    );
    assert_eq!(
        client.try_string(false).await.unwrap_err().message(),
        "something went wrong"
    );
}

#[wasm_bindgen_test]
async fn option_argument() {
    let (client, _server) = common::connect::<WrappedService<_>, WrappedClient>(WrappedImpl).await;
    assert_eq!(
        client
            .string_length(Some(Post(js_sys::JsString::from("test"))))
            .await,
        4
    );
    assert_eq!(client.string_length(None).await, 0);
}

#[wasm_bindgen_test]
async fn option_stream_items() {
    let (client, _server) = common::connect::<WrappedService<_>, WrappedClient>(WrappedImpl).await;
    let items: Vec<Option<Post<js_sys::JsString>>> = client.stream_maybe(4).collect().await;
    assert_eq!(
        items,
        vec![
            Some(Post(js_sys::JsString::from("item-0"))),
            None,
            Some(Post(js_sys::JsString::from("item-2"))),
            None
        ]
    );
}

#[wasm_bindgen_test]
async fn result_stream_items() {
    let (client, _server) = common::connect::<WrappedService<_>, WrappedClient>(WrappedImpl).await;
    let items: Vec<Result<Post<js_sys::JsString>, Post<js_sys::Error>>> =
        client.stream_results(4).collect().await;
    assert_eq!(items.len(), 4);
    assert_eq!(**items[0].as_ref().unwrap(), js_sys::JsString::from("ok-0"));
    assert_eq!(items[1].as_ref().unwrap_err().message(), "err-1");
    assert_eq!(**items[2].as_ref().unwrap(), js_sys::JsString::from("ok-2"));
    assert_eq!(items[3].as_ref().unwrap_err().message(), "err-3");
}
