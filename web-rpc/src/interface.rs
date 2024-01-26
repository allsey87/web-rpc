use futures_channel::oneshot;
use futures_core::future::BoxFuture;
use futures_util::{future, FutureExt};
use gloo_events::EventListener;
use wasm_bindgen::JsValue;

pub trait Interface: AsRef<web_sys::EventTarget> {
    fn post_message(
        &self,
        message: &JsValue,
        transfer: &JsValue
    ) -> std::result::Result<(), JsValue>;
    
    fn pre_attach(&self) -> BoxFuture<'static, ()> {
        future::ready(()).boxed()
    }
    
    fn post_attach(&self) -> BoxFuture<'static, ()> {
        future::ready(()).boxed()
    }
}

impl Interface for web_sys::DedicatedWorkerGlobalScope {
    fn post_message(
        &self,
        message: &JsValue,
        transfer: &JsValue
    ) -> std::result::Result<(), JsValue> {
        self.post_message_with_transfer(message, transfer)
    }

    fn post_attach(&self) -> BoxFuture<'static, ()> {
        self.post_message(&JsValue::UNDEFINED).unwrap();
        future::ready(()).boxed()
    }
}

impl Interface for web_sys::Worker {
    fn post_message(
        &self,
        message: &JsValue,
        transfer: &JsValue
    ) -> std::result::Result<(), JsValue> {
        self.post_message_with_transfer(message, transfer)
    }

    fn pre_attach(&self) -> BoxFuture<'static, ()> {
        let (ready_tx, ready_rx) = oneshot::channel();
        let _ready_listener = EventListener::once(self.as_ref(), "message", move |_| {
            ready_tx.send(()).unwrap();
        });
        ready_rx.map(|_| ()).boxed()
    }
}

impl Interface for web_sys::MessagePort {
    fn post_message(
        &self,
        message: &JsValue,
        transfer: &JsValue
    ) -> std::result::Result<(), JsValue> {
        self.post_message_with_transferable(message, transfer)
    }

    fn post_attach(&self) -> BoxFuture<'static, ()> {
        self.start();
        future::ready(()).boxed()
    }
}