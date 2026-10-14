//! `data:` URLs (RFC 2397, as processed by the WHATWG Fetch standard).

use url::Url;

use crate::error::NetError;
use crate::mime::parse_mime;
use crate::response::Response;

/// A decoded `data:` URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DataUrl {
    /// Lowercased essence of the media type (`text/plain` by default).
    pub mime: String,
    /// Lowercased charset parameter (`us-ascii` for the default media type).
    pub charset: Option<String>,
    /// Decoded body.
    pub body: Vec<u8>,
}

/// Loads a `data:` URL as a [`Response`].
pub(crate) fn fetch(url: &Url) -> Result<Response, NetError> {
    let data = parse(url)?;
    Ok(Response::ok(
        url.clone(),
        data.mime,
        data.charset,
        data.body,
    ))
}

fn is_ascii_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\x0c' | '\r' | ' ')
}

/// Runs the Fetch "data: URL processor".
pub(crate) fn parse(url: &Url) -> Result<DataUrl, NetError> {
    let mut without_fragment = url.clone();
    without_fragment.set_fragment(None);
    let serialized = without_fragment.as_str();
    let input = serialized
        .get(..5)
        .filter(|scheme| scheme.eq_ignore_ascii_case("data:"))
        .and_then(|_| serialized.get(5..))
        .ok_or_else(|| NetError::Invalid("not a data: URL".into()))?;

    let (mime_part, encoded_body) = input
        .split_once(',')
        .ok_or_else(|| NetError::Invalid("data: URL without a comma".into()))?;
    let mime_part = mime_part.trim_matches(is_ascii_whitespace);
    let mut body = percent_decode(encoded_body.as_bytes());

    let (mime_part, is_base64) = strip_base64_marker(mime_part);
    if is_base64 {
        body = forgiving_base64_decode(&body)
            .ok_or_else(|| NetError::Invalid("malformed base64 in data: URL".into()))?;
    }

    let mime_string;
    let mime_str = if mime_part.starts_with(';') {
        mime_string = format!("text/plain{mime_part}");
        mime_string.as_str()
    } else {
        mime_part
    };
    let (mime, charset) = match parse_mime(mime_str) {
        Some(m) => (m.essence, m.charset),
        None => ("text/plain".to_owned(), Some("us-ascii".to_owned())),
    };
    Ok(DataUrl {
        mime,
        charset,
        body,
    })
}

