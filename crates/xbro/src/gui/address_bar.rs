//! The address bar: an editable field when focused, a formatted URL otherwise.

use std::ops::Range;

use xbro_engine::Url;

use super::line_edit::LineEdit;

/// Longest URL whose escapes are decoded for display (longer ones, such as
/// big `data:` URLs, are shown as they are: they are drawn every frame).
const MAX_DECODED_URL: usize = 8 * 1024;

/// Text shown for a URL in the address bar (see [`readable_url`]). The new
/// tab page shows an empty field (with a placeholder).
pub fn display_text(url: Option<&Url>) -> String {
    match url {
        Some(url) if url.as_str() == "about:home" => String::new(),
        Some(url) => readable_url(url),
        None => String::new(),
    }
}

/// A URL as people read it: percent-escapes of non-ASCII characters are
/// shown as the characters (`…/wiki/Fran%C3%A7ais` reads `…/wiki/Français`),
/// as other browsers do. ASCII escapes stay escaped (they may stand for
/// delimiters: `%2F`, `%23`, `%25`…), and so do invisible or misleading
/// characters (spaces, controls, zero-width and bidi marks, slash
/// look-alikes). Parsing the result gives back the same URL.
pub fn readable_url(url: &Url) -> String {
    let s = url.as_str();
    if s.len() > MAX_DECODED_URL || !s.contains('%') {
        return s.to_string();
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            if let Some((c, len)) = escaped_char(&bytes[i..]).filter(|&(c, _)| readable(c)) {
                let mut buf = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                i += len;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    // Only whole characters were added to the (ASCII) serialization.
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// The non-ASCII character escaped as UTF-8 at the start of `bytes`
/// (`%C3%A7` → `ç`) and the length of its escapes.
fn escaped_char(bytes: &[u8]) -> Option<(char, usize)> {
    let byte = |k: usize| -> Option<u8> {
        let escape = bytes.get(3 * k..3 * k + 3)?;
        if escape.first() != Some(&b'%') {
            return None;
        }
        let hex = std::str::from_utf8(escape.get(1..)?).ok()?;
        u8::from_str_radix(hex, 16).ok()
    };
    let first = byte(0)?;
    let len = match first {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None,
    };
    let mut utf8 = [first, 0, 0, 0];
    for (k, slot) in utf8.iter_mut().enumerate().take(len).skip(1) {
        *slot = byte(k)?;
    }
    let c = std::str::from_utf8(utf8.get(..len)?).ok()?.chars().next()?;
    Some((c, 3 * len))
}

/// Whether a decoded character can be shown without misleading the reader.
fn readable(c: char) -> bool {
    !c.is_control()
        && !c.is_whitespace()
        && !matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{17B4}'
                | '\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202F}'
                | '\u{2044}'
                | '\u{205F}'..='\u{206F}'
                | '\u{2215}'
                | '\u{2571}'
                | '\u{29F8}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FF0F}'
                | '\u{FFA0}'
                | '\u{FFF0}'..='\u{FFFF}'
                | '\u{E0000}'..='\u{E0FFF}'
        )
}

/// Splits a displayed URL into runs drawn normally (`false`) or dimmed
/// (`true`): for http(s) URLs the scheme and everything after the host are
/// dimmed so that the domain stands out; other URLs are not dimmed.
pub fn url_segments(text: &str) -> Vec<(Range<usize>, bool)> {
    let scheme_end = ["https://", "http://"]
        .iter()
        .find(|p| {
            text.get(..p.len())
                .is_some_and(|s| s.eq_ignore_ascii_case(p))
        })
        .map(|p| p.len());
    let Some(host_start) = scheme_end else {
        return vec![(0..text.len(), false)];
    };
    let host_end = text[host_start..]
        .find(['/', '?', '#'])
        .map_or(text.len(), |i| host_start + i);
    let mut segments = vec![(0..host_start, true), (host_start..host_end, false)];
    if host_end < text.len() {
        segments.push((host_end..text.len(), true));
    }
    segments
}

/// The address bar state.
#[derive(Clone, Debug, Default)]
pub struct AddressBar {
    pub edit: LineEdit,
    pub focused: bool,
    /// Horizontal scroll of the edited text, in logical px.
    pub scroll_x: f32,
}

impl AddressBar {
    /// Focuses the field with `current` (the displayed URL) selected.
    pub fn focus(&mut self, current: &str) {
        self.edit.set_text(current);
        self.edit.select_all();
        self.focused = true;
        self.scroll_x = 0.0;
    }

    /// Leaves editing mode; the field shows the tab's URL again.
    pub fn blur(&mut self) {
        self.focused = false;
        self.scroll_x = 0.0;
    }

    /// Enter: the URL to navigate to (address-bar interpretation of the
    /// text), leaving editing mode.
    pub fn submit(&mut self) -> Url {
        self.blur();
        xbro_net::parse_user_input(self.edit.text())
    }
}

/// Horizontal scroll that keeps the caret (at `caret_x`) visible in a field
/// `inner_width` wide showing `text_width` px of text, starting from `scroll`.
pub fn scroll_to_caret(scroll: f32, caret_x: f32, text_width: f32, inner_width: f32) -> f32 {
    let mut s = scroll;
    if caret_x - s > inner_width {
        s = caret_x - inner_width;
    }
    if caret_x < s {
        s = caret_x;
    }
    // No empty space after the end of the text when it shrinks.
    s.min((text_width - inner_width).max(0.0)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_urls() {
        let home = Url::parse("about:home").unwrap();
        assert_eq!(display_text(Some(&home)), "");
        assert_eq!(display_text(None), "");
        let url = Url::parse("https://example.com/a?b=c").unwrap();
        assert_eq!(display_text(Some(&url)), "https://example.com/a?b=c");
    }

    #[test]
    fn non_ascii_escapes_are_shown_as_characters() {
        let shown = |s: &str| readable_url(&Url::parse(s).unwrap());
        assert_eq!(
            shown("https://fr.wikipedia.org/wiki/Fran%C3%A7ais"),
            "https://fr.wikipedia.org/wiki/Français"
        );
        // Typed non-ASCII text is escaped by the parser, then shown again.
        assert_eq!(
            shown("http://a.test/r?q=héllo wörld €#ç"),
            "http://a.test/r?q=héllo%20wörld%20€#ç"
        );
        assert_eq!(shown("https://a.test/%F0%9F%A6%80"), "https://a.test/🦀");
        // ASCII escapes, spaces, invisible and misleading characters stay escaped.
        for kept in [
            "https://a.test/a%2Fb%25c%23d%20e",
            "https://a.test/%C2%A0x",
            "https://a.test/%E2%80%AEevil",
            "https://a.test/%E2%80%8B",
            "https://a.test/%E2%88%95etc",
            "https://a.test/%C2%85",
        ] {
            assert_eq!(shown(kept), kept);
        }
        // Broken sequences are left alone, the rest is decoded.
        assert_eq!(
            shown("https://a.test/%C3%28%C3%A9"),
            "https://a.test/%C3%28é"
        );
        assert_eq!(shown("https://a.test/%E2%82"), "https://a.test/%E2%82");
        assert_eq!(shown("https://a.test/%C3"), "https://a.test/%C3");
        assert_eq!(
            shown("https://a.test/%ED%A0%80"),
            "https://a.test/%ED%A0%80"
        );
        assert_eq!(shown("https://a.test/%c3%a9"), "https://a.test/é");
        // What is shown parses back to the same URL.
        for s in [
            "https://fr.wikipedia.org/wiki/Fran%C3%A7ais",
            "http://a.test/r?q=h%C3%A9llo+w%C3%B6rld&x=%E2%82%AC#%C3%A7",
            "https://a.test/a%2Fb%E2%80%AE%C3%A9",
        ] {
            let url = Url::parse(s).unwrap();
            assert_eq!(xbro_net::parse_user_input(&readable_url(&url)), url);
        }
        // Huge URLs are shown as they are.
        let long = format!("data:text/plain,%C3%A9{}", "a".repeat(MAX_DECODED_URL));
        assert_eq!(shown(&long), long);
        assert_eq!(
            display_text(Some(&Url::parse("https://a.test/%C3%A9").unwrap())),
            "https://a.test/é"
        );
    }

    #[test]
    fn segments_dim_scheme_and_path() {
        let text = "https://www.example.com/wiki/Page";
        let segs = url_segments(text);
        let parts: Vec<(&str, bool)> = segs.iter().map(|(r, d)| (&text[r.clone()], *d)).collect();
        assert_eq!(
            parts,
            [
                ("https://", true),
                ("www.example.com", false),
                ("/wiki/Page", true)
            ]
        );
        let text = "http://localhost:8000";
        let segs = url_segments(text);
        assert_eq!(segs, vec![(0..7, true), (7..21, false)]);
        assert_eq!(url_segments("file:///tmp/a.html"), vec![(0..18, false)]);
        assert_eq!(url_segments(""), vec![(0..0, false)]);
        assert_eq!(url_segments("http"), vec![(0..4, false)]);
        // Non-ASCII text right after a would-be scheme does not panic.
        assert_eq!(
            url_segments("httpé://x"),
            vec![(0.."httpé://x".len(), false)]
        );
        assert_eq!(url_segments("httpsé"), vec![(0.."httpsé".len(), false)]);
    }

    #[test]
    fn focus_submit_and_blur() {
        let mut bar = AddressBar::default();
        bar.focus("https://example.com/");
        assert!(bar.focused);
        assert_eq!(bar.edit.selected_text(), "https://example.com/");
        bar.edit.insert("rust lang");
        let url = bar.submit();
        assert!(!bar.focused);
        assert!(url.as_str().contains("rust"), "search URL: {url}");
        bar.focus("");
        bar.edit.insert("about:help");
        assert_eq!(bar.submit().as_str(), "about:help");
    }

    #[test]
    fn caret_scrolling() {
        // Short text: never scrolls.
        assert_eq!(scroll_to_caret(0.0, 50.0, 80.0, 200.0), 0.0);
        // Caret past the right edge: scrolls just enough.
        assert_eq!(scroll_to_caret(0.0, 350.0, 400.0, 200.0), 150.0);
        // Caret left of the view: scrolls back to it.
        assert_eq!(scroll_to_caret(150.0, 100.0, 400.0, 200.0), 100.0);
        // Caret visible: unchanged.
        assert_eq!(scroll_to_caret(100.0, 200.0, 400.0, 200.0), 100.0);
        // Text shrank: no blank space at the end.
        assert_eq!(scroll_to_caret(150.0, 250.0, 260.0, 200.0), 60.0);
    }
}
