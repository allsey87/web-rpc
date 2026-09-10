//! Explicit routing wrappers.
//!
//! A value in an RPC signature crosses the channel either as postcard bytes inside the
//! payload, or as a Javascript value in the message array. The wrapper picks the route:
//! [`Post`] and [`Transfer`] take the Javascript path, everything else is postcard-encoded
//! and must implement [`postcard_schema::Schema`].
//!
//! [`Transfer`] additionally puts the value on the `postMessage` transfer list, so it moves
//! rather than being copied by the structured clone algorithm.
//!
//! No trait constrains what may be transferred. `T` must be a transferable object as defined
//! by the structured clone algorithm (`ArrayBuffer`, `MessagePort`, `OffscreenCanvas`,
//! `ImageBitmap`, the stream types, ...); anything else is a `DataCloneError` thrown by the
//! browser at the moment of sending. A typed array is not transferable: send
//! `Transfer<ArrayBuffer>` and rebuild the view on the other side. A view over wasm linear
//! memory can never be transferred, so [`Post`] it instead.
//!
//! A bare Javascript type in a signature does not compile:
//!
//! ```compile_fail
//! #[web_rpc::service]
//! pub trait Echo {
//!     fn echo(&self, value: js_sys::JsString) -> js_sys::JsString;
//! }
//! ```
//!
//! A payload type without `#[derive(Schema)]` is rejected at the argument that uses it:
//!
//! ```compile_fail
//! #[derive(serde::Serialize, serde::Deserialize)]
//! pub struct Point { x: u32 }
//!
//! #[web_rpc::service]
//! pub trait Plot {
//!     fn plot(&self, point: Point);
//! }
//! ```

use std::ops::Deref;

use wasm_bindgen::JsCast;

/// Wrapper that routes `T` across the channel as a Javascript value.
///
/// ```rust
/// # use web_rpc::wrap::Post;
/// #[web_rpc::service]
/// pub trait Echo {
///     fn echo(&self, value: Post<js_sys::JsString>) -> Post<js_sys::JsString>;
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Post<T: JsCast>(pub T);

/// Wrapper that routes `T` across the channel as a Javascript value and puts it on the
/// transfer list, moving it out of the sending context.
///
/// ```rust
/// # use web_rpc::wrap::Transfer;
/// #[web_rpc::service]
/// pub trait Upload {
///     fn upload(&self, buffer: Transfer<js_sys::ArrayBuffer>) -> u32;
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer<T: JsCast>(pub T);

macro_rules! impl_wrapper {
    ($wrapper:ident) => {
        impl<T: JsCast> $wrapper<T> {
            /// Wrap a value.
            pub fn new(value: T) -> Self {
                Self(value)
            }

            /// Unwrap, returning the inner Javascript value.
            pub fn into_inner(self) -> T {
                self.0
            }
        }

        impl<T: JsCast> Deref for $wrapper<T> {
            type Target = T;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl<T: JsCast> From<T> for $wrapper<T> {
            fn from(value: T) -> Self {
                Self(value)
            }
        }

        impl<T: JsCast> AsRef<T> for $wrapper<T> {
            fn as_ref(&self) -> &T {
                &self.0
            }
        }
    };
}

impl_wrapper!(Post);
impl_wrapper!(Transfer);

/// Borrow a Javascript value as a [`JsValue`](wasm_bindgen::JsValue).
///
/// The `js_sys` and `web_sys` types implement `AsRef` for their whole prototype chain; this
/// selects the `JsValue` impl.
#[doc(hidden)]
pub fn js_value<T: JsCast>(value: &T) -> &wasm_bindgen::JsValue {
    value.as_ref()
}
