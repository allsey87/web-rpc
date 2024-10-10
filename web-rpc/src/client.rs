use std::{cell::RefCell, collections::HashMap, pin::Pin, rc::Rc, task::{Context, Poll}};

use futures_channel::oneshot;
use futures_core::{future::LocalBoxFuture, Future};
use futures_util::{future::{self, Shared}, FutureExt};

#[doc(hidden)]
pub trait Client {
    type Request;
    type Response;
}

#[doc(hidden)]
pub type CallbackMap<Response> = HashMap<
    usize,
    oneshot::Sender<(Response, js_sys::Array)>
>;

#[doc(hidden)]
pub type Configuration<Request, Response> = (
    Rc<RefCell<CallbackMap<Response>>>,
    crate::port::Port,
    Rc<gloo_events::EventListener>,
    Shared<LocalBoxFuture<'static, ()>>,
    Rc<dyn Fn(usize, Request) -> Vec<u8>>,
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
                abort
            })
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
