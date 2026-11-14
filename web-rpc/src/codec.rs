//! The on-wire representation of a single argument or return value.
//!
//! The macro walks each signature type syntactically. A type with no
//! [`crate::wrap::Post`] or [`crate::wrap::Transfer`] anywhere inside it is postcard-encoded
//! whole into [`WireArg::Bytes`]. Otherwise `Option`, `Result`, `Vec`, arrays and the values
//! of `HashMap` and `BTreeMap` recurse into the matching [`WireArg`] variants until they
//! reach a `Post` or `Transfer`, which becomes [`WireArg::Js`] and takes the next slot of the
//! message array, or a subtree free of them, which is postcard-encoded.

use serde::{Deserialize, Serialize};

/// On-wire representation of a single argument or return value.
///
/// A value of type `Result<Vec<Post<JsString>>, MyError>` round-trips as
/// `WireArg::Ok(WireArg::Seq([WireArg::Js, ..]))` or `WireArg::Err(WireArg::Bytes(_))`,
/// while `Result<u32, MyError>` is a single `WireArg::Bytes(_)`.
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
    /// The items of a `Vec` or an array, in order.
    Seq(Vec<WireArg>),
    /// The entries of a map, each a postcard-encoded key and its value.
    Map(Vec<(Vec<u8>, WireArg)>),
}
