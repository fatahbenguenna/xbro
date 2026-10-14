//! Character encoding detection and decoding of HTML bytes.
//!
//! Order of precedence: byte-order mark, transport-layer charset (HTTP
//! `Content-Type`), `<meta>` prescan of the first 1024 bytes, then UTF-8
//! when the bytes are valid UTF-8 (tolerating a sequence truncated at the
//! very end) and windows-1252 otherwise. Decoding is lossy and never fails.

use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8, WINDOWS_1252, X_USER_DEFINED};

/// Number of bytes examined by the `<meta>` prescan.
const PRESCAN_LIMIT: usize = 1024;

/// Decodes `bytes` to a string, detecting the encoding as described in the
/// module documentation.
pub(crate) fn decode_bytes(bytes: &[u8], transport_charset: Option<&str>) -> String {
    let (encoding, body) = detect(bytes, transport_charset);
    decode_with(encoding, body)
}

/// Returns the encoding to use and the bytes to decode (without the BOM).
pub(crate) fn detect<'b>(
    bytes: &'b [u8],
    transport_charset: Option<&str>,
) -> (&'static Encoding, &'b [u8]) {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return (UTF_8, rest);
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        return (UTF_16LE, rest);
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        return (UTF_16BE, rest);
    }
    if let Some(encoding) = transport_charset.and_then(transport_encoding) {
        return (encoding, bytes);
    }
    if let Some(encoding) = prescan(bytes) {
        return (encoding, bytes);
    }
    if looks_like_utf8(bytes) {
        (UTF_8, bytes)
    } else {
        (WINDOWS_1252, bytes)
    }
}

/// Resolves a transport-layer charset label. Surrounding whitespace and
/// quotes (`charset="utf-8"` is valid in a `Content-Type` header) are
/// ignored.
fn transport_encoding(label: &str) -> Option<&'static Encoding> {
    let label = label.trim();
    let label = label
        .strip_prefix('"')
        .and_then(|l| l.strip_suffix('"'))
        .or_else(|| label.strip_prefix('\'').and_then(|l| l.strip_suffix('\'')))
        .unwrap_or(label);
    Encoding::for_label(label.trim().as_bytes())
}

/// True when `bytes` are valid UTF-8, tolerating an incomplete multi-byte
/// sequence at the very end (a document cut off mid-character is still
/// UTF-8; only that tail becomes U+FFFD).
fn looks_like_utf8(bytes: &[u8]) -> bool {
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    }
}

fn decode_with(encoding: &'static Encoding, bytes: &[u8]) -> String {
    if encoding == UTF_8 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let (text, _had_errors) = encoding.decode_without_bom_handling(bytes);
    text.into_owned()
}

fn is_space(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ')
}

/// The "prescan a byte stream to determine its encoding" algorithm
/// (simplified): finds `<meta charset>` or `<meta http-equiv=content-type
/// content="...charset=...">`, skipping comments and other tags.
pub(crate) fn prescan(bytes: &[u8]) -> Option<&'static Encoding> {
    let b = bytes.get(..bytes.len().min(PRESCAN_LIMIT)).unwrap_or(bytes);
    let mut pos = 0;
    while pos < b.len() {
        let rest = b.get(pos..).unwrap_or(&[]);
        if rest.starts_with(b"<!--") {
            // Skip to the end of the comment ("<!-->" counts).
            pos = find(b, pos + 2, b"-->").map_or(b.len(), |p| p + 3);
        } else if rest.len() > 5
            && rest[..5].eq_ignore_ascii_case(b"<meta")
            && (is_space(rest[5]) || rest[5] == b'/')
        {
            pos += 5;
            if let Some(encoding) = meta_charset(b, &mut pos) {
                return Some(encoding);
            }
        } else if rest.first() == Some(&b'<')
            && (rest.get(1).is_some_and(u8::is_ascii_alphabetic)
                || (rest.get(1) == Some(&b'/') && rest.get(2).is_some_and(u8::is_ascii_alphabetic)))
        {
            // Skip the tag name, then its attributes.
            while pos < b.len() && !is_space(b[pos]) && b[pos] != b'>' {
                pos += 1;
            }
            while get_attribute(b, &mut pos).is_some() {}
            pos += 1;
        } else if rest.starts_with(b"<!") || rest.starts_with(b"</") || rest.starts_with(b"<?") {
            pos = b
                .get(pos..)
                .and_then(|r| r.iter().position(|&c| c == b'>'))
                .map_or(b.len(), |p| pos + p + 1);
        } else {
            pos += 1;
        }
    }
    None
}

