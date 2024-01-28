use std::{collections::HashMap, rc::Rc};

use futures_channel::{mpsc, oneshot};
use futures_core::Future;
use futures_util::{stream::FuturesUnordered, StreamExt};
use js_sys::{Array, Uint8Array};
use serde::Serialize;

pub trait Service {
    type Request;
    type Response;

    fn execute(
        &self,
        seq_id: usize,
        abort_rx: oneshot::Receiver<()>,
        request: Self::Request,
        js_args: Array
    ) -> impl Future<Output = (usize, Option<(Self::Response, Array, Array)>)>;
}

pub(crate) async fn task<S, I, Request>(
    service: S,
    interface: Rc<I>,
    mut server_requests_rx: mpsc::UnboundedReceiver<(usize, <S as Service>::Request, js_sys::Array)>,
    mut abort_requests_rx: mpsc::UnboundedReceiver<usize>,
) where
    S: Service + 'static,
    I: crate::interface::Interface + 'static,
    Request: Serialize,
    <S as Service>::Response: Serialize {
    let mut server_tasks: HashMap<usize, oneshot::Sender<_>> = Default::default();
    let mut server_responses_rx: FuturesUnordered<_> = Default::default();
    loop {
        futures_util::select! {
            server_request = server_requests_rx.next() => {
                let (seq_id, request, post_args) = server_request.unwrap();
                let (abort_tx, abort_rx) = oneshot::channel::<()>();
                server_tasks.insert(seq_id, abort_tx);
                server_responses_rx.push(service.execute(seq_id, abort_rx, request, post_args));
            },
            abort_request = abort_requests_rx.next() => {
                if let Some(seq_id) = abort_request {
                    if let Some(abort_tx) = server_tasks.remove(&seq_id) {
                        let _ = abort_tx.send(());
                    }
                }
            },
            server_response = server_responses_rx.next() => {
                if let Some((seq_id, response)) = server_response {
                    if server_tasks.remove(&seq_id).is_some() {
                        if let Some((response, post_args, transfer_args)) = response {
                            let response = crate::Message::<Request, S::Response>::Response(seq_id, response);
                            let response = bincode::serialize(&response).unwrap();
                            let buffer = Uint8Array::from(&response[..]).buffer();
                            post_args.unshift(&buffer);
                            transfer_args.unshift(&buffer);
                            interface.post_message(&post_args, &transfer_args).unwrap();
                        }
                    }
                }
            }
        }
    }
}