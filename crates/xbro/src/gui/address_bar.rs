//! The address bar: an editable field when focused, a formatted URL otherwise.

use std::ops::Range;

use xbro_engine::Url;

use super::line_edit::LineEdit;

/// Text shown for a URL when the address bar is not being edited. The new
/// tab page shows an empty field (with a placeholder).
pub fn display_text(url: Option<&Url>) -> String {
    match url {
        Some(url) if url.as_str() == "about:home" => String::new(),
        Some(url) => url.as_str().to_string(),
        None => String::new(),
    }
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
