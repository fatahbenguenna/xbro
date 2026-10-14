//! Text decoding and form encoding helpers.

use encoding_rs::{Encoding, UTF_8};

/// Decodes `body` with the encoding named by `charset` (a WHATWG label such as
/// `"latin1"` or `"utf-8"`). A byte order mark always wins over the label;
/// unknown or missing labels fall back to UTF-8. Malformed sequences become
/// U+FFFD, so this never fails.
pub(crate) fn decode_text(body: &[u8], charset: Option<&str>) -> String {
    let encoding = charset
        .map(|label| label.trim_matches(|c: char| c.is_ascii_whitespace() || c == '"'))
        .and_then(|label| Encoding::for_label(label.as_bytes()))
        .unwrap_or(UTF_8);
    // `decode` performs BOM sniffing (UTF-8, UTF-16LE, UTF-16BE) and strips the BOM.
    let (text, _, _) = encoding.decode(body);
    text.into_owned()
}

/// Serializes `fields` as `application/x-www-form-urlencoded`.
pub(crate) fn encode_form(fields: &[(String, String)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::Rng;

    fn fields(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn decode_defaults_to_utf8() {
        assert_eq!(decode_text("héllo".as_bytes(), None), "héllo");
        assert_eq!(decode_text(b"", None), "");
    }

    #[test]
    fn decode_with_label() {
        assert_eq!(
            decode_text(&[0x63, 0x61, 0x66, 0xE9], Some("latin1")),
            "café"
        );
        assert_eq!(
            decode_text(&[0x63, 0x61, 0x66, 0xE9], Some("ISO-8859-1")),
            "café"
        );
        // windows-1252 specific: 0x80 is the euro sign.
        assert_eq!(decode_text(&[0x80], Some("windows-1252")), "€");
        // Labels are trimmed and may be quoted.
        assert_eq!(decode_text(&[0xE9], Some("  \"latin1\" ")), "é");
        // Shift_JIS.
        assert_eq!(decode_text(&[0x82, 0xA0], Some("shift_jis")), "あ");
    }

    #[test]
    fn decode_unknown_label_falls_back_to_utf8() {
        assert_eq!(
            decode_text("ok ✓".as_bytes(), Some("no-such-charset")),
            "ok ✓"
        );
        assert_eq!(decode_text("ok".as_bytes(), Some("")), "ok");
    }

    #[test]
    fn decode_is_lossy() {
        assert_eq!(decode_text(&[b'a', 0xFF, b'b'], None), "a\u{FFFD}b");
        assert_eq!(decode_text(&[0xE2, 0x82], Some("utf-8")), "\u{FFFD}");
    }

    #[test]
    fn bom_overrides_label() {
        let mut utf8 = vec![0xEF, 0xBB, 0xBF];
        utf8.extend_from_slice("é".as_bytes());
        assert_eq!(decode_text(&utf8, Some("latin1")), "é");

        let utf16le = [0xFF, 0xFE, b'h', 0, b'i', 0];
        assert_eq!(decode_text(&utf16le, None), "hi");
        let utf16be = [0xFE, 0xFF, 0, b'h', 0, b'i'];
        assert_eq!(decode_text(&utf16be, Some("utf-8")), "hi");
    }

    #[test]
    fn fuzz_decode_never_panics() {
        let labels = [
            None,
            Some("utf-8"),
            Some("UTF-16"),
            Some("utf-16be"),
            Some("shift_jis"),
            Some("gb18030"),
            Some("iso-2022-jp"),
            Some("x-user-defined"),
            Some("windows-1252"),
            Some("big5"),
            Some("euc-kr"),
            Some("\"\""),
            Some("é"),
        ];
        let mut rng = Rng::new(3);
        for _ in 0..5_000 {
            let mut body = rng.bytes(200);
            if rng.below(4) == 0 {
                body.splice(0..0, [0xEF, 0xBB, 0xBF]);
            }
            let label = labels[rng.below(labels.len())];
            let text = decode_text(&body, label);
            if label.is_none() && !body.starts_with(&[0xEF, 0xBB, 0xBF]) {
                if let Ok(valid) = std::str::from_utf8(&body) {
                    assert_eq!(text, valid);
                }
            }
        }
    }

    #[test]
    fn fuzz_encode_form_round_trips() {
        let alphabet = [
            "a", " ", "+", "&", "=", "%", "é", "✓", "\n", "\r\n", "#", "?", "/", "*", "~", "\u{0}",
        ];
        let mut rng = Rng::new(11);
        for _ in 0..2_000 {
            let pairs: Vec<(String, String)> = (0..rng.below(5))
                .map(|_| (rng.string_from(&alphabet, 6), rng.string_from(&alphabet, 6)))
                .collect();
            let encoded = encode_form(&pairs);
            assert!(encoded.is_ascii());
            assert!(!encoded.contains(' '));
            let decoded: Vec<(String, String)> = url::form_urlencoded::parse(encoded.as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            assert_eq!(decoded, pairs, "{encoded}");
        }
    }

    #[test]
    fn encode_form_basic() {
        assert_eq!(
            encode_form(&fields(&[("a", "1"), ("b", "hello world")])),
            "a=1&b=hello+world"
        );
        assert_eq!(encode_form(&[]), "");
        assert_eq!(encode_form(&fields(&[("empty", "")])), "empty=");
    }

    #[test]
    fn encode_form_escapes() {
        assert_eq!(
            encode_form(&fields(&[("q", "a&b=c+d%"), ("k y", "x/y?z")])),
            "q=a%26b%3Dc%2Bd%25&k+y=x%2Fy%3Fz"
        );
        assert_eq!(
            encode_form(&fields(&[("name", "Zoë Ünïcode ✓")])),
            "name=Zo%C3%AB+%C3%9Cn%C3%AFcode+%E2%9C%93"
        );
        assert_eq!(encode_form(&fields(&[("t", "a\nb")])), "t=a%0Ab");
    }
}
