use futures_channel::{mpsc, oneshot};
use futures_util::future;
use wasm_bindgen::{JsCast, JsValue};

/// An interface represents a [`crate::port::Port`] that has been fully initialised and has
/// verified that the other end of the channel is ready to receive messages.
pub struct Interface {
    pub(crate) port: crate::port::Port,
    pub(crate) listener: gloo_events::EventListener,
    pub(crate) messages_rx: mpsc::UnboundedReceiver<js_sys::Array>,
}

impl Interface {
    /// Create a new interface from anything that implements `Into<Port>`, for example, a
    /// [`web_sys::MessagePort`], a [`web_sys::Worker`], or a
    /// [`web_sys::DedicatedWorkerGlobalScope`]. This function is async and resolves to the new
    /// interface instance once the other side of the channel is ready.
    pub async fn new(port: impl Into<crate::port::Port>) -> Self {
        let port = port.into();
        let (dispatcher_tx, dispatcher_rx) = mpsc::unbounded();
        let (ready_tx, ready_rx) = oneshot::channel();
        let mut ready_tx = Option::from(ready_tx);
        let listener =
            gloo_events::EventListener::new(port.event_target(), "message", move |event| {
                let message = event.unchecked_ref::<web_sys::MessageEvent>().data();
                match message.dyn_into::<js_sys::Array>() {
                    /* default path, enqueue the message for deserialization by the dispatcher */
                    Ok(array) => {
                        let _ = dispatcher_tx.unbounded_send(array);
                    }
                    /* handshake path */
                    Err(_) => {
                        if let Some(ready_tx) = ready_tx.take() {
                            let _ = ready_tx.send(());
                        }
                    }
                }
            });
        /* needed for MessagePort */
        port.start();
        /* poll other end of the channel */
        let port_cloned = port.clone();
        let poll = async move {
            loop {
                port_cloned
                    .post_message(&JsValue::NULL, &JsValue::UNDEFINED)
                    .unwrap();
                gloo_timers::future::TimeoutFuture::new(10).await;
            }
        };
        pin_utils::pin_mut!(poll);
        future::select(ready_rx, poll).await;
        /* at this point we know the other end's listener is available, but we may
        need to send one last message to indicate that we are available */
        port.post_message(&JsValue::NULL, &JsValue::UNDEFINED)
            .unwrap();
        /* return the interface */
        Self {
            messages_rx: dispatcher_rx,
            listener,
            port,
        }
    }
}
