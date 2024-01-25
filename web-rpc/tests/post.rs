use futures_util::FutureExt;
use wasm_bindgen_test::*;

#[web_rpc::service]
pub trait Service {
    #[post(left, right, return)]
    fn concat_with_space(
        left: js_sys::JsString,
        right: js_sys::JsString
    ) -> js_sys::JsString;
}
struct ServiceServerImpl;
impl Service for ServiceServerImpl {
    fn concat_with_space(
        &self,
        left: js_sys::JsString,
        right: js_sys::JsString
    ) -> js_sys::JsString {
        js_sys::Array::of2(&left, &right).join(" ")
    }
}

#[wasm_bindgen_test]
async fn post() {
    console_error_panic_hook::set_once();
    /* create channel */
    let channel = web_sys::MessageChannel::new().unwrap();
    /* create and spawn server (shuts down when _server_handle is dropped) */
    let (server, _server_handle) = web_rpc::Builder::new(channel.port1())
        .with_server(ServiceServer::new(ServiceServerImpl))
        .build().await
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    /* create client */
    let client = web_rpc::Builder::new(channel.port2())
        .with_client::<ServiceClient>()
        .build().await;
    /* run test */
    let response = client.concat_with_space("hello".into(), "world".into()).await
        .expect("RPC failure");
    assert_eq!(response, "hello world");
}