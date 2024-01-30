use std::rc::Rc;

use futures_channel::{mpsc, oneshot};
use futures_util::future;
use wasm_bindgen::{JsCast, JsValue};

pub struct Interface<P> {
    pub(crate) port: Rc<P>,
    pub(crate) listener: gloo_events::EventListener,
    pub(crate) messages_rx: mpsc::UnboundedReceiver<js_sys::Array>,
}

impl<P: crate::port::Port + 'static> Interface<P> {
    pub async fn new(port: P) -> Self {
        let port = Rc::new(port);
        let (dispatcher_tx, dispatcher_rx) = mpsc::unbounded();
        let (ready_tx, ready_rx) = oneshot::channel();
        let mut ready_tx = Option::from(ready_tx);
        let listener = gloo_events::EventListener::new((*port).as_ref(), "message", move |event| {
            let message = event.unchecked_ref::<web_sys::MessageEvent>().data();
            match message.dyn_into::<js_sys::Array>() {
                /* default path, enqueue the message for deserialization by the dispatcher */
                Ok(array) => {
                    let _ = dispatcher_tx.unbounded_send(array);
                },
                /* handshake path */
                Err(_) => if let Some(ready_tx) = ready_tx.take() {
                    let _ = ready_tx.send(());
                }
            }
        });
        /* needed for P = MessagePort */
        port.start();
        /* poll other end of the channel */
        let port_cloned = port.clone();
        let poll = async move {
            loop {
                port_cloned.post_message(&JsValue::NULL, &JsValue::UNDEFINED).unwrap();
                gloo_timers::future::TimeoutFuture::new(10).await;
            }
        };
        pin_utils::pin_mut!(poll);
        future::select(ready_rx, poll).await;
        /* at this point we know the other end's listener is available, but we may
           need to send one last message to indicate that we are available */
        port.post_message(&JsValue::NULL, &JsValue::UNDEFINED).unwrap();
        /* return the interface */
        Self {
            messages_rx: dispatcher_rx,
            listener,
            port,
        }
    }
}

