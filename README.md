[![CI](https://github.com/allsey87/web-rpc/actions/workflows/test.yaml/badge.svg)](https://github.com/allsey87/web-rpc/actions)
[![Crates.io](https://img.shields.io/crates/v/web-rpc.svg)](https://crates.io/crates/web-rpc)
[![api-docs](https://docs.rs/web-rpc/badge.svg)](https://docs.rs/web-rpc/)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

# web-rpc
Bidirectional RPC for browsing contexts, web workers, and message channels. Inspired by Google's [tarpc](https://github.com/google/tarpc): define a service as a trait, annotate, and the macro generates the client and server. JS-bearing types (anything `AsRef<JsValue>`) are auto-routed through `postMessage`; everything else through bincode. Put the trait definition in a shared crate so both ends share it.

```rust
#[web_rpc::service]
pub trait Calculator {
    fn add(&self, left: u32, right: u32) -> u32;
}
```

The macro generates `CalculatorClient`, `CalculatorService`, and a `Calculator` trait you implement on the server side:

```rust
struct CalculatorImpl;
impl Calculator for CalculatorImpl {
    fn add(&self, left: u32, right: u32) -> u32 { left + right }
}
```

Wire up over a `MessageChannel` (or a `Worker` / `MessagePort`):

```rust
let channel = web_sys::MessageChannel::new().unwrap();
let (server_iface, client_iface) = futures_util::future::join(
    web_rpc::Interface::new(channel.port1()),
    web_rpc::Interface::new(channel.port2()),
).await;

let server = web_rpc::Builder::new(server_iface)
    .with_service::<CalculatorService<_>>(CalculatorImpl)
    .build();
wasm_bindgen_futures::spawn_local(server);

let client = web_rpc::Builder::new(client_iface)
    .with_client::<CalculatorClient>()
    .build();

assert_eq!(client.add(41, 1).await, 42);
```

## Features
- **Auto-routing of JS vs serializable types**: types implementing `AsRef<JsValue>` (e.g. `JsString`, `OffscreenCanvas`) are posted; everything else is serialized using bincode.
- **`Option`/`Result` wrappers (and nested)**: each variant routes independently, so types like `Result<JsT, RustError>` and `Result<Option<JsT>, _>` just work.
- **Bidirectional RPC** over a single channel, with both ends simultaneously acting as client and server.
- **Streaming RPCs** via `impl Stream<Item = T>` returns, with abort-on-drop and close-and-drain.
- **Transfer semantics** via `#[transfer(...)]`: bare params, derived expressions (`data => data.buffer()`), refutable closures (`return => |Ok(buf)| buf.buffer()`), or `match`-blocks.
- **Async or sync** server methods, with per-request cancellation when the client drops the future of an RPC method that returns.
- **Notifications**: methods with no return type are fire-and-forget.
- **Borrowed `&str` / `&[u8]`**: zero-copy deserialization on the server side.
- **Conditional methods**: `#[cfg(...)]` on a trait method is propagated to all generated code, so the method is stripped from the client, server, and wire format when the cfg is off.

See the [crate documentation](https://docs.rs/web-rpc/latest/web_rpc/) for the full feature reference. Need help with your latest project? Get in touch via [contact@allwright.io](mailto:contact@allwright.io). I'm available for new assignments.
