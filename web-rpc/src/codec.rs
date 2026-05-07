//! Wire-format primitives and autoref-based dispatch for JS-vs-serialize routing.
//!
//! The macro emits calls into this module to encode/decode RPC arguments and return
//! values. Two pairs of traits drive the decision at compile time via the
//! [autoref-based stable specialization] pattern: a value is routed through the JS
//! post-array if it implements `AsRef<JsValue>`, or through bincode otherwise.
//!
//! [autoref-based stable specialization]: https://lukaskalbertodt.github.io/2019/12/05/generalized-autoref-based-specialization.html

use std::marker::PhantomData;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use wasm_bindgen::{JsCast, JsValue};

/// On-wire representation of a single argument or return value.
///
/// `Js` indicates the value lives at the next slot of the post-array (positional).
/// `Bytes` carries the bincoded value inline. The `Some`/`None`/`Ok`/`Err` variants
/// carry the inner wrapper structure recursively, so a value of type
/// `Result<Option<JsT>, RustErr>` round-trips as
/// `WireArg::Ok(WireArg::Some(WireArg::Js))` or `WireArg::Err(WireArg::Bytes(_))`.
#[derive(Serialize, Deserialize)]
pub enum WireArg {
    /// Value lives at the next slot of the post-array.
    Js,
    /// Bincoded payload, inline.
    Bytes(Vec<u8>),
    /// `Option::None`.
    None,
    /// `Option::Some(_)`.
    Some(Box<WireArg>),
    /// `Result::Ok(_)`.
    Ok(Box<WireArg>),
    /// `Result::Err(_)`.
    Err(Box<WireArg>),
    /// Reserved for future `Vec<T>`/array/tuple support; not yet emitted by the macro.
    Seq(Vec<WireArg>),
}

// ---------------------------------------------------------------------------
// Encoder side — autoref dispatch
// ---------------------------------------------------------------------------

/// Higher-priority encode path: types that can be referenced as a `JsValue`
/// are pushed onto the post-array verbatim and represented as [`WireArg::Js`].
///
/// Resolved at autoref level 0 for receiver `&T`. Method name is intentionally
/// dunder-prefixed to avoid collisions with user traits in scope.
#[doc(hidden)]
pub trait __RpcJsEncode {
    fn __rpc_encode(self, post: &js_sys::Array) -> WireArg;
}

impl<T: AsRef<JsValue> + ?Sized> __RpcJsEncode for &T {
    fn __rpc_encode(self, post: &js_sys::Array) -> WireArg {
        post.push(self.as_ref());
        WireArg::Js
    }
}

/// Lower-priority encode path: types that implement `Serialize` are bincoded
/// inline and represented as [`WireArg::Bytes`].
///
/// Resolved at autoref level 1 (`&&T`). The method name matches `__RpcJsEncode`
/// so that method dispatch picks one or the other at the call site.
#[doc(hidden)]
pub trait __RpcSerialEncode {
    fn __rpc_encode(self, post: &js_sys::Array) -> WireArg;
}

impl<T: Serialize + ?Sized> __RpcSerialEncode for &&T {
    fn __rpc_encode(self, _post: &js_sys::Array) -> WireArg {
        WireArg::Bytes(bincode::serialize(self).unwrap())
    }
}

// ---------------------------------------------------------------------------
// Decoder side — autoref dispatch
// ---------------------------------------------------------------------------

/// Type-tag used by the macro to drive decoder-side autoref dispatch.
///
/// The macro emits `(&Decoder::<T>::default()).__rpc_decode(wire, post)`. The
/// `T` parameter is the desired output type; the candidate-list autoref then
/// selects between [`__RpcJsDecode`] and [`__RpcSerialDecode`] based on the
/// trait bounds satisfied by `T`.
#[doc(hidden)]
pub struct Decoder<T: ?Sized>(PhantomData<T>);

impl<T: ?Sized> Default for Decoder<T> {
    fn default() -> Self {
        Decoder(PhantomData)
    }
}

/// Higher-priority decode path: types that can be cast from `JsValue` are
/// extracted from the post-array via `JsCast::dyn_into`.
#[doc(hidden)]
pub trait __RpcJsDecode<T> {
    fn __rpc_decode(self, wire: WireArg, post: &js_sys::Array) -> T;
}

impl<T: JsCast> __RpcJsDecode<T> for &Decoder<T> {
    fn __rpc_decode(self, wire: WireArg, post: &js_sys::Array) -> T {
        match wire {
            WireArg::Js => post.shift().dyn_into::<T>().unwrap(),
            _ => panic!("web_rpc: wire/type mismatch — expected Js variant"),
        }
    }
}

/// Lower-priority decode path: types that implement `DeserializeOwned` are
/// reconstructed from the inline bincoded bytes.
#[doc(hidden)]
pub trait __RpcSerialDecode<T> {
    fn __rpc_decode(self, wire: WireArg, post: &js_sys::Array) -> T;
}

impl<T: DeserializeOwned> __RpcSerialDecode<T> for &&Decoder<T> {
    fn __rpc_decode(self, wire: WireArg, _post: &js_sys::Array) -> T {
        match wire {
            WireArg::Bytes(b) => bincode::deserialize(&b).unwrap(),
            _ => panic!("web_rpc: wire/type mismatch — expected Bytes variant"),
        }
    }
}
