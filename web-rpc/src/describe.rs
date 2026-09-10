//! The compile-time description of a service trait.
//!
//! Every `#[web_rpc::service]` trait emits a `&'static Service` next to itself, named after
//! the trait in `SCREAMING_SNAKE_CASE` with a `_DESCRIPTION` suffix (`FooBar` becomes
//! `FOO_BAR_DESCRIPTION`). The description is role-agnostic: the same value describes the
//! trait whether Javascript calls it or implements it. [`crate::js::endpoint`] renders
//! Javascript and Typescript from it at compile time.
//!
//! Every type in a signature must implement either [`postcard_schema::Schema`] (the postcard
//! route) or [`JsName`] (inside [`crate::wrap::Post`] or [`crate::wrap::Transfer`]).

use postcard_schema::schema::NamedType;

/// One `#[web_rpc::service]` trait.
pub struct Service {
    /// The trait's identifier, as written.
    pub name: &'static str,
    /// The trait's methods, one slice per method in declaration order.
    ///
    /// A method gated by a `#[cfg(...)]` that evaluates to false contributes an empty
    /// slice. The method's index on the wire is its position among the non-empty entries,
    /// which is also its variant index in the request enum.
    pub methods: &'static [&'static [Method]],
}

/// One method of a service trait.
pub struct Method {
    /// The method's wire name, in camelCase.
    pub name: &'static str,
    /// The method's arguments, in declaration order.
    pub args: &'static [Arg],
    /// What the method sends back.
    pub ret: Return,
}

/// One argument of a method.
pub struct Arg {
    /// The argument's name, in camelCase.
    pub name: &'static str,
    /// How the argument crosses the channel.
    pub desc: &'static Desc,
}

/// How one value crosses the channel.
///
/// This mirrors [`crate::codec::WireArg`]: the `Option` and `Result` variants describe the
/// wrapper structure that the macro walks, and the leaves say whether the value travels as a
/// Javascript value or as postcard bytes.
pub enum Desc {
    /// A [`crate::wrap::Post`] or [`crate::wrap::Transfer`] leaf.
    Js {
        /// The DOM class name of the wrapped type, from [`JsName`].
        name: &'static str,
        /// Whether the value goes on the transfer list.
        transfer: bool,
    },
    /// A postcard-encoded leaf.
    Postcard(&'static NamedType),
    /// A `&str` or `&[u8]` argument, written into the request payload directly with no
    /// `WireArg` around it.
    Inline(&'static NamedType),
    /// `Option<T>`, whose `Some` and `None` route independently.
    Option(&'static Desc),
    /// `Result<T, E>`, whose `Ok` and `Err` route independently.
    Result(&'static Desc, &'static Desc),
}

/// What a method sends back.
pub enum Return {
    /// No return type: a fire-and-forget notification, with no response message.
    Notify,
    /// A single response.
    Value(&'static Desc),
    /// A stream of items, each in its own message.
    Stream(&'static Desc),
}

/// The DOM class name of a Javascript type, for the generated Typescript declarations.
///
/// Implemented for the `js_sys` types listed below and for the `web_sys` transferable objects.
/// A Javascript type outside that list needs a local newtype implementing this trait, since
/// the orphan rule prevents implementing it downstream for a foreign type.
pub trait JsName {
    /// The name this type has in Typescript.
    const NAME: &'static str;
}

macro_rules! impl_js_name {
    ($($ty:ty => $name:literal),* $(,)?) => {
        $(impl JsName for $ty {
            const NAME: &'static str = $name;
        })*
    };
}

impl_js_name! {
    wasm_bindgen::JsValue => "unknown",
    js_sys::Object => "object",
    js_sys::Array => "unknown[]",
    js_sys::Function => "Function",
    js_sys::Promise => "Promise<unknown>",
    js_sys::JsString => "string",
    js_sys::Error => "Error",
    js_sys::Date => "Date",
    js_sys::RegExp => "RegExp",
    js_sys::Map => "Map<unknown, unknown>",
    js_sys::Set => "Set<unknown>",
    js_sys::ArrayBuffer => "ArrayBuffer",
    js_sys::SharedArrayBuffer => "SharedArrayBuffer",
    js_sys::DataView => "DataView",
    js_sys::Int8Array => "Int8Array",
    js_sys::Uint8Array => "Uint8Array",
    js_sys::Uint8ClampedArray => "Uint8ClampedArray",
    js_sys::Int16Array => "Int16Array",
    js_sys::Uint16Array => "Uint16Array",
    js_sys::Int32Array => "Int32Array",
    js_sys::Uint32Array => "Uint32Array",
    js_sys::Float32Array => "Float32Array",
    js_sys::Float64Array => "Float64Array",
    js_sys::BigInt64Array => "BigInt64Array",
    js_sys::BigUint64Array => "BigUint64Array",
    web_sys::MessagePort => "MessagePort",
    web_sys::OffscreenCanvas => "OffscreenCanvas",
    web_sys::ImageBitmap => "ImageBitmap",
    web_sys::ReadableStream => "ReadableStream",
    web_sys::WritableStream => "WritableStream",
    web_sys::TransformStream => "TransformStream",
}
