use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use futures_channel::{mpsc, oneshot};
use futures_core::{future::LocalBoxFuture, Future, Stream};
use futures_util::{future, FutureExt, StreamExt};
use js_sys::Array;
use serde::Serialize;

use crate::{port::Port, Dispatcher, MessageHeader};

#[doc(hidden)]
pub trait Client {
    type Response;
}

#[doc(hidden)]
pub type CallbackMap<Response> = HashMap<u32, oneshot::Sender<(Response, Array)>>;

#[doc(hidden)]
pub type StreamCallbackMap<Response> = HashMap<u32, mpsc::UnboundedSender<(Response, Array)>>;

/// Everything a generated client holds. Clones share the maps and the sequence counter, so
/// clones of one client never collide.
#[doc(hidden)]
pub struct State<Response> {
    pub callbacks: Rc<RefCell<CallbackMap<Response>>>,
    pub stream_callbacks: Rc<RefCell<StreamCallbackMap<Response>>>,
    pub port: Port,
    pub listener: Rc<gloo_events::EventListener>,
    pub dispatcher: Dispatcher,
    pub sequence: Rc<Cell<u32>>,
}

impl<Response> Clone for State<Response> {
    fn clone(&self) -> Self {
        Self {
            callbacks: self.callbacks.clone(),
            stream_callbacks: self.stream_callbacks.clone(),
            port: self.port.clone(),
            listener: self.listener.clone(),
            dispatcher: self.dispatcher.clone(),
            sequence: self.sequence.clone(),
        }
    }
}

impl<Response: 'static> State<Response> {
    /// Post a request and return its sequence number.
    pub fn send(&self, request: &impl Serialize, post_args: &Array, transfer_args: &Array) -> u32 {
        let sequence = self.sequence.get();
        self.sequence.set(sequence.wrapping_add(1));
        crate::post_message(
            &self.port,
            MessageHeader::Request(sequence),
            request,
            post_args,
            transfer_args,
        );
        sequence
    }

    /// Await the response to a request sent with [`State::send`].
    pub fn request<T: 'static>(
        &self,
        sequence: u32,
        decode: impl FnOnce(Response, Array) -> T + 'static,
    ) -> RequestFuture<T> {
        let (response_tx, response_rx) = oneshot::channel();
        self.callbacks.borrow_mut().insert(sequence, response_tx);
        let result = response_rx.map(move |received| {
            let (response, js_values) = received.expect("web_rpc: the response channel closed");
            decode(response, js_values)
        });
        let callbacks = self.callbacks.clone();
        let port = self.port.clone();
        RequestFuture {
            result: future::select(result.boxed_local(), self.dispatcher.clone())
                .map(|selected| match selected {
                    future::Either::Left((result, _)) => result,
                    future::Either::Right(_) => {
                        unreachable!(
                            "web_rpc: the dispatcher completed while a request was pending"
                        )
                    }
                })
                .boxed_local(),
            _listener: self.listener.clone(),
            abort: AbortOnDrop::new(move || {
                callbacks.borrow_mut().remove(&sequence);
                crate::post_header(&port, MessageHeader::Abort(sequence));
            }),
        }
    }

    /// Receive the items of a stream started with [`State::send`].
    pub fn stream<T: 'static>(
        &self,
        sequence: u32,
        mut decode: impl FnMut(Response, Array) -> T + 'static,
    ) -> StreamReceiver<T> {
        let (item_tx, item_rx) = mpsc::unbounded();
        self.stream_callbacks.borrow_mut().insert(sequence, item_tx);
        let items = item_rx.map(move |(response, js_values)| decode(response, js_values));
        let stream_callbacks = self.stream_callbacks.clone();
        let port = self.port.clone();
        StreamReceiver {
            items: Box::pin(items),
            dispatcher: self.dispatcher.clone(),
            _listener: self.listener.clone(),
            abort: AbortOnDrop::new(move || {
                stream_callbacks.borrow_mut().remove(&sequence);
                crate::post_header(&port, MessageHeader::Abort(sequence));
            }),
        }
    }
}

/// Runs `abort` when dropped, unless disarmed first.
struct AbortOnDrop {
    active: bool,
    abort: Box<dyn Fn()>,
}

impl AbortOnDrop {
    fn new(abort: impl Fn() + 'static) -> Self {
        Self {
            active: true,
            abort: Box::new(abort),
        }
    }

    fn fire(&mut self) {
        if self.active {
            self.active = false;
            (self.abort)();
        }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.fire();
    }
}

/// This future represents a RPC request that is currently being executed. Note that
/// dropping this future will result in the RPC request being cancelled
#[must_use = "Either await this future or remove the return type from the RPC method"]
pub struct RequestFuture<T: 'static> {
    result: LocalBoxFuture<'static, T>,
    _listener: Rc<gloo_events::EventListener>,
    abort: AbortOnDrop,
}

impl<T> Future for RequestFuture<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let polled = self.result.poll_unpin(cx);
        if polled.is_ready() {
            self.abort.disarm();
        }
        polled
    }
}

/// A stream of items from a streaming RPC method. Dropping this will send an
/// abort to the server, cancelling the stream. Call [`close`](StreamReceiver::close)
/// to stop the server while still draining buffered items.
pub struct StreamReceiver<T: 'static> {
    items: Pin<Box<dyn Stream<Item = T>>>,
    dispatcher: Dispatcher,
    _listener: Rc<gloo_events::EventListener>,
    abort: AbortOnDrop,
}

impl<T> StreamReceiver<T> {
    /// Stop the server from producing more items. Buffered items can still
    /// be drained by continuing to poll the stream.
    pub fn close(&mut self) {
        self.abort.fire();
    }
}

impl<T> Stream for StreamReceiver<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.items.as_mut().poll_next(cx) {
            Poll::Ready(Some(item)) => Poll::Ready(Some(item)),
            Poll::Ready(None) => {
                self.abort.disarm();
                Poll::Ready(None)
            }
            Poll::Pending => match self.dispatcher.poll_unpin(cx) {
                Poll::Ready(_) => {
                    unreachable!("web_rpc: the dispatcher completed while a stream was open")
                }
                Poll::Pending => Poll::Pending,
            },
        }
    }
}
