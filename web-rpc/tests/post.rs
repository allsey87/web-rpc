mod common;

use wasm_bindgen_test::*;
use web_rpc::wrap::Post;

#[web_rpc::service]
pub trait Concat {
    fn concat_with_space(
        &self,
        left: Post<js_sys::JsString>,
        right: Post<js_sys::JsString>,
    ) -> Post<js_sys::JsString>;
}

struct ConcatImpl;
impl Concat for ConcatImpl {
    fn concat_with_space(
        &self,
        left: Post<js_sys::JsString>,
        right: Post<js_sys::JsString>,
    ) -> Post<js_sys::JsString> {
        Post(js_sys::Array::of2(&left, &right).join(" "))
    }
}

#[wasm_bindgen_test]
async fn post() {
    let (client, _server) = common::connect::<ConcatService<_>, ConcatClient>(ConcatImpl).await;
    let response = client
        .concat_with_space(Post("hello".into()), Post("world".into()))
        .await;
    assert_eq!(*response, "hello world");
}
