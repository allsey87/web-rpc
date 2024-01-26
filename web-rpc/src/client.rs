use std::{collections::HashMap, pin::Pin, task::{Context, Poll}};

use futures_channel::{mpsc, oneshot};
use futures_core::{future::LocalBoxFuture, Future};
use futures_util::{stream::FuturesUnordered, FutureExt, StreamExt};
use js_sys::Array;
use serde::{de::DeserializeOwned, Serialize};

pub struct None;
pub struct Some<C>(pub C);

pub trait Client {
    type Request: DeserializeOwned + Serialize;
    type Response: DeserializeOwned + Serialize;
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

pub(crate) async fn task<C, I, R>(
    interface: I,
    mut client_requests_rx: RequestReceiver<C>,
    mut client_responses_rx: mpsc::UnboundedReceiver<(u32, <C as Client>::Response, Array)>,
) where
    C: Client,
    I: crate::interface::Interface + 'static,
    R: Serialize {

    let mut last_seq_id: u32 = 0; // REMOVE
    let mut client_tasks: HashMap<u32, oneshot::Sender<_>> = Default::default();
    let mut abort_requests: FuturesUnordered<_> = Default::default();
    
    loop {
        futures_util::select! {
            client_request = client_requests_rx.next() => {
                if let Option::Some((request, post_args, transfer_args, response_tx, abort_rx)) = client_request {
                    let seq_id = last_seq_id;
                    last_seq_id = last_seq_id.wrapping_add(1);
                    /* abort_rx will complete once RequestFuture is dropped */
                    abort_requests.push(abort_rx.map(move |_| seq_id));
                    client_tasks.insert(seq_id, response_tx);
                    let request = crate::Message::<C::Request, R>::Request(seq_id, request);
                    let request = bincode::serialize(&request).unwrap();
                    let buffer = js_sys::Uint8Array::from(&request[..]).buffer();
                    post_args.unshift(&buffer);
                    transfer_args.unshift(&buffer);
                    interface.post_message(&post_args, &transfer_args).unwrap();
                }
            },
            abort_request = abort_requests.next() => {
                if let Option::Some(seq_id) = abort_request {
                    if client_tasks.remove(&seq_id).is_some() {
                        let abort_request = crate::Message::<C::Request, R>::Abort(seq_id);
                        let abort_request = bincode::serialize(&abort_request).unwrap();
                        let buffer = js_sys::Uint8Array::from(&abort_request[..]).buffer();
                        let post_args = js_sys::Array::of1(&buffer);
                        let transfer_args = js_sys::Array::of1(&buffer);
                        interface.post_message(&post_args, &transfer_args).unwrap();
                    }
                }
            },
            client_response = client_responses_rx.next() => {
                // TODO check why this channel started returning None, it could be a good
                // thing and could indicate that this task should shutdown
                if let Option::Some((seq_id, response, js_args)) = client_response {
                    if let Option::Some(client_task) = client_tasks.remove(&seq_id) {
                        let _ = client_task.send((response, js_args));
                    }
                }
            },
            // new: investigate the implications of this break
            complete => break,
        }
    } 
}