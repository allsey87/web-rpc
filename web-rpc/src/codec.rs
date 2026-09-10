//! The on-wire representation of a single argument or return value.
//!
//! The macro walks each signature type syntactically: `Option` and `Result` recurse into
//! the matching [`WireArg`] variants, a [`crate::wrap::Post`] or [`crate::wrap::Transfer`]
//! leaf becomes [`WireArg::Js`] and takes the next slot of the message array, and any other
//! leaf is postcard-encoded into [`WireArg::Bytes`].

use serde::{Deserialize, Serialize};

/// On-wire representation of a single argument or return value.
///
/// A value of type `Result<Option<Post<JsString>>, MyError>` round-trips as
/// `WireArg::Ok(WireArg::Some(WireArg::Js))` or `WireArg::Err(WireArg::Bytes(_))`.
#[derive(Serialize, Deserialize)]
pub enum WireArg {
    /// Value lives at the next slot of the message array.
    Js,
    /// Postcard-encoded payload, inline.
    Bytes(Vec<u8>),
    /// `Option::None`.
    None,
    /// `Option::Some(_)`.
    Some(Box<WireArg>),
    /// `Result::Ok(_)`.
    Ok(Box<WireArg>),
    /// `Result::Err(_)`.
    Err(Box<WireArg>),
}
