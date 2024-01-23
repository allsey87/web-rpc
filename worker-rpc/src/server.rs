use futures_channel::oneshot;
use js_sys::Array;
use serde::{de::DeserializeOwned, Serialize};

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

pub struct None;

impl Server for None {
    type Request = ();
    type Response = ();

    async fn execute(
        &self,
        _seq_id: u32,
        _abort_rx: oneshot::Receiver<()>,
        _request: Self::Request,
        _js_args: Array
    ) -> (u32, Option<(Self::Response, Array, Array)>) {
        unreachable!()
    }
}
