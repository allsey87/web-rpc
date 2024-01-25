use futures_channel::oneshot;
use gloo_events::EventListener;
use wasm_bindgen::JsValue;

pub trait Interface: AsRef<web_sys::EventTarget> + Clone {   
    fn post_message(
        &self,
        message: &JsValue,
        transfer: &JsValue
    ) -> std::result::Result<(), JsValue>;
    
    fn pre_attach(&self) -> impl futures_core::Future<Output = ()> {
        async {}
    }
    
    fn post_attach(&self) -> impl futures_core::Future<Output = ()> {
        async {}
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

    async fn post_attach(&self) {
        self.post_message(&JsValue::UNDEFINED).unwrap();
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

    async fn pre_attach(&self) {
        let (ready_tx, ready_rx) = oneshot::channel();
        let _ready_listener = EventListener::once(self.as_ref(), "message", move |_| {
            ready_tx.send(()).unwrap();
        });
        ready_rx.await.unwrap();
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

    async fn post_attach(&self) {
        self.start();
    }
}