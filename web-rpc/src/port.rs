use wasm_bindgen::JsValue;

pub trait Port: AsRef<web_sys::EventTarget> {
    fn post_message(
        &self,
        message: &JsValue,
        transfer: &JsValue
    ) -> Result<(), JsValue>;
    
    fn start(&self) {}
}

impl Port for web_sys::DedicatedWorkerGlobalScope {
    fn post_message(
        &self,
        message: &JsValue,
        transfer: &JsValue
    ) -> Result<(), JsValue> {
        self.post_message_with_transfer(message, transfer)
    }
}

impl Port for web_sys::Worker {
    fn post_message(
        &self,
        message: &JsValue,
        transfer: &JsValue
    ) -> Result<(), JsValue> {
        self.post_message_with_transfer(message, transfer)
    }
}

impl Port for web_sys::MessagePort {
    fn post_message(
        &self,
        message: &JsValue,
        transfer: &JsValue
    ) -> Result<(), JsValue> {
        self.post_message_with_transferable(message, transfer)
    }

    fn start(&self) {
        self.start()
    }
}