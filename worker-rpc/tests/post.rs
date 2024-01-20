use wasm_bindgen_test::*;

#[worker_rpc::service]
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
    let channel = web_sys::MessageChannel::new().unwrap();
    let port1 = channel.port1();
    let port2 = channel.port2();
    let server = ServiceServer::new(ServiceServerImpl);
    let _server_interface = worker_rpc::Interface::from(port1)
        .with_server(server)
        .connect();
    let client_interface = worker_rpc::Interface::from(port2)
        .with_client::<ServiceClient>()
        .connect();
    let response = client_interface.concat_with_space("hello".into(), "world".into()).await
        .expect("RPC failure");
    assert_eq!(response, "hello world");
}