use std::{
    cell::RefCell,
    collections::HashMap,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use futures_channel::{mpsc, oneshot};
use futures_core::{future::LocalBoxFuture, Future};
use futures_util::{
    future::{self, Shared},
    FutureExt,
};

#[doc(hidden)]
pub trait Client {
    type Response;
}

#[doc(hidden)]
pub type CallbackMap<Response> = HashMap<usize, oneshot::Sender<(Response, js_sys::Array)>>;

#[doc(hidden)]
pub type StreamCallbackMap<Response> =
    HashMap<usize, mpsc::UnboundedSender<(Response, js_sys::Array)>>;

#[doc(hidden)]
pub type Configuration<Response> = (
    Rc<RefCell<CallbackMap<Response>>>,
    Rc<RefCell<StreamCallbackMap<Response>>>,
    crate::port::Port,
    Rc<gloo_events::EventListener>,
    Shared<LocalBoxFuture<'static, ()>>,
    Rc<dyn Fn(usize)>,
);

/// This future represents a RPC request that is currently being executed. Note that
/// dropping this future will result in the RPC request being cancelled
#[must_use = "Either await this future or remove the return type from the RPC method"]
pub struct RequestFuture<T: 'static> {
    result: LocalBoxFuture<'static, T>,
    abort: Pin<Box<RequestAbort>>,
}

impl<T> RequestFuture<T> {
    pub fn new(
        result: impl Future<Output = T> + 'static,
        dispatcher: Shared<LocalBoxFuture<'static, ()>>,
        abort: Box<dyn Fn()>,
    ) -> Self {
        Self {
            result: future::select(result.boxed_local(), dispatcher)
                .map(|select| match select {
                    future::Either::Left((result, _)) => result,
                    future::Either::Right(_) => unreachable!("dispatcher should not complete"),
                })
                .boxed_local(),
            abort: Box::pin(RequestAbort {
                active: true,
                abort,
            }),
        }
    }
}

struct RequestAbort {
    active: bool,
    abort: Box<dyn Fn()>,
}

impl Drop for RequestAbort {
    fn drop(&mut self) {
        if self.active {
            (self.abort)();
        }
    }
}

impl<T> Future for RequestFuture<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let poll_result = self.as_mut().result.poll_unpin(cx);
        if matches!(poll_result, Poll::Ready(_)) {
            self.as_mut().abort.active = false;
        }
        poll_result
    }
}

/// A stream of items from a streaming RPC method. Dropping this will send an
/// abort to the server, cancelling the stream. Call [`close`](StreamReceiver::close)
/// to stop the server while still draining buffered items.
pub struct StreamReceiver<T: 'static> {
    inner: Pin<Box<dyn futures_core::Stream<Item = T>>>,
    dispatcher: Shared<LocalBoxFuture<'static, ()>>,
    abort: Pin<Box<StreamAbort>>,
}

struct StreamAbort {
    active: bool,
    abort: Box<dyn Fn()>,
}

impl<T> StreamReceiver<T> {
    pub fn new(
        inner: impl futures_core::Stream<Item = T> + 'static,
        dispatcher: Shared<LocalBoxFuture<'static, ()>>,
        abort: Box<dyn Fn()>,
    ) -> Self {
        Self {
            inner: Box::pin(inner),
            dispatcher,
            abort: Box::pin(StreamAbort {
                active: true,
                abort,
            }),
        }
    }

    /// Stop the server from producing more items. Buffered items can still
    /// be drained by continuing to poll the stream.
    pub fn close(&mut self) {
        if self.abort.active {
            (self.abort.abort)();
            self.abort.active = false;
        }
    }
}

impl<T> futures_core::Stream for StreamReceiver<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(item)) => Poll::Ready(Some(item)),
            Poll::Ready(None) => {
                self.abort.active = false;
                Poll::Ready(None)
            }
            Poll::Pending => match self.dispatcher.poll_unpin(cx) {
                Poll::Ready(_) => unreachable!("dispatcher should not complete"),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

impl Drop for StreamAbort {
    fn drop(&mut self) {
        if self.active {
            (self.abort)();
        }
    }
}
