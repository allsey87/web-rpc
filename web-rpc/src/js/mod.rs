//! Javascript endpoints rendered from service traits at compile time.
//!
//! [`endpoint!`](macro@endpoint) is the Javascript counterpart of [`crate::Builder`] and reads
//! the same way: `service =` names the trait the generated endpoint **serves**, `client =` the
//! trait it **calls**, and the values are the same generated struct names one would pass to
//! [`with_service`](crate::Builder::with_service) and
//! [`with_client`](crate::Builder::with_client). At least one is required.
//!
//! ```rust,ignore
//! // The Rust side of this binary.
//! Builder::new(iface)
//!     .with_service::<CalculatorService<_>>(calculator)
//!     .with_client::<DisplayClient>();
//! // The other end of the same connection, described as itself.
//! web_rpc::js::endpoint!(service = DisplayService, client = CalculatorClient);
//! ```
//!
//! The expansion writes a `.mjs` and a `.d.ts` into two custom sections of the wasm binary,
//! named after the class in snake_case: `__web_rpc_calculator_client_js` and
//! `__web_rpc_calculator_client_d_ts` for the example above. Extract them with `llvm-objcopy`
//! (or `rust-objcopy` from `cargo-binutils`), before wasm-bindgen runs:
//!
//! ```text
//! llvm-objcopy --dump-section=__web_rpc_calculator_client_js=calculator_client.mjs \
//!              --dump-section=__web_rpc_calculator_client_d_ts=calculator_client.d.ts \
//!              in.wasm out.wasm
//! ```
//!
//! Add `--remove-section=...` for each to strip them from what you ship. The `.mjs` has no
//! imports and needs no bundling.
//!
//! The generated module is the shell in `js/endpoint.mjs`, which is trait-independent,
//! followed by data: a schema value per type the traits reach, a method table per trait, and
//! a class whose methods forward to the shell. Encoding and decoding are interpreted from
//! those values by the shell's `Codec`, which the module also exports along with `Writer` and
//! `Reader`, for an embedder that wants to speak postcard itself.
//!
//! One caveat follows from how `#[link_section]` works on wasm: the macro must be invoked in
//! the binary crate that is linked into the wasm module, because a static in an rlib that
//! contributes no symbol to the link is dropped by wasm-ld.
//!
//! # What the renderers reject
//!
//! Rendering happens during const evaluation, which cannot format a panic message, so the
//! compiler's const-eval backtrace is what points at the offending type. An enum whose struct
//! variant has a field named `tag` would collide with the discriminant of the Typescript union
//! that represents it:
//!
//! ```compile_fail
//! #[derive(serde::Serialize, serde::Deserialize, postcard_schema::Schema)]
//! pub enum Bad {
//!     Variant { tag: u32 },
//! }
//!
//! #[web_rpc::service]
//! pub trait Uses {
//!     fn take(&self, value: Bad);
//! }
//!
//! web_rpc::js::endpoint!(client = UsesClient);
//! ```
//!
//! So would two types that render to the same Typescript name:
//!
//! ```compile_fail
//! #[derive(serde::Serialize, serde::Deserialize, postcard_schema::Schema)]
//! pub struct Alpha { pub x: u32 }
//!
//! #[derive(serde::Serialize, serde::Deserialize, postcard_schema::Schema)]
//! #[serde(rename = "Alpha")]
//! pub struct Beta { pub y: String }
//!
//! #[web_rpc::service]
//! pub trait Uses {
//!     fn one(&self, value: Alpha);
//!     fn two(&self, value: Beta);
//! }
//!
//! web_rpc::js::endpoint!(client = UsesClient);
//! ```
//!
//! And so would a type named `Request`, `Subscription` or `Endpoint`, which the generated
//! declarations define themselves:
//!
//! ```compile_fail
//! #[derive(serde::Serialize, serde::Deserialize, postcard_schema::Schema)]
//! pub struct Request { pub id: u32 }
//!
//! #[web_rpc::service]
//! pub trait Uses {
//!     fn one(&self, value: Request);
//! }
//!
//! web_rpc::js::endpoint!(client = UsesClient);
//! ```

use crate::describe::{Method, Service};

mod code;
mod decls;
mod dts;
mod writer;

pub use decls::MAX_DECLARATIONS;
pub use web_rpc_macro::endpoint;
pub use writer::Output;

/// The trait-independent part of every generated endpoint, emitted ahead of the rendered
/// schemas, method tables and class.
pub const SHELL: &str = include_str!("../../js/endpoint.mjs");

/// What the macro renders: a class name and the traits filling each half of the connection.
pub struct Endpoint {
    /// The name of the generated Javascript class.
    pub class: &'static str,
    /// The trait this endpoint implements, if any.
    pub service: Option<&'static Service>,
    /// The trait this endpoint calls, if any.
    pub client: Option<&'static Service>,
}

/// The number of methods of a service that survived cfg evaluation.
pub const fn method_count(service: &Service) -> usize {
    let mut count = 0;
    let mut group = 0;
    while group < service.methods.len() {
        count += service.methods[group].len();
        group += 1;
    }
    count
}

/// The `index`th enabled method of a service. Its position here is its index on the wire.
pub const fn method_at(service: &Service, index: usize) -> &'static Method {
    let mut seen = 0;
    let mut group = 0;
    while group < service.methods.len() {
        let methods = service.methods[group];
        if index < seen + methods.len() {
            return &methods[index - seen];
        }
        seen += methods.len();
        group += 1;
    }
    panic!("web_rpc: method index out of range")
}

/// Render the Javascript module for one endpoint.
///
/// Call once with `CAPACITY = 0` to measure, then again with `CAPACITY` set to the measured
/// length.
pub const fn render_js<const CAPACITY: usize>(endpoint: &Endpoint) -> Output<CAPACITY> {
    code::render(endpoint)
}

/// Render the Typescript declarations for one endpoint.
///
/// Call once with `CAPACITY = 0` to measure, then again with `CAPACITY` set to the measured
/// length.
pub const fn render_dts<const CAPACITY: usize>(endpoint: &Endpoint) -> Output<CAPACITY> {
    dts::render(endpoint)
}
