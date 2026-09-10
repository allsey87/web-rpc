# The web-rpc wire format

Derived from the Rust types in `web-rpc/src/lib.rs`, `web-rpc/src/codec.rs` and the enums
`#[web_rpc::service]` generates, so that a Javascript implementation is written against the
source of truth rather than against a description of it. `web-rpc/tests/wire.rs` asserts the
byte strings below.

Payloads are [postcard](https://postcard.jamesmunns.com/wire-format) 1.x, wire format
specification 1.0. Both ends of a channel must be on the same version of web-rpc: the format
changed in 0.0.8, which replaced bincode with postcard.

## Messages

A message is one Javascript array, posted with a transfer list:

```
[ header: ArrayBuffer, payload: ArrayBuffer?, ...jsValues ]
```

`header` is the postcard encoding of `MessageHeader`, whose declaration order is its tag:

| Variant | Tag | Payload buffer | Meaning |
|:--|:-:|:-:|:--|
| `Request(seq)` | 0 | yes | Call a method, or start a stream. |
| `Abort(seq)` | 1 | no | Cancel a request, or stop a stream. |
| `Response(seq)` | 2 | yes | The single result of a request. |
| `StreamItem(seq)` | 3 | yes | One item of a stream. |
| `StreamEnd(seq)` | 4 | no | No more items will follow. |

So a header is `varint(u32 tag) varint(u32 seq)`, at most six bytes. `seq` is allocated by
the caller and wraps; the two directions of a bidirectional connection allocate independently,
so a sequence number only identifies a message within the direction it travels.

`jsValues` holds, positionally and in encounter order, the values that the payload marks as
living on the Javascript side. A `Response` and a `StreamItem` each carry their own slots and
their own transfer list, exactly like a `Request`.

## Payloads

The payload of a `Request` is the generated `<Trait>Request` enum: `varint(u32 method index)`
followed by one `WireArg` per argument, in declaration order. A `&str` or `&[u8]` argument is
the exception: it is written directly, as `varint(usize len)` then the bytes, with no `WireArg`
around it. The payload of a `Response` or a
`StreamItem` is the generated `<Trait>Response` enum: `varint(u32 method index)` followed by
one `WireArg`.

The method index is the method's position among the variants that survive cfg evaluation, so
a `#[cfg]` that is off on one end and on at the other silently shifts every later method. Both
ends must agree.

A method with no return type is a notification: it is sent as a `Request` and no `Response`
ever comes back.

## `WireArg`

Every argument and every returned value is a `WireArg`, whose declaration order is its tag:

| Variant | Tag | Body |
|:--|:-:|:--|
| `Js` | 0 | none; the value is the next unconsumed slot of `jsValues` |
| `Bytes(Vec<u8>)` | 1 | `varint(usize len)` then `len` bytes: the postcard encoding of the value |
| `None` | 2 | none |
| `Some(Box<WireArg>)` | 3 | one `WireArg` |
| `Ok(Box<WireArg>)` | 4 | one `WireArg` |
| `Err(Box<WireArg>)` | 5 | one `WireArg` |

`Post<T>` and `Transfer<T>` produce `Js`; `Transfer<T>` additionally puts the value on the
message's transfer list. `Option` and `Result` produce the wrapper variants and recurse, so
each of their variants routes independently: `Result<Option<Post<JsString>>, MyError>` is
`Ok(Some(Js))` or `Err(Bytes(..))`. Anything else produces `Bytes`.

`Bytes` nests one length prefix inside another, since the payload enum that contains it is
itself length-delimited by the message. That is accepted rather than optimised away, so that
`WireArg` stays a plain serde enum.

## Postcard rules used

- `bool`: one byte, `00` or `01`.
- `u8`, `i8`: one byte. `i8` is the raw two's complement byte, not zigzagged.
- `u16`, `u32`, `u64`, `u128`: LEB128 varint, little-endian groups of seven bits with the high
  bit set on every byte but the last.
- `i16`, `i32`, `i64`, `i128`: zigzag (`(n << 1) ^ (n >> bits - 1)`) then varint.
- `usize`, `isize`: serde has no `usize` in its data model and widens them, so these encode
  exactly as `u64` and `i64` do, on every target.
- `f32`, `f64`: 4 or 8 bytes, IEEE-754 little-endian.
- `char`: encoded as a one-character string.
- `String`, `&str`, byte arrays, sequences, maps: `varint(usize len)` then the contents; a map
  writes each key immediately before its value.
- `Option`: `00`, or `01` followed by the value. (This is postcard's own `Option`, used inside
  a payload type. An `Option` in a *signature* becomes `WireArg::None`/`WireArg::Some`.)
- Enums: `varint(u32 variant index)` then the variant's fields.
- Structs, tuples, tuple structs: their fields in order, with nothing in between.
- Unit, unit structs: nothing at all.
- Newtype structs: transparent, exactly their inner value.

A sequence, map or string length is the one place postcard writes a `usize` itself rather than
through serde, and it varints the value at the target's own width. On `wasm32` that is 32 bits,
so a length never exceeds five bytes. The Javascript reader relies on that bound.

postcard-schema implements `Schema` for neither `usize` nor `isize`, so a signature needs a
fixed-width integer. That follows from the widening above: a `usize` field is a `u64` on the
wire, and there is no honest schema for a Rust type whose width is a property of the target.

## Handshake

Symmetric, and unchanged since 0.0.7. Each side posts `null` every 10 ms until a non-array
message arrives. On any non-array message it marks itself ready and posts `null` once more, so
that a peer which has not yet seen it does. Every array received before that point is a real
message and is queued until the handshake completes.

A `MessagePort` must be `start()`ed by its owner before either side is handed it. web-rpc does
not start it, on either the Rust or the Javascript side, and an unstarted port delivers nothing
to a listener, so the symptom is a handshake that spins rather than an error.

## Stream lifecycle

The caller posts an ordinary `Request(seq)`. The producer posts `StreamItem(seq)` per item and
`StreamEnd(seq)` when it finishes. The caller cancels with `Abort(seq)`, after which the
producer stops and **still** posts `StreamEnd(seq)`; items already in flight may arrive between
the abort and the end, which is what lets a caller close a stream and drain it.
