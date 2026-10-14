//! Named character references (`&amp;`, `&eacute;`, ...).
//!
//! The table contains every HTML 4 entity and the full set of HTML5 (WHATWG)
//! named references. Decoding follows the WHATWG "character reference state":
//! longest-prefix matching for the legacy semicolon-less names, the attribute
//! special case, and the numeric reference fix-ups (NUL, surrogates,
//! out-of-range values and the windows-1252 remapping of 0x80–0x9F).

mod table;

/// Length of the longest entity name (`CounterClockwiseContourIntegral`).
const MAX_NAME_LEN: usize = 32;
/// Length of the longest legacy (semicolon-less) entity name (`middot`).
const MAX_LEGACY_LEN: usize = 6;

/// Looks up a named character reference without the leading `&` and the
/// trailing `;` (e.g. `"amp"` → `"&"`). Returns `None` for unknown names.
pub fn lookup(name: &str) -> Option<&'static str> {
    let idx = table::ENTITIES
        .binary_search_by(|(n, _)| n.as_bytes().cmp(name.as_bytes()))
        .ok()?;
    table::ENTITIES.get(idx).map(|&(_, value)| value)
}

/// True when `name` is one of the legacy names that browsers also recognise
/// without a trailing semicolon (`&amp`, `&copy`, `&eacute`, ...).
pub fn is_legacy(name: &str) -> bool {
    table::LEGACY
        .binary_search_by(|n| n.as_bytes().cmp(name.as_bytes()))
        .is_ok()
}

/// Decodes all character references (`&name;`, `&#123;`, `&#x7B;`, and the
/// legacy semicolon-less forms such as `&amp`) in `text`.
/// `in_attribute` enables the attribute-value special case where `&name=`
/// or `&nameX` (alphanumeric after a legacy name) is left untouched.
pub fn decode(text: &str, in_attribute: bool) -> String {
    let mut out = String::with_capacity(text.len());
    decode_into(text, in_attribute, &mut out);
    out
}