/// Processes the attributes of a `<meta` tag (position just after `<meta`).
fn meta_charset(b: &[u8], pos: &mut usize) -> Option<&'static Encoding> {
    let mut seen: Vec<Vec<u8>> = Vec::new();
    let mut got_pragma = false;
    let mut need_pragma: Option<bool> = None;
    let mut charset: Option<&'static Encoding> = None;
    while let Some((name, value)) = get_attribute(b, pos) {
        if seen.contains(&name) {
            continue;
        }
        match name.as_slice() {
            b"http-equiv" => {
                if value == b"content-type" {
                    got_pragma = true;
                }
            }
            b"content" => {
                if charset.is_none() {
                    if let Some(enc) = charset_from_content(&value) {
                        charset = Some(enc);
                        need_pragma = Some(true);
                    }
                }
            }
            b"charset" => {
                charset = Encoding::for_label(&value);
                need_pragma = Some(false);
            }
            _ => {}
        }
        seen.push(name);
    }
    let need_pragma = need_pragma?;
    if need_pragma && !got_pragma {
        return None;
    }
    let encoding = charset?;
    Some(if encoding == UTF_16BE || encoding == UTF_16LE {
        UTF_8
    } else if encoding == X_USER_DEFINED {
        WINDOWS_1252
    } else {
        encoding
    })
}

/// The prescan "get an attribute" algorithm. Names and values are
/// lowercased. Returns `None` at `>` or at the end of the input.
fn get_attribute(b: &[u8], pos: &mut usize) -> Option<(Vec<u8>, Vec<u8>)> {
    while *pos < b.len() && (is_space(b[*pos]) || b[*pos] == b'/') {
        *pos += 1;
    }
    let first = *b.get(*pos)?;
    if first == b'>' {
        return None;
    }
    let mut name = Vec::new();
    let mut value = Vec::new();
    // Attribute name.
    loop {
        let c = *b.get(*pos)?;
        if c == b'=' && !name.is_empty() {
            *pos += 1;
            break;
        }
        if is_space(c) {
            while *pos < b.len() && is_space(b[*pos]) {
                *pos += 1;
            }
            if b.get(*pos) != Some(&b'=') {
                return Some((name, value));
            }
            *pos += 1;
            break;
        }
        if c == b'/' || c == b'>' {
            return Some((name, value));
        }
        name.push(c.to_ascii_lowercase());
        *pos += 1;
    }
    // Attribute value.
    while *pos < b.len() && is_space(b[*pos]) {
        *pos += 1;
    }
    let c = *b.get(*pos)?;
    if c == b'"' || c == b'\'' {
        *pos += 1;
        loop {
            let d = *b.get(*pos)?;
            *pos += 1;
            if d == c {
                return Some((name, value));
            }
            value.push(d.to_ascii_lowercase());
        }
    }
    if c == b'>' {
        return Some((name, value));
    }
    loop {
        let Some(&d) = b.get(*pos) else {
            return Some((name, value));
        };
        if is_space(d) || d == b'>' {
            return Some((name, value));
        }
        value.push(d.to_ascii_lowercase());
        *pos += 1;
    }
}

/// "Extracting a character encoding from a meta element" (`content`).
fn charset_from_content(content: &[u8]) -> Option<&'static Encoding> {
    let mut pos = 0;
    loop {
        let found = find_ci(content, pos, b"charset")?;
        pos = found + 7;
        while pos < content.len() && is_space(content[pos]) {
            pos += 1;
        }
        if content.get(pos) != Some(&b'=') {
            continue;
        }
        pos += 1;
        while pos < content.len() && is_space(content[pos]) {
            pos += 1;
        }
        let first = *content.get(pos)?;
        if first == b'"' || first == b'\'' {
            let start = pos + 1;
            let len = content.get(start..)?.iter().position(|&c| c == first)?;
            return Encoding::for_label(content.get(start..start + len)?);
        }
        let start = pos;
        let len = content
            .get(start..)?
            .iter()
            .position(|&c| is_space(c) || c == b';')
            .unwrap_or(content.len() - start);
        if len == 0 {
            return None;
        }
        return Encoding::for_label(content.get(start..start + len)?);
    }
}

fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn find_ci(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
        .map(|p| p + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bom_wins() {
        assert_eq!(
            decode_bytes(b"\xEF\xBB\xBFh\xC3\xA9", Some("windows-1252")),
            "hé"
        );
        assert_eq!(decode_bytes(b"\xFF\xFEh\x00i\x00", None), "hi");
        assert_eq!(decode_bytes(b"\xFE\xFF\x00h\x00i", Some("utf-8")), "hi");
        let with_meta = b"\xEF\xBB\xBF<meta charset=windows-1252>\xC3\xA9";
        assert!(decode_bytes(with_meta, None).ends_with('é'));
    }

    #[test]
    fn transport_charset() {
        assert_eq!(decode_bytes(b"caf\xE9", Some("iso-8859-1")), "café");
        assert_eq!(decode_bytes(b"caf\xE9", Some(" Windows-1252 ")), "café");
        assert_eq!(decode_bytes(b"caf\xC3\xA9", Some("utf-8")), "café");
        // Transport wins over meta.
        let doc = b"<meta charset=utf-8>caf\xE9";
        assert!(decode_bytes(doc, Some("latin1")).ends_with("café"));
        // Unknown labels are ignored.
        assert_eq!(
            decode_bytes(b"caf\xC3\xA9", Some("no-such-charset")),
            "café"
        );
        // Lossy UTF-8.
        assert_eq!(decode_bytes(b"a\xFFb", Some("utf-8")), "a\u{FFFD}b");
    }

    #[test]
    fn meta_charset_prescan() {
        let doc =
            b"<!doctype html><html><head><meta charset=\"windows-1252\"><title>\xE9t\xE9</title>";
        assert!(decode_bytes(doc, None).contains("été"));
        let doc =
            b"<meta http-equiv=\"Content-Type\" content=\"text/html; charset=ISO-8859-1\">\xE9";
        assert!(decode_bytes(doc, None).ends_with('é'));
        let doc = b"<META CHARSET='koi8-r'>\xC1";
        assert!(decode_bytes(doc, None).ends_with('а'));
        // content without http-equiv is ignored.
        let doc = b"<meta content=\"text/html; charset=iso-8859-1\">\xC3\xA9";
        assert!(decode_bytes(doc, None).ends_with('é'));
        // UTF-16 labels in meta mean UTF-8; x-user-defined means windows-1252.
        let doc = b"<meta charset=utf-16le>\xC3\xA9";
        assert!(decode_bytes(doc, None).ends_with('é'));
        let doc = b"<meta charset=x-user-defined>\xE9";
        assert!(decode_bytes(doc, None).ends_with('é'));
    }

    #[test]
    fn prescan_skips_comments_and_attributes() {
        let doc = b"<!-- <meta charset=koi8-r> --><meta charset=iso-8859-2>";
        assert_eq!(prescan(doc), Encoding::for_label(b"iso-8859-2"));
        let doc = b"<div title='<meta charset=koi8-r>'></div><meta charset=windows-1251>";
        assert_eq!(prescan(doc), Encoding::for_label(b"windows-1251"));
        let doc = b"<!--><meta charset=iso-8859-5>";
        assert_eq!(prescan(doc), Encoding::for_label(b"iso-8859-5"));
        assert_eq!(prescan(b"<meta charset=bogus>"), None);
        assert_eq!(prescan(b"<metacharset=utf-8>"), None);
        assert_eq!(prescan(b"<meta charset"), None);
        assert_eq!(prescan(b""), None);
        // Beyond the first 1024 bytes the meta is not seen.
        let mut late = vec![b' '; 2000];
        late.extend_from_slice(b"<meta charset=iso-8859-2>");
        assert_eq!(prescan(&late), None);
    }

    #[test]
    fn content_attribute_extraction() {
        assert_eq!(
            charset_from_content(b"text/html; charset=utf-8"),
            Some(UTF_8)
        );
        assert_eq!(
            charset_from_content(b"text/html;charset = \"koi8-r\""),
            Encoding::for_label(b"koi8-r")
        );
        assert_eq!(
            charset_from_content(b"text/html; charset='iso-8859-2'x"),
            Encoding::for_label(b"iso-8859-2")
        );
        assert_eq!(charset_from_content(b"charsetcharset=utf-8"), Some(UTF_8));
        assert_eq!(charset_from_content(b"text/html"), None);
        assert_eq!(charset_from_content(b"charset="), None);
        assert_eq!(charset_from_content(b"charset=\"utf-8"), None);
    }

    #[test]
    fn fallback() {
        assert_eq!(decode_bytes("héllo".as_bytes(), None), "héllo");
        assert_eq!(decode_bytes(b"h\xE9llo", None), "héllo");
        assert_eq!(decode_bytes(b"\x80", None), "€");
        assert_eq!(decode_bytes(b"", None), "");
    }
}
