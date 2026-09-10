[![CI](https://github.com/allsey87/web-rpc/actions/workflows/test.yaml/badge.svg)](https://github.com/allsey87/web-rpc/actions)
[![Crates.io](https://img.shields.io/crates/v/web-rpc.svg)](https://crates.io/crates/web-rpc)
[![api-docs](https://docs.rs/web-rpc/badge.svg)](https://docs.rs/web-rpc/)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

# web-rpc
Bidirectional RPC for browsing contexts, web workers, and message channels. Inspired by Google's [tarpc](https://github.com/google/tarpc): define a service as a trait, annotate, and the macro generates the client, the server, and a typed JavaScript endpoint for the other end of the connection. Payloads are encoded with [postcard](https://postcard.jamesmunns.com/); values wrapped in `Post<T>` or `Transfer<T>` cross as JavaScript values through `postMessage`. Put the trait definition in a shared crate so both ends share it.

```rust
#[web_rpc::service]
pub trait Calculator {
    fn add(&self, left: u32, right: u32) -> u32;
}
```

The macro generates `CalculatorClient`, `CalculatorService`, a `Calculator` trait you implement on the server side, and a `CALCULATOR_DESCRIPTION` const describing the trait:

```rust
struct CalculatorImpl;
impl Calculator for CalculatorImpl {
    fn add(&self, left: u32, right: u32) -> u32 { left + right }
}
```

Wire up over a `MessageChannel` (or a `Worker` / `MessagePort`):

```rust
let channel = web_sys::MessageChannel::new().unwrap();
// web-rpc uses the transport it is handed and never manages its lifecycle: start the ports
// yourself, and terminate your own workers.
channel.port1().start();
channel.port2().start();

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
- **Explicit routing**: `Post<T>` crosses as a JavaScript value copied by structured clone, `Transfer<T>` crosses as one that is moved onto the `postMessage` transfer list. Everything else is postcard-encoded and must implement `postcard_schema::Schema`.
- **`Option`/`Result` wrappers (and nested)**: each variant routes independently, so `Result<Transfer<ArrayBuffer>, RustError>` and `Result<Option<Post<JsT>>, _>` just work, with no attribute.
- **Bidirectional RPC** over a single channel, with both ends simultaneously acting as client and server.
- **Streaming RPCs** via `impl Stream<Item = T>` returns, with abort-on-drop and close-and-drain.
- **Async or sync** server methods, with per-request cancellation when the client drops the future of an RPC method that returns.
- **Notifications**: methods with no return type are fire-and-forget.
- **Borrowed `&str` / `&[u8]`**: zero-copy deserialization on the server side.
- **Conditional methods**: `#[cfg(...)]` on a trait method is propagated to all generated code, so the method is stripped from the client, server, description and wire format when the cfg is off.
- **Generated JavaScript endpoints**: `js::endpoint!` renders a typed `.mjs` and `.d.ts` for the other end of a connection, at compile time, from the same traits.

## JavaScript endpoints

`js::endpoint!` is the JavaScript counterpart of `Builder` and reads the same way: `service =` is the trait the generated endpoint **serves**, `client =` the trait it **calls**. Place it in the binary crate that owns the transport, beside the Rust builder it mirrors:

```rust
// The Rust side of this binary.
Builder::new(iface)
    .with_service::<CalculatorService<_>>(calculator)
    .with_client::<DisplayClient>();
// The other end of the same connection, described as itself.
web_rpc::js::endpoint!(service = DisplayService, client = CalculatorClient);
```

The class is named after `client =`, or after `service =` when there is no client, and is handed its transport rather than creating one:

```js
import { CalculatorClient } from './calculator_client.mjs';

const worker = new Worker(url, { type: 'module' });
const calculator = new CalculatorClient({ endpoint: worker, handlers: { /* the Display trait */ } });

await calculator.add(41, 1);                       // Request<number>, with .abort()
calculator.note('hello');                          // a notification, returns void
const { close, done } = calculator.items(10, item => console.log(item));
```

The expansion writes the module and its declarations into two custom sections named after the class in snake_case, which survive wasm-bindgen and wasm-opt. Extract them before wasm-bindgen runs, with `llvm-objcopy` or `rust-objcopy` from `cargo-binutils`:

```
llvm-objcopy --dump-section=__web_rpc_calculator_client_js=calculator_client.mjs \
             --dump-section=__web_rpc_calculator_client_d_ts=calculator_client.d.ts in.wasm out.wasm
```

Add `--remove-section=...` for each to strip them from what you ship. The `.mjs` has no imports and needs no bundling: it is a small trait-independent shell plus the traits' types and methods as data, which the shell interprets. Two endpoints may coexist in one binary; two with the same class name are a duplicate-symbol error.

## Migrating from 0.0.7

Both ends must be on 0.0.8: the wire format changed from bincode to postcard. See [WIRE.md](WIRE.md) for the format itself.

- Replace bare JavaScript types in signatures with `Post<T>` or `Transfer<T>`, and delete every `#[transfer(...)]`. The compiler finds all the sites. A typed array is not transferable: send `Transfer<ArrayBuffer>` and rebuild the view on the other side.
- Add `#[derive(Schema)]` to every payload type reachable from a `#[web_rpc::service]` trait, and add `postcard-schema` as a direct dependency, since the derive emits `::postcard_schema::` paths. A foreign type with no upstream impl needs a local mirror type. `usize` and `isize` have no `Schema` impl, because serde widens them to `u64`/`i64` and their Rust-side width is a property of the target; use a fixed-width integer.
- Dropping a `Port` or an `Interface` no longer terminates a `Worker`, and a `MessagePort` is no longer `start()`ed for you, on either side. Call `port.start()` before handing a port over, or the handshake spins forever rather than failing.
- A request or stream now keeps the connection's listener alive on its own, so a client may be dropped while one is still pending.

See the [crate documentation](https://docs.rs/web-rpc/latest/web_rpc/) for the full feature reference. Need help with your latest project? Get in touch via [contact@allwright.io](mailto:contact@allwright.io). I'm available for new assignments.
