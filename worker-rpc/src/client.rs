use std::{pin::Pin, task::{Context, Poll}};

use futures_channel::{mpsc, oneshot};
use futures_core::{future::LocalBoxFuture, Future};
use futures_util::FutureExt;
use js_sys::Array;
use serde::{de::DeserializeOwned, Serialize};

pub trait Client {
    type Request: DeserializeOwned + Serialize;
    type Response: DeserializeOwned + Serialize;
}

pub struct None;

impl Client for None {
    type Request = ();
    type Response = ();
}

impl From<RequestSender<None>> for None {
    fn from(_: RequestSender<None>) -> Self {
        Self
    }
}

pub type RequestSender<C> = mpsc::UnboundedSender<(
    <C as Client>::Request,
    Array,
    Array,
    oneshot::Sender<(<C as Client>::Response, Array)>,
    oneshot::Receiver<()>
)>;

pub type RequestReceiver<C> = mpsc::UnboundedReceiver<(
    <C as Client>::Request,
    Array,
    Array,
    oneshot::Sender<(<C as Client>::Response, Array)>,
    oneshot::Receiver<()>
)>;

pub struct RequestFuture<T> {
    result: LocalBoxFuture<'static, T>,
    _abort_tx: oneshot::Sender<()>,
}

impl<T> RequestFuture<T> {
    pub fn new(
        result: impl Future<Output = T> + 'static,
        abort_tx: oneshot::Sender<()>
    ) -> Self {
        Self {
            result: result.boxed_local(),
            _abort_tx: abort_tx
        }
    }
}

impl<T> Future for RequestFuture<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.result.as_mut().poll_unpin(cx)
    }
}