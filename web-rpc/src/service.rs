use std::collections::HashMap;

use futures_channel::{mpsc, oneshot};
use futures_core::Future;
use futures_util::{stream::FuturesUnordered, StreamExt};
use js_sys::Array;
use serde::Serialize;

use crate::{port::Port, Dispatcher, MessageHeader};

/// A response with its Javascript values and its transfer list.
pub type Outgoing<Response> = (Response, Array, Array);

pub enum ExecuteResult<Response> {
    /// What to post back, if anything. A notification and an aborted request post nothing.
    Response(Option<Outgoing<Response>>),
    /// The stream ended. Its end marker already travelled through the item channel, behind
    /// the last item, so nothing more is posted here.
    StreamComplete,
}

/// One message of a stream: an item, or `None` once it ends.
pub type StreamMessage<Response> = (u32, Option<Outgoing<Response>>);

pub trait Service {
    type Response;

    fn execute(
        &self,
        sequence: u32,
        abort_rx: oneshot::Receiver<()>,
        payload: Vec<u8>,
        js_args: Array,
        stream_tx: mpsc::UnboundedSender<StreamMessage<Self::Response>>,
    ) -> impl Future<Output = (u32, ExecuteResult<Self::Response>)>;
}

/// An inbound request: sequence number, payload bytes, and the Javascript values behind them.
pub type Request = (u32, Vec<u8>, Array);

pub(crate) async fn task<S>(
    service: S,
    port: Port,
    mut dispatcher: Dispatcher,
    mut requests_rx: mpsc::UnboundedReceiver<Request>,
    mut aborts_rx: mpsc::UnboundedReceiver<u32>,
) where
    S: Service + 'static,
    S::Response: Serialize,
{
    let (stream_tx, mut stream_rx) = mpsc::unbounded();
    let mut running: HashMap<u32, oneshot::Sender<()>> = HashMap::new();
    let mut executions: FuturesUnordered<_> = FuturesUnordered::new();
    loop {
        futures_util::select! {
            _ = dispatcher => {}
            request = requests_rx.next() => {
                let (sequence, payload, js_args) = request.expect("web_rpc: the request channel closed");
                let (abort_tx, abort_rx) = oneshot::channel();
                running.insert(sequence, abort_tx);
                executions.push(service.execute(sequence, abort_rx, payload, js_args, stream_tx.clone()));
            },
            abort = aborts_rx.next() => {
                if let Some(sequence) = abort {
                    if let Some(abort_tx) = running.remove(&sequence) {
                        let _ = abort_tx.send(());
                    }
                }
            },
            message = stream_rx.next() => {
                if let Some((sequence, message)) = message {
                    match message {
                        Some((item, post_args, transfer_args)) => {
                            crate::post_message(&port, MessageHeader::StreamItem(sequence), &item, &post_args, &transfer_args);
                        }
                        None => {
                            running.remove(&sequence);
                            crate::post_header(&port, MessageHeader::StreamEnd(sequence));
                        }
                    }
                }
            },
            execution = executions.next() => {
                if let Some((sequence, result)) = execution {
                    match result {
                        ExecuteResult::Response(response) => {
                            // An aborted request has already been forgotten, and its response
                            // is dropped with it.
                            if running.remove(&sequence).is_some() {
                                if let Some((response, post_args, transfer_args)) = response {
                                    crate::post_message(&port, MessageHeader::Response(sequence), &response, &post_args, &transfer_args);
                                }
                            }
                        }
                        ExecuteResult::StreamComplete => {}
                    }
                }
            }
        }
    }
}
