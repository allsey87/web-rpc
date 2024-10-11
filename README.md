[![CI](https://github.com/allsey87/web-rpc/actions/workflows/test.yaml/badge.svg)](https://github.com/allsey87/web-rpc/actions)
[![Crates.io](https://img.shields.io/crates/v/web-rpc.svg)](https://crates.io/crates/web-rpc)
[![api-docs](https://docs.rs/web-rpc/badge.svg)](https://docs.rs/web-rpc/)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

# web-rpc
*WARNING: This crate is still being developed and is not ready for production use.*

## Quick start
`web-rpc` is a crate for defining RPCs over browsing contexts, web workers, and channels. The crate exports a proc macro called `service` that will consume a trait as follows:
```rust
#[web_rpc::service]
pub trait Calculator {
    fn add(left: u32, right: u32) -> u32;
}
```
This macro is based on Google's [tarpc](https://github.com/google/tarpc) and will generate the structs `CalculatorClient`, `CalculatorService`, and a new trait `Calculator` that you can use to implement a calculator as follows:
```rust
struct CalculatorServiceImpl;

impl Calculator for CalculatorServiceImpl {
    fn add(&self, left: u32, right: u32) -> u32 {
        left + right
    }
}
```
Note that the version of the trait emitted from the macro adds a `&self` receiver. Although not used in this example, this is useful when we want the RPC to modify some state (via interior mutability). Now that we have defined our RPC, let's create a client and server for it! In this example, we will use [`MessageChannel`](https://docs.rs/web-sys/latest/web_sys/struct.MessageChannel.html) since it is easy to construct and test, however, a more common case would be to construct the channel from a [`Worker`](https://docs.rs/web-sys/latest/web_sys/struct.Worker.html) and a [`DedicatedWorkerGlobalScope`](https://docs.rs/web-sys/latest/web_sys/struct.DedicatedWorkerGlobalScope.html). Let's start with the server:
```rust
// create a MessageChannel
let channel = web_sys::MessageChannel::new();
// Create two interfaces from the ports. web_rpc::Interface::new is an async method that
// will return once the other end is ready, hence we need to poll both at the same time
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
`web_rpc::Interface::new` is an async method since there is no way to synchronously check whether a channel is ready to receive messages. To workaround this, temporary listeners are attached to determine when a channel is ready for communication. The output of this method is a future that can be added to the browser's event loop using [`wasm_bindgen_futures::spawn_local`](https://docs.rs/wasm-bindgen-futures/latest/wasm_bindgen_futures/fn.spawn_local.html), however, this will run the server indefinitely. For more control, consider wrapping the server with [`FutureExt::remote_handle`](https://docs.rs/futures/latest/futures/future/trait.FutureExt.html#method.remote_handle) before spawning it, which will shutdown the server once the handle has been dropped. Let's move on the client:
```rust
// create a client using the second port
let client = web_rpc::Builder::new(client_interface)
    .with_client::<CalculatorClient>()
    .build();
/* call `add` */
assert_eq!(client.add(41, 1).await, 42);
```
That is it! Underneath the hood, the client will serialize its arguments and transfer the bytes to server. The server will deserialize those arguments and run `<CalculatorServiceImpl as Calculator>::add` before returning the result to the client. Note that we are only awaiting the response of the call to `add`, the request itself is sent synchronously before we await anything.

## Advanced topics
Now that we have the basic idea of how define an RPC trait and set up a server and client, let's dive into some of the more advanced features of this library!

### Synchronous and asynchronous RPC methods
Server methods can be asynchronous! That is, you can define the following RPC trait and service implementation:

```rust
#[web_rpc::service]
pub trait Sleep {
    async fn sleep(interval: Duration);
}

struct SleepServiceImpl;
impl Sleep for SleepServiceImpl {
    async fn sleep(&self, interval: Duration) -> bool {
        gloo_timers::future::sleep(interval).await;
        // sleep completed (was not cancelled)
        true
    }
}
```
Asynchronous RPC methods are run concurrently on the server and also support cancellation if the future on the client side is dropped. However, such a future is only returned from a client method if the RPC returns a value. Otherwise the RPC is considered a notification.

### Notifications
Notifications are RPCs that do not return anything. On the client side, the method is completely synchronous and also returns nothing. This setup is useful if you need to communicate with another part of your application but cannot yield to the event loop.

The implication of this, however, is that even if the server method is asynchronous, we are unable to cancel it from the client side since we do not have a future that can be dropped.

### Working with web types
In the example above, we discussed how the client serializes its arguments before sending them to the server. This approach is convenient, but how do send web types such as a `WebAssembly.Module` or an `OffscreenCanvas` that have no serializable representation? Well, we are in luck since this happens to be one of the key features of this crate. Consider the following RPC trait:
```rust
#[web_rpc::service]
pub trait Concat {
    #[post(left, right, return)]
    fn concat_with_space(
        left: js_sys::JsString,
        right: js_sys::JsString
    ) -> js_sys::JsString;
}
```
All we have done is added the `post` attribute to the method and listed the arguments that we would like to be posted to the other side. Under the hood, the implementation of the client will then skip these arguments during serialization and just append them after the serialized message to the array that will be posted. As shown above, this also works for the return type by just specifying `return` in the post list. For web types that need to be transferred, we simply wrap them in `transfer` as follows:
```rust
#[web_rpc::service]
pub trait GameEngine {
    #[post(transfer(canvas))]
    fn send_canvas(
        canvas: js_sys::OffscreenCanvas,
    );
}
```
### Bi-directional RPC
In the original example, we created a server on the first port of the message channel and a client on the second port. However, it is possible to define both a client and a server on each side, enabling full-duplex RPC. This useful if we have two actors that are communicating with each other and do not want to have to poll one side from the other side. The original example can be extended as follows:
```rust
/* create channel */
let channel = web_sys::MessageChannel::new().unwrap();
let (interface1, interface2) = futures_util::future::join(
    web_rpc::Interface::new(channel.port1()),
    web_rpc::Interface::new(channel.port2()),
).await;
/* create server1 and client1 */
let (client1, server1) = web_rpc::Builder::new(interface1)
    .with_service::<CalculatorService<_>>(CalculatorServiceImpl)
    .with_client::<CalculatorClient>()
    .build();
/* create server2 and client2 */
let (client2, server2) = web_rpc::Builder::new(interface2)
    .with_service::<CalculatorService<_>>(CalculatorServiceImpl)
    .with_client::<CalculatorClient>()
    .build();
```



