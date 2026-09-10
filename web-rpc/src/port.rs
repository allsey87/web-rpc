use wasm_bindgen::JsValue;

/// The Javascript types that send and receive messages.
///
/// A port is a transport that web-rpc is handed, never one that it owns: each variant is a
/// cheap handle to a Javascript object, and dropping the last clone does nothing.
///
/// - Nothing here terminates a [`web_sys::Worker`]. Whoever created the worker terminates it.
/// - Nothing here calls [`web_sys::MessagePort::start`]. **A `MessagePort` must be started by
///   its owner before it is handed over**, otherwise it delivers nothing to the listener that
///   [`crate::Interface::new`] installs and the handshake never completes.
#[derive(Clone)]
pub enum Port {
    Worker(web_sys::Worker),
    DedicatedWorkerGlobalScope(web_sys::DedicatedWorkerGlobalScope),
    MessagePort(web_sys::MessagePort),
}

impl Port {
    /// Post a message with a transfer list.
    pub fn post_message(&self, message: &JsValue, transfer: &JsValue) -> Result<(), JsValue> {
        match self {
            Port::Worker(worker) => worker.post_message_with_transfer(message, transfer),
            Port::DedicatedWorkerGlobalScope(scope) => {
                scope.post_message_with_transfer(message, transfer)
            }
            Port::MessagePort(port) => port.post_message_with_transferable(message, transfer),
        }
    }

    pub(crate) fn event_target(&self) -> &web_sys::EventTarget {
        match self {
            Port::Worker(worker) => worker.as_ref(),
            Port::DedicatedWorkerGlobalScope(scope) => scope.as_ref(),
            Port::MessagePort(port) => port.as_ref(),
        }
    }
}

impl From<web_sys::Worker> for Port {
    fn from(worker: web_sys::Worker) -> Self {
        Port::Worker(worker)
    }
}

impl From<web_sys::DedicatedWorkerGlobalScope> for Port {
    fn from(scope: web_sys::DedicatedWorkerGlobalScope) -> Self {
        Port::DedicatedWorkerGlobalScope(scope)
    }
}

impl From<web_sys::MessagePort> for Port {
    fn from(port: web_sys::MessagePort) -> Self {
        Port::MessagePort(port)
    }
}
