use std::collections::HashMap;

use futures_channel::{mpsc, oneshot};
use futures_util::{stream::FuturesUnordered, StreamExt};
use js_sys::{Array, Uint8Array};
use serde::{de::DeserializeOwned, Serialize};

pub struct None;
pub struct Some<S>(pub S);

pub trait Server {
    type Request: DeserializeOwned + Serialize;
    type Response: DeserializeOwned + Serialize;

    fn execute(
        &self,
        seq_id: u32,
        abort_rx: oneshot::Receiver<()>,
        request: Self::Request,
        js_args: Array
    ) -> impl futures_core::Future<Output = (u32, Option<(Self::Response, Array, Array)>)>;
}

pub(crate) async fn task<S, I, R>(
    server: S,
    interface: I,
    mut server_requests_rx: mpsc::UnboundedReceiver<(u32, <S as Server>::Request, js_sys::Array)>,
    mut abort_requests_rx: mpsc::UnboundedReceiver<u32>,
) where
    S: Server + 'static,
    I: crate::interface::Interface + 'static,
    R: Serialize {
    let mut server_tasks: HashMap<u32, oneshot::Sender<_>> = Default::default();
    let mut server_responses_rx: FuturesUnordered<_> = Default::default();
    loop {
        futures_util::select! {
            server_request = server_requests_rx.next() => {
                let (seq_id, request, post_args) = server_request.unwrap();
                let (abort_tx, abort_rx) = oneshot::channel::<()>();
                server_tasks.insert(seq_id, abort_tx);
                server_responses_rx.push(server.execute(seq_id, abort_rx, request, post_args));
            },
            abort_request = abort_requests_rx.next() => {
                if let Option::Some(seq_id) = abort_request {
                    if let Option::Some(abort_tx) = server_tasks.remove(&seq_id) {
                        /* this causes S::execute to terminate with None */
                        let _ = abort_tx.send(());
                    }
                }
            },
            server_response = server_responses_rx.next() => {
                if let Option::Some((seq_id, response)) = server_response {
                    if server_tasks.remove(&seq_id).is_some() {
                        if let Option::Some((response, post_args, transfer_args)) = response {
                            let response = crate::Message::<R, S::Response>::Response(seq_id, response);
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