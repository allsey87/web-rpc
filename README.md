[![CI](https://github.com/allsey87/web-rpc/actions/workflows/test.yaml/badge.svg)](https://github.com/allsey87/web-rpc/actions)
[![Crates.io](https://img.shields.io/crates/v/web-rpc.svg)](https://crates.io/crates/web-rpc)
[![api-docs](https://docs.rs/web-rpc/badge.svg)](https://docs.rs/web-rpc/)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

# web-rpc
`web-rpc` is a crate for executing RPCs between browsing contexts, web workers, and channels. Similar to Google's [tarpc](https://github.com/google/tarpc), this crate allows you to define your RPC in code using trait syntax. This trait is consumed by a `service` macro, 
which will generate everything that you need to implement RPC. Two notable features of this crate are that it supports bidirectional RPC over a single channel (e.g., between a [Worker](https://docs.rs/web-sys/latest/web_sys/struct.Worker.html) and a [DedicatedWorkerGlobalScope](https://docs.rs/web-sys/latest/web_sys/struct.DedicatedWorkerGlobalScope.html)) and posting/transferring Javascript types (e.g., [OffscreenCanvas](https://docs.rs/web-sys/latest/web_sys/struct.OffscreenCanvas.html)). The following is a simple example, see the [crate documentation](https://docs.rs/web-rpc/latest/web_rpc/) for a more complete explaination and more advanced examples.

The following code generates the RPC components using an attribute macro applied to a trait. It is recommended to put this RPC definition into some sort of shared crate that your modules can both access.
```rust
#[web_rpc::service]
pub trait Calculator {
    fn add(&self, left: u32, right: u32) -> u32;
}
```
The code above will generate `CalculatorClient`, `CalculatorService`, and a new trait `Calculator` that you can use to implement a calculator as follows:
```rust
struct CalculatorServiceImpl;

impl Calculator for CalculatorServiceImpl {
    fn add(&self, left: u32, right: u32) -> u32 {
        left + right
    }
}
```
In the following example, we will use [MessageChannel](https://docs.rs/web-sys/latest/web_sys/struct.MessageChannel.html) and [MessagePort](https://docs.rs/web-sys/latest/web_sys/struct.MessagePort.html) since they are easy to test and demonstrate the use of this crate inside a single module. A more interesting example however, is to use this crate to communicate between two browsing contexts or a [Worker](https://docs.rs/web-sys/latest/web_sys/struct.Worker.html) and a [DedicatedWorkerGlobalScope](https://docs.rs/web-sys/latest/web_sys/struct.DedicatedWorkerGlobalScope.html). The following code defines the server:
```rust
let channel = web_sys::MessageChannel::new();
// note that interface::new is async and that both interfaces need to be polled in order to establish the connection between them
let (server_interface, client_interface) = futures_util::future::join(
    web_rpc::Interface::new(channel.port1()),
    web_rpc::Interface::new(channel.port2()),
).await;
// create a server with the first port
let server = web_rpc::Builder::new(server_interface)
    .with_service::<CalculatorService<_>>(CalculatorServiceImpl)
    .build();
// spawn the server
wasm_bindgen_futures::spawn_local(server);
```
To create a client:
```rust
// create a client using the second interface from above
let client = web_rpc::Builder::new(client_interface)
    .with_client::<CalculatorClient>()
    .build();
/* call `add` */
assert_eq!(client.add(41, 1).await, 42);
```

For more advanced examples, check out the [crate documentation](https://docs.rs/web-rpc/latest/web_rpc/). Need help with your latest project? Get in touch via [contact@allwright.io](mailto:contact@allwright.io) and tell me about what you are working on - I am a available for new assignments.