/// Removes a trailing `;base64` (with optional spaces before `base64`) from
/// the media type, returning whether it was present.
fn strip_base64_marker(mime: &str) -> (&str, bool) {
    const MARKER: &str = "base64";
    let Some(split) = mime.len().checked_sub(MARKER.len()) else {
        return (mime, false);
    };
    let (Some(head), Some(tail)) = (mime.get(..split), mime.get(split..)) else {
        return (mime, false);
    };
    if !tail.eq_ignore_ascii_case(MARKER) {
        return (mime, false);
    }
    match head.trim_end_matches(' ').strip_suffix(';') {
        Some(rest) => (rest, true),
        None => (mime, false),
    }
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Percent-decodes bytes; `%` not followed by two hex digits is kept as-is.
pub(crate) fn percent_decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let b = input[i];
        if b == b'%' {
            let hi = input.get(i + 1).copied().and_then(hex_value);
            let lo = input.get(i + 2).copied().and_then(hex_value);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    out
}

fn base64_value(b: u8) -> Option<u32> {
    let v = match b {
        b'A'..=b'Z' => b - b'A',
        b'a'..=b'z' => b - b'a' + 26,
        b'0'..=b'9' => b - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    };
    Some(u32::from(v))
}

/// The WHATWG "forgiving-base64 decode": ASCII whitespace is ignored, padding
/// is optional, anything else that is not in the base64 alphabet is an error.
pub(crate) fn forgiving_base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut data: Vec<u8> = input
        .iter()
        .copied()
        .filter(|b| !matches!(b, b'\t' | b'\n' | b'\x0c' | b'\r' | b' '))
        .collect();
    if data.len() % 4 == 0 {
        if data.ends_with(b"==") {
            data.truncate(data.len() - 2);
        } else if data.ends_with(b"=") {
            data.truncate(data.len() - 1);
        }
    }
    if data.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(data.len() / 4 * 3 + 2);
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for &b in &data {
        buffer = (buffer << 6) | base64_value(b)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            // Truncation to the low byte is intended.
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::Rng;

    fn data(s: &str) -> Result<DataUrl, NetError> {
        parse(&Url::parse(s).unwrap())
    }

    #[test]
    fn default_media_type() {
        let d = data("data:,Hello%2C%20World!").unwrap();
        assert_eq!(d.mime, "text/plain");
        assert_eq!(d.charset.as_deref(), Some("us-ascii"));
        assert_eq!(d.body, b"Hello, World!");
    }

    #[test]
    fn explicit_media_type_and_charset() {
        let d = data("data:text/html;charset=UTF-8,<h1>Hi</h1>").unwrap();
        assert_eq!(d.mime, "text/html");
        assert_eq!(d.charset.as_deref(), Some("utf-8"));
        assert_eq!(d.body, b"<h1>Hi</h1>");

        let d = data("data:Text/HTML,%3Cp%3E").unwrap();
        assert_eq!(d.mime, "text/html");
        assert_eq!(d.charset, None);
        assert_eq!(d.body, b"<p>");
    }

    #[test]
    fn parameters_without_type() {
        let d = data("data:;charset=utf-8,%C3%A9").unwrap();
        assert_eq!(d.mime, "text/plain");
        assert_eq!(d.charset.as_deref(), Some("utf-8"));
        assert_eq!(d.body, "é".as_bytes());
    }

    #[test]
    fn invalid_media_type_falls_back() {
        let d = data("data:foo,bar").unwrap();
        assert_eq!(d.mime, "text/plain");
        assert_eq!(d.charset.as_deref(), Some("us-ascii"));
        assert_eq!(d.body, b"bar");
    }

    #[test]
    fn base64() {
        let d = data("data:text/plain;base64,SGVsbG8sIFdvcmxkIQ==").unwrap();
        assert_eq!(d.mime, "text/plain");
        assert_eq!(d.charset, None);
        assert_eq!(d.body, b"Hello, World!");

        // Case-insensitive marker, spaces before it, missing padding.
        let d = data("data:image/png; BASE64,iVBORw0KGgo").unwrap();
        assert_eq!(d.mime, "image/png");
        assert_eq!(d.body, b"\x89PNG\r\n\x1a\n");

        // Only ";base64" means base64.
        let d = data("data:;base64,SGk=").unwrap();
        assert_eq!(d.mime, "text/plain");
        assert_eq!(d.charset.as_deref(), Some("us-ascii"));
        assert_eq!(d.body, b"Hi");
        let d = data("data:text/plainbase64,SGk=").unwrap();
        assert_eq!(d.body, b"SGk=");
    }

    #[test]
    fn base64_with_whitespace_and_percent_escapes() {
        let d = data("data:text/plain;base64,SGVs%20bG8s%0AIFdv%0D%0AcmxkIQ%3D%3D").unwrap();
        assert_eq!(d.body, b"Hello, World!");
        let d = data("data:text/plain;base64,SGVs bG8=").unwrap();
        assert_eq!(d.body, b"Hello");
    }

    #[test]
    fn invalid_base64() {
        assert!(matches!(
            data("data:text/plain;base64,SGV$bG8="),
            Err(NetError::Invalid(_))
        ));
        assert!(matches!(data("data:;base64,A"), Err(NetError::Invalid(_))));
        assert!(matches!(
            data("data:;base64,SG=k"),
            Err(NetError::Invalid(_))
        ));
        assert!(matches!(
            data("data:;base64,SGk==="),
            Err(NetError::Invalid(_))
        ));
    }

    #[test]
    fn missing_comma() {
        assert!(matches!(data("data:text/plain"), Err(NetError::Invalid(_))));
        assert!(matches!(data("data:"), Err(NetError::Invalid(_))));
    }

    #[test]
    fn fragment_and_query() {
        let d = data("data:text/plain,a?b=c#frag").unwrap();
        assert_eq!(d.body, b"a?b=c");
        let d = data("data:text/plain,%23notfrag").unwrap();
        assert_eq!(d.body, b"#notfrag");
    }

    #[test]
    fn empty_body() {
        let d = data("data:text/html,").unwrap();
        assert_eq!(d.mime, "text/html");
        assert!(d.body.is_empty());
        let d = data("data:;base64,").unwrap();
        assert!(d.body.is_empty());
    }

    #[test]
    fn not_a_data_url() {
        assert!(matches!(
            parse(&Url::parse("https://example.com/").unwrap()),
            Err(NetError::Invalid(_))
        ));
    }

    #[test]
    fn fetch_builds_response() {
        let url = Url::parse("data:text/css,a%7Bcolor:red%7D").unwrap();
        let r = fetch(&url).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.url, url);
        assert_eq!(r.content_type.as_deref(), Some("text/css"));
        assert_eq!(r.body, b"a{color:red}");
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode(b"a%20b"), b"a b");
        assert_eq!(percent_decode(b"%zz%2"), b"%zz%2");
        assert_eq!(percent_decode(b"%"), b"%");
        assert_eq!(percent_decode(b"%e2%9c%93"), "✓".as_bytes());
        assert_eq!(percent_decode(b""), b"");
    }

    #[test]
    fn base64_decoder() {
        assert_eq!(forgiving_base64_decode(b"").unwrap(), b"");
        assert_eq!(forgiving_base64_decode(b"Zg==").unwrap(), b"f");
        assert_eq!(forgiving_base64_decode(b"Zg").unwrap(), b"f");
        assert_eq!(forgiving_base64_decode(b"Zm8=").unwrap(), b"fo");
        assert_eq!(forgiving_base64_decode(b"Zm9v").unwrap(), b"foo");
        assert_eq!(
            forgiving_base64_decode(b" Zm 9v\n YmFy ").unwrap(),
            b"foobar"
        );
        assert_eq!(
            forgiving_base64_decode(b"+/+/").unwrap(),
            [0xFB, 0xFF, 0xBF]
        );
        assert!(forgiving_base64_decode(b"Z").is_none());
        assert!(forgiving_base64_decode(b"Zg=").is_none());
        assert!(forgiving_base64_decode(b"Z===").is_none());
        assert!(forgiving_base64_decode(b"Zm9v-_").is_none());
        // Round trip over every byte value.
        let all: Vec<u8> = (0..=255).collect();
        let encoded = encode_for_test(&all);
        assert_eq!(forgiving_base64_decode(encoded.as_bytes()).unwrap(), all);
    }

    fn encode_for_test(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
            for i in 0..=chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            }
        }
        out
    }

    #[test]
    fn spaces_around_media_type_and_marker() {
        let d = data("data: text/html ; base64 , SGk=").unwrap();
        assert_eq!(d.mime, "text/html");
        assert_eq!(d.body, b"Hi");
    }

    #[test]
    fn real_world_media_types() {
        // Common non-standard `;utf8` flag on SVG data URLs.
        let d = data("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        assert_eq!(d.mime, "image/svg+xml");
        assert_eq!(d.charset, None);
        assert_eq!(d.body, b"<svg xmlns='http://www.w3.org/2000/svg'/>");
        let d = data("data:text/plain;charset=\"utf-8\",x").unwrap();
        assert_eq!(d.charset.as_deref(), Some("utf-8"));
        let d = data("data:text/plain;CHARSET=UTF-8;BASE64,SGk=").unwrap();
        assert_eq!(
            (d.charset.as_deref(), d.body.as_slice()),
            (Some("utf-8"), b"Hi".as_slice())
        );
    }

    #[test]
    fn base64_marker_must_be_literal_and_last() {
        // The media type is not percent-decoded (Fetch standard).
        let d = data("data:text/plain;%62ase64,SGk=").unwrap();
        assert_eq!(d.body, b"SGk=");
        // `;base64` followed by another parameter is just a parameter.
        let d = data("data:;base64;charset=utf-8,SGk=").unwrap();
        assert_eq!(d.body, b"SGk=");
        assert_eq!(d.charset.as_deref(), Some("utf-8"));
        // A lone `%` stays as is in plain bodies but breaks base64.
        assert_eq!(data("data:,%").unwrap().body, b"%");
        assert!(matches!(data("data:;base64,%"), Err(NetError::Invalid(_))));
    }

    #[test]
    fn fuzz_base64_round_trips_through_whitespace_and_padding() {
        let mut rng = Rng::new(7);
        for _ in 0..2_000 {
            let bytes = rng.bytes(200);
            let encoded = encode_for_test(&bytes);
            let mut noisy = String::new();
            for c in encoded.chars() {
                if rng.below(5) == 0 {
                    noisy.push([' ', '\t', '\n', '\r', '\x0c'][rng.below(5)]);
                }
                noisy.push(c);
            }
            // Optional padding.
            let padded = match encoded.len() % 4 {
                2 => format!("{noisy}=="),
                3 => format!("{noisy}="),
                _ => noisy.clone(),
            };
            assert_eq!(
                forgiving_base64_decode(noisy.as_bytes()).as_deref(),
                Some(&bytes[..])
            );
            assert_eq!(
                forgiving_base64_decode(padded.as_bytes()).as_deref(),
                Some(&bytes[..])
            );
            // Through a real data: URL with percent-encoded padding.
            let url = format!(
                "data:application/octet-stream;base64,{}",
                padded.replace('=', "%3D")
            );
            if let Ok(parsed) = Url::parse(&url) {
                assert_eq!(parse(&parsed).unwrap().body, bytes);
            }
        }
    }

    #[test]
    fn fuzz_random_data_urls_never_panic() {
        let alphabet = [
            "a", "/", ";", ",", "=", "%", "%2", "%41", "%zz", "base64", "BASE64", " ", "\"", "#",
            "?", "é", "+", "==", "charset", "text", "\\", "\t",
        ];
        let mut rng = Rng::new(99);
        for _ in 0..20_000 {
            let input = format!("data:{}", rng.string_from(&alphabet, 16));
            let Ok(url) = Url::parse(&input) else {
                continue;
            };
            if let Ok(d) = parse(&url) {
                assert!(parse_mime(&d.mime).is_some(), "{input:?} -> {}", d.mime);
                assert!(d.body.len() <= url.as_str().len());
            }
        }
        // Raw percent-decoding is total and never grows the input.
        for _ in 0..5_000 {
            let bytes = rng.bytes(64);
            assert!(percent_decode(&bytes).len() <= bytes.len());
            assert!(forgiving_base64_decode(&bytes).is_none_or(|out| out.len() <= bytes.len()));
        }
    }

    #[test]
    fn large_data_url_is_linear() {
        let payload = "QUJD".repeat(500_000);
        let d = data(&format!("data:application/octet-stream;base64,{payload}")).unwrap();
        assert_eq!(d.body.len(), 1_500_000);
    }
}
