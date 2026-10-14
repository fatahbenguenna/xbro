//! HTML parsing for xbro: byte decoding, tokenizer and tree builder.
//!
//! The parser follows the WHATWG HTML parsing algorithm loosely, with
//! scripting disabled (so `<noscript>` content is parsed as markup). It never
//! panics: any input, however malformed, produces a [`Document`] that has an
//! `<html>` element with `<head>` and `<body>` children.
//!
//! The produced tree is never deeper than [`MAX_DEPTH`] elements, which
//! protects the recursive stages downstream (style, layout, paint).

mod decode;
pub mod entities;
pub mod tokenizer;
pub mod tree_builder;

pub use tree_builder::MAX_DEPTH;
pub use xbro_dom::{Attribute, Document, NodeData, NodeId};

/// Parses a complete HTML document.
///
/// The resulting tree always contains `html > head` and `html > body`
/// (implied elements are synthesized), mirroring browsers. A single leading
/// U+FEFF (byte-order mark left in the string) is ignored.
pub fn parse_document(input: &str) -> Document {
    tree_builder::TreeBuilder::new().parse(input)
}

/// Decodes the raw bytes of an HTML resource to a `String`.
///
/// Encoding is chosen, in order, from: a byte-order mark, the `charset`
/// parameter of the HTTP `Content-Type` (`transport_charset`; surrounding
/// whitespace and quotes are ignored), a `<meta charset>` /
/// `<meta http-equiv="content-type">` found by prescanning the first 1024
/// bytes, and finally UTF-8 (when the bytes are valid UTF-8, a multi-byte
/// sequence cut off at the very end being tolerated; otherwise
/// windows-1252, as browsers do for legacy pages). Invalid sequences are
/// replaced with U+FFFD.
pub fn decode_html_bytes(bytes: &[u8], transport_charset: Option<&str>) -> String {
    tree_builder::decode_bytes(bytes, transport_charset)
}
