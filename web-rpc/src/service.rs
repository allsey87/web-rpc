use std::collections::HashMap;

use futures_channel::{mpsc, oneshot};
use futures_core::{future::LocalBoxFuture, Future};
use futures_util::{future::Shared, stream::FuturesUnordered, StreamExt};
use js_sys::{Array, Uint8Array};
use serde::Serialize;

pub enum ExecuteResult<R> {
    /// Single response (Some) or notification (None)
    Response(Option<(R, Array, Array)>),
    /// Stream completed — end signal was sent through stream_tx
    StreamComplete,
}

/// Stream messages: Some = item, None = end of stream
pub type StreamMessage<R> = (usize, Option<(R, Array, Array)>);

pub trait Service {
    type Response;

    fn execute(
        &self,
        seq_id: usize,
        abort_rx: oneshot::Receiver<()>,
        payload: Vec<u8>,
        js_args: Array,
        stream_tx: mpsc::UnboundedSender<StreamMessage<Self::Response>>,
    ) -> impl Future<Output = (usize, ExecuteResult<Self::Response>)>;
}

pub(crate) async fn task<S>(
    service: S,
    port: crate::port::Port,
    mut dispatcher: Shared<LocalBoxFuture<'static, ()>>,
    mut server_requests_rx: mpsc::UnboundedReceiver<(usize, Vec<u8>, js_sys::Array)>,
    mut abort_requests_rx: mpsc::UnboundedReceiver<usize>,
) where
    S: Service + 'static,
    <S as Service>::Response: Serialize,
{
    let (stream_tx, mut stream_rx) = mpsc::unbounded();
    let mut server_tasks: HashMap<usize, oneshot::Sender<_>> = Default::default();
    let mut server_responses_rx: FuturesUnordered<_> = Default::default();
    loop {
        futures_util::select! {
            _ = dispatcher => {}
            server_request = server_requests_rx.next() => {
                let (seq_id, payload, post_args) = server_request.unwrap();
                let (abort_tx, abort_rx) = oneshot::channel::<()>();
                server_tasks.insert(seq_id, abort_tx);
                server_responses_rx.push(
                    service.execute(seq_id, abort_rx, payload, post_args, stream_tx.clone())
                );
            },
            abort_request = abort_requests_rx.next() => {
                if let Some(seq_id) = abort_request {
                    if let Some(abort_tx) = server_tasks.remove(&seq_id) {
                        let _ = abort_tx.send(());
                    }
                }
            },
            stream_msg = stream_rx.next() => {
                if let Some((seq_id, msg)) = stream_msg {
                    match msg {
                        Some((response, post_args, transfer_args)) => {
                            let header = crate::MessageHeader::StreamItem(seq_id);
                            let header_bytes = bincode::serialize(&header).unwrap();
                            let header_buffer = Uint8Array::from(&header_bytes[..]).buffer();
                            let response_bytes = bincode::serialize(&response).unwrap();
                            let response_buffer = Uint8Array::from(&response_bytes[..]).buffer();
                            post_args.unshift(&response_buffer);
                            post_args.unshift(&header_buffer);
                            transfer_args.unshift(&response_buffer);
                            transfer_args.unshift(&header_buffer);
                            port.post_message(&post_args, &transfer_args).unwrap();
                        }
                        None => {
                            server_tasks.remove(&seq_id);
                            let header = crate::MessageHeader::StreamEnd(seq_id);
                            let header_bytes = bincode::serialize(&header).unwrap();
                            let header_buffer = Uint8Array::from(&header_bytes[..]).buffer();
                            let post_args = js_sys::Array::of1(&header_buffer);
                            let transfer_args = js_sys::Array::of1(&header_buffer);
                            port.post_message(&post_args, &transfer_args).unwrap();
                        }
                    }
                }
            },
            server_response = server_responses_rx.next() => {
                if let Some((seq_id, result)) = server_response {
                    match result {
                        ExecuteResult::Response(response) => {
                            if server_tasks.remove(&seq_id).is_some() {
                                if let Some((response, post_args, transfer_args)) = response {
                                    let header = crate::MessageHeader::Response(seq_id);
                                    let header_bytes = bincode::serialize(&header).unwrap();
                                    let header_buffer = Uint8Array::from(&header_bytes[..]).buffer();
                                    let response_bytes = bincode::serialize(&response).unwrap();
                                    let response_buffer = Uint8Array::from(&response_bytes[..]).buffer();
                                    post_args.unshift(&response_buffer);
                                    post_args.unshift(&header_buffer);
                                    transfer_args.unshift(&response_buffer);
                                    transfer_args.unshift(&header_buffer);
                                    port.post_message(&post_args, &transfer_args).unwrap();
                                }
                            }
                        }
                        ExecuteResult::StreamComplete => {
                            // End signal already sent through stream_tx
                            server_tasks.remove(&seq_id);
                        }
                    }
                }
            }
        }
    }
}
