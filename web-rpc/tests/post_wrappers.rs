use futures_core::Stream;
use futures_util::{FutureExt, StreamExt};
use wasm_bindgen_test::*;

// --- Option<JsType> as post return (non-streaming) ---

#[web_rpc::service]
pub trait OptionalReturn {
    #[post(return)]
    fn maybe_string(&self, return_some: bool) -> Option<js_sys::JsString>;
}

struct OptionalReturnImpl;
impl OptionalReturn for OptionalReturnImpl {
    fn maybe_string(&self, return_some: bool) -> Option<js_sys::JsString> {
        if return_some {
            Some(js_sys::JsString::from("hello"))
        } else {
            None
        }
    }
}

#[wasm_bindgen_test]
async fn option_post_return_some() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<OptionalReturnService<_>>(OptionalReturnImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<OptionalReturnClient>()
        .build();

    let result = client.maybe_string(true).await;
    assert_eq!(result, Some(js_sys::JsString::from("hello")));
}

#[wasm_bindgen_test]
async fn option_post_return_none() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<OptionalReturnService<_>>(OptionalReturnImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<OptionalReturnClient>()
        .build();

    let result = client.maybe_string(false).await;
    assert_eq!(result, None);
}

// --- Result<JsType, JsType> as post return (non-streaming) ---

#[web_rpc::service]
pub trait FallibleReturn {
    #[post(return)]
    fn try_string(&self, succeed: bool) -> Result<js_sys::JsString, js_sys::Error>;
}

struct FallibleReturnImpl;
impl FallibleReturn for FallibleReturnImpl {
    fn try_string(&self, succeed: bool) -> Result<js_sys::JsString, js_sys::Error> {
        if succeed {
            Ok(js_sys::JsString::from("success"))
        } else {
            Err(js_sys::Error::new("something went wrong"))
        }
    }
}

#[wasm_bindgen_test]
async fn result_post_return_ok() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<FallibleReturnService<_>>(FallibleReturnImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<FallibleReturnClient>()
        .build();

    let result = client.try_string(true).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), js_sys::JsString::from("success"));
}

#[wasm_bindgen_test]
async fn result_post_return_err() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<FallibleReturnService<_>>(FallibleReturnImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<FallibleReturnClient>()
        .build();

    let result = client.try_string(false).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().message(), "something went wrong");
}

// --- Option<JsType> as post arg ---

#[web_rpc::service]
pub trait OptionalArg {
    #[post(value)]
    fn string_len(&self, value: Option<js_sys::JsString>) -> u32;
}

struct OptionalArgImpl;
impl OptionalArg for OptionalArgImpl {
    fn string_len(&self, value: Option<js_sys::JsString>) -> u32 {
        value.map(|s| s.length()).unwrap_or(0)
    }
}

#[wasm_bindgen_test]
async fn option_post_arg_some() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<OptionalArgService<_>>(OptionalArgImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<OptionalArgClient>()
        .build();

    let result = client
        .string_len(Some(js_sys::JsString::from("test")))
        .await;
    assert_eq!(result, 4);
}

#[wasm_bindgen_test]
async fn option_post_arg_none() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<OptionalArgService<_>>(OptionalArgImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<OptionalArgClient>()
        .build();

    let result = client.string_len(None).await;
    assert_eq!(result, 0);
}

// --- Streaming with Option<JsType> post return ---

#[web_rpc::service]
pub trait StreamOptional {
    #[post(return)]
    fn stream_maybe(&self, count: u32) -> impl Stream<Item = Option<js_sys::JsString>>;
}

struct StreamOptionalImpl;
impl StreamOptional for StreamOptionalImpl {
    fn stream_maybe(&self, count: u32) -> impl Stream<Item = Option<js_sys::JsString>> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        for i in 0..count {
            if i % 2 == 0 {
                let _ = tx.unbounded_send(Some(js_sys::JsString::from(format!("item-{}", i))));
            } else {
                let _ = tx.unbounded_send(None);
            }
        }
        rx
    }
}

#[wasm_bindgen_test]
async fn streaming_option_post_return() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<StreamOptionalService<_>>(StreamOptionalImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<StreamOptionalClient>()
        .build();

    let items: Vec<Option<js_sys::JsString>> = client.stream_maybe(4).collect().await;
    assert_eq!(items.len(), 4);
    assert_eq!(items[0], Some(js_sys::JsString::from("item-0")));
    assert_eq!(items[1], None);
    assert_eq!(items[2], Some(js_sys::JsString::from("item-2")));
    assert_eq!(items[3], None);
}

// --- Streaming with Result<JsType, JsType> post return ---

#[web_rpc::service]
pub trait StreamFallible {
    #[post(return)]
    fn stream_results(
        &self,
        count: u32,
    ) -> impl Stream<Item = Result<js_sys::JsString, js_sys::Error>>;
}

struct StreamFallibleImpl;
impl StreamFallible for StreamFallibleImpl {
    fn stream_results(
        &self,
        count: u32,
    ) -> impl Stream<Item = Result<js_sys::JsString, js_sys::Error>> {
        let (tx, rx) = futures_channel::mpsc::unbounded();
        for i in 0..count {
            if i % 2 == 0 {
                let _ = tx.unbounded_send(Ok(js_sys::JsString::from(format!("ok-{}", i))));
            } else {
                let _ = tx.unbounded_send(Err(js_sys::Error::new(&format!("err-{}", i))));
            }
        }
        rx
    }
}

#[wasm_bindgen_test]
async fn streaming_result_post_return() {
    console_error_panic_hook::set_once();
    let channel = web_sys::MessageChannel::new().unwrap();
    let (server_interface, client_interface) = futures_util::future::join(
        web_rpc::Interface::new(channel.port1()),
        web_rpc::Interface::new(channel.port2()),
    )
    .await;
    let (server, _handle) = web_rpc::Builder::new(server_interface)
        .with_service::<StreamFallibleService<_>>(StreamFallibleImpl)
        .build()
        .remote_handle();
    wasm_bindgen_futures::spawn_local(server);
    let client = web_rpc::Builder::new(client_interface)
        .with_client::<StreamFallibleClient>()
        .build();

    let items: Vec<Result<js_sys::JsString, js_sys::Error>> =
        client.stream_results(4).collect().await;
    assert_eq!(items.len(), 4);
    assert!(items[0].is_ok());
    assert_eq!(items[0].as_ref().unwrap(), &js_sys::JsString::from("ok-0"));
    assert!(items[1].is_err());
    assert_eq!(items[1].as_ref().unwrap_err().message(), "err-1");
    assert!(items[2].is_ok());
    assert_eq!(items[2].as_ref().unwrap(), &js_sys::JsString::from("ok-2"));
    assert!(items[3].is_err());
    assert_eq!(items[3].as_ref().unwrap_err().message(), "err-3");
}