/// Like [`decode`], appending the result to `out`.
pub(crate) fn decode_into(text: &str, in_attribute: bool, out: &mut String) {
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        let (before, from_amp) = rest.split_at(amp);
        out.push_str(before);
        let after = &from_amp[1..];
        match match_reference(after, in_attribute) {
            Some((consumed, decoded)) => {
                decoded.push_to(out);
                rest = after.get(consumed..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
}

/// The replacement text of a matched character reference.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Decoded {
    Str(&'static str),
    Char(char),
}

impl Decoded {
    pub(crate) fn push_to(self, out: &mut String) {
        match self {
            Decoded::Str(s) => out.push_str(s),
            Decoded::Char(c) => out.push(c),
        }
    }
}

/// Tries to match a character reference at the start of `after_amp` (the
/// text that follows a `&`). Returns the number of bytes consumed (not
/// counting the `&`) and the replacement, or `None` when the `&` must be
/// kept literally.
pub(crate) fn match_reference(after_amp: &str, in_attribute: bool) -> Option<(usize, Decoded)> {
    let bytes = after_amp.as_bytes();
    match bytes.first()? {
        b'#' => match_numeric(bytes),
        b if b.is_ascii_alphanumeric() => match_named(after_amp, in_attribute),
        _ => None,
    }
}

fn match_named(s: &str, in_attribute: bool) -> Option<(usize, Decoded)> {
    let bytes = s.as_bytes();
    // Longer runs cannot match any name; capping keeps the scan constant-time.
    let run = bytes
        .iter()
        .take(MAX_NAME_LEN + 1)
        .take_while(|b| b.is_ascii_alphanumeric())
        .count();
    // `name;` matches only when the semicolon directly follows the whole run.
    if run <= MAX_NAME_LEN && bytes.get(run) == Some(&b';') {
        if let Some(value) = s.get(..run).and_then(lookup) {
            return Some((run + 1, Decoded::Str(value)));
        }
    }
    // Legacy names: longest prefix of the run, no semicolon.
    let longest = run.min(MAX_LEGACY_LEN);
    for len in (2..=longest).rev() {
        let Some(prefix) = s.get(..len) else { continue };
        if !is_legacy(prefix) {
            continue;
        }
        if in_attribute {
            // Historical reasons: `&copy=` or `&copyx` in attributes stay as-is.
            let next = bytes.get(len).copied();
            if next.is_some_and(|b| b == b'=' || b.is_ascii_alphanumeric()) {
                return None;
            }
        }
        let value = lookup(prefix)?;
        return Some((len, Decoded::Str(value)));
    }
    None
}

fn match_numeric(bytes: &[u8]) -> Option<(usize, Decoded)> {
    // bytes[0] == b'#'
    let (radix, start) = match bytes.get(1) {
        Some(b'x') | Some(b'X') => (16u32, 2usize),
        _ => (10u32, 1usize),
    };
    let digits = bytes
        .get(start..)?
        .iter()
        .take_while(|b| (**b as char).is_digit(radix))
        .count();
    if digits == 0 {
        return None;
    }
    let mut value: u32 = 0;
    for &b in bytes.get(start..start + digits)? {
        let d = (b as char).to_digit(radix).unwrap_or(0);
        value = value.saturating_mul(radix).saturating_add(d);
    }
    let mut consumed = start + digits;
    if bytes.get(consumed) == Some(&b';') {
        consumed += 1;
    }
    Some((consumed, Decoded::Char(numeric_char(value))))
}

/// Maps the value of a numeric character reference to a character, applying
/// the WHATWG fix-ups.
pub(crate) fn numeric_char(value: u32) -> char {
    const REPLACEMENT: char = '\u{FFFD}';
    match value {
        0 => REPLACEMENT,
        0xD800..=0xDFFF => REPLACEMENT,
        v if v > 0x10FFFF => REPLACEMENT,
        0x80..=0x9F => windows_1252_c1(value),
        v => char::from_u32(v).unwrap_or(REPLACEMENT),
    }
}

/// The windows-1252 interpretation of C1 control code points, as mandated
/// for numeric character references (`&#128;` → `€`).
fn windows_1252_c1(value: u32) -> char {
    let mapped = match value {
        0x80 => 0x20AC,
        0x82 => 0x201A,
        0x83 => 0x0192,
        0x84 => 0x201E,
        0x85 => 0x2026,
        0x86 => 0x2020,
        0x87 => 0x2021,
        0x88 => 0x02C6,
        0x89 => 0x2030,
        0x8A => 0x0160,
        0x8B => 0x2039,
        0x8C => 0x0152,
        0x8E => 0x017D,
        0x91 => 0x2018,
        0x92 => 0x2019,
        0x93 => 0x201C,
        0x94 => 0x201D,
        0x95 => 0x2022,
        0x96 => 0x2013,
        0x97 => 0x2014,
        0x98 => 0x02DC,
        0x99 => 0x2122,
        0x9A => 0x0161,
        0x9B => 0x203A,
        0x9C => 0x0153,
        0x9E => 0x017E,
        0x9F => 0x0178,
        other => other,
    };
    char::from_u32(mapped).unwrap_or('\u{FFFD}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_and_unique() {
        for pair in table::ENTITIES.windows(2) {
            assert!(pair[0].0 < pair[1].0, "{} !< {}", pair[0].0, pair[1].0);
        }
        for pair in table::LEGACY.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        assert!(table::ENTITIES.len() >= 2125);
        assert_eq!(table::LEGACY.len(), 106);
        assert!(table::ENTITIES
            .iter()
            .all(|(n, _)| n.len() <= MAX_NAME_LEN && n.bytes().all(|b| b.is_ascii_alphanumeric())));
        assert!(table::LEGACY.iter().all(|n| n.len() <= MAX_LEGACY_LEN));
    }

    #[test]
    fn all_html4_entities_present() {
        assert_eq!(table::HTML4.len(), 252);
        for name in table::HTML4 {
            assert!(lookup(name).is_some(), "missing HTML4 entity {name}");
        }
        assert!(table::LEGACY.iter().all(|n| lookup(n).is_some()));
    }

    #[test]
    fn common_html5_entities_present() {
        let names = [
            "apos", "hellip", "mdash", "ndash", "lsquo", "rsquo", "ldquo", "rdquo", "sbquo",
            "bdquo", "laquo", "raquo", "nbsp", "thinsp", "ensp", "emsp", "zwj", "zwnj", "lrm",
            "rlm", "euro", "trade", "check", "cross", "star", "starf", "larr", "rarr", "uarr",
            "darr", "harr", "lArr", "rArr", "middot", "bull", "dagger", "Dagger", "permil",
            "prime", "Prime", "oline", "frasl", "times", "divide", "minus", "plusmn", "le", "ge",
            "ne", "asymp", "infin", "sum", "prod", "radic", "part", "nabla", "isin", "notin", "ni",
            "and", "or", "cap", "cup", "int", "there4", "sim", "cong", "equiv", "sub", "sup",
            "nsub", "sube", "supe", "oplus", "otimes", "perp", "sdot", "lceil", "rceil", "lfloor",
            "rfloor", "lang", "rang", "loz", "spades", "clubs", "hearts", "diams", "NewLine",
            "Tab", "colon", "comma", "period", "excl", "quest", "num", "dollar", "percnt", "lpar",
            "rpar", "ast", "plus", "sol", "bsol", "lsqb", "rsqb", "lcub", "rcub", "verbar", "vert",
            "grave", "Hat", "lowbar", "equals", "semi", "commat", "shy",
        ];
        for name in names {
            assert!(lookup(name).is_some(), "missing entity {name}");
        }
    }

    #[test]
    fn lookup_values() {
        assert_eq!(lookup("amp"), Some("&"));
        assert_eq!(lookup("lt"), Some("<"));
        assert_eq!(lookup("eacute"), Some("é"));
        assert_eq!(lookup("hellip"), Some("…"));
        assert_eq!(lookup("euro"), Some("€"));
        assert_eq!(lookup("nbsp"), Some("\u{A0}"));
        assert_eq!(lookup("NewLine"), Some("\n"));
        assert_eq!(lookup("lang"), Some("\u{27E8}"));
        assert_eq!(lookup("NotEqualTilde"), Some("\u{2242}\u{338}"));
        assert_eq!(lookup("Ascr"), Some("\u{1D49C}"));
        assert_eq!(lookup("nosuchentity"), None);
        assert_eq!(lookup(""), None);
        assert_eq!(lookup("AMP"), Some("&"));
        assert_eq!(lookup("amp;"), None);
    }

    #[test]
    fn decode_named() {
        assert_eq!(decode("a &amp; b", false), "a & b");
        assert_eq!(decode("&lt;p&gt;", false), "<p>");
        assert_eq!(decode("caf&eacute;", false), "café");
        assert_eq!(decode("&hellip;&mdash;&rsquo;", false), "…—’");
        assert_eq!(decode("&unknown; &", false), "&unknown; &");
        assert_eq!(decode("&&amp;&", false), "&&&");
        assert_eq!(decode("&;", false), "&;");
        assert_eq!(decode("", false), "");
        assert_eq!(decode("no refs", false), "no refs");
        assert_eq!(decode("&CounterClockwiseContourIntegral;", false), "∳");
    }

    #[test]
    fn decode_legacy_without_semicolon() {
        assert_eq!(decode("&amp", false), "&");
        assert_eq!(decode("&ampx", false), "&x");
        assert_eq!(decode("&copy 2024", false), "© 2024");
        assert_eq!(decode("&notit;", false), "¬it;");
        assert_eq!(decode("&notin;", false), "∉");
        assert_eq!(decode("&noti", false), "¬i");
        assert_eq!(decode("&eacutex", false), "éx");
        // Non-legacy names require the semicolon.
        assert_eq!(decode("&hellip", false), "&hellip");
        assert_eq!(decode("&euro", false), "&euro");
        assert_eq!(decode("&ltfoo", false), "<foo");
        assert_eq!(decode("&LT", false), "<");
    }

    #[test]
    fn decode_attribute_special_case() {
        assert_eq!(decode("?a=1&copy=2", true), "?a=1&copy=2");
        assert_eq!(decode("?a=1&copyx", true), "?a=1&copyx");
        assert_eq!(decode("?a=1&copy;=2", true), "?a=1©=2");
        assert_eq!(decode("&copy", true), "©");
        assert_eq!(decode("&copy ", true), "© ");
        assert_eq!(decode("x&amp;y", true), "x&y");
        assert_eq!(decode("x&ampy", true), "x&ampy");
        assert_eq!(decode("&notit;", true), "&notit;");
        assert_eq!(decode("&lang=fr", true), "&lang=fr");
        assert_eq!(decode("&#65=", true), "A=");
    }

    #[test]
    fn decode_numeric() {
        assert_eq!(decode("&#65;&#x42;&#X43;", false), "ABC");
        assert_eq!(decode("&#65", false), "A");
        assert_eq!(decode("&#x41g", false), "Ag");
        assert_eq!(decode("&#0;", false), "\u{FFFD}");
        assert_eq!(decode("&#xD800;", false), "\u{FFFD}");
        assert_eq!(decode("&#xDFFF;", false), "\u{FFFD}");
        assert_eq!(decode("&#x110000;", false), "\u{FFFD}");
        assert_eq!(decode("&#99999999999999999999;", false), "\u{FFFD}");
        assert_eq!(decode("&#128;", false), "€");
        assert_eq!(decode("&#x80;", false), "€");
        assert_eq!(decode("&#x9F;", false), "Ÿ");
        assert_eq!(decode("&#x81;", false), "\u{81}");
        assert_eq!(decode("&#x1F600;", false), "😀");
        assert_eq!(decode("&#;", false), "&#;");
        assert_eq!(decode("&#x;", false), "&#x;");
        assert_eq!(decode("&#xg", false), "&#xg");
        assert_eq!(decode("&#", false), "&#");
        assert_eq!(decode("&#00065;", false), "A");
    }

    #[test]
    fn decode_handles_multibyte_neighbours() {
        assert_eq!(decode("é&amp;é", false), "é&é");
        assert_eq!(decode("&é", false), "&é");
        assert_eq!(decode("日本&lt;語", false), "日本<語");
    }
}
