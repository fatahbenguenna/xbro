//! Internal `about:` pages.

use url::Url;

use crate::error::NetError;
use crate::response::Response;

/// The start page (`about:home`): search form, quick links and shortcuts.
pub(crate) const HOME_HTML: &str = include_str!("home.html");

/// The keyboard shortcut reference (`about:help`).
pub(crate) const HELP_HTML: &str = include_str!("help.html");

/// Loads `about:blank`, `about:home` or `about:help` (case-insensitive; any
/// query or fragment is ignored). Other pages are [`NetError::NotFound`].
pub(crate) fn fetch(url: &Url) -> Result<Response, NetError> {
    let page = url.path().to_ascii_lowercase();
    let body = match page.as_str() {
        "blank" => "",
        "home" => HOME_HTML,
        "help" => HELP_HTML,
        _ => return Err(NetError::NotFound),
    };
    Ok(Response::ok(
        url.clone(),
        "text/html",
        Some("utf-8".to_owned()),
        body.as_bytes().to_vec(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn about(s: &str) -> Result<Response, NetError> {
        fetch(&Url::parse(s).unwrap())
    }

    /// Minimal well-formedness check: every opened non-void element is closed
    /// in order (the pages are hand-written, so keep them strict).
    fn assert_balanced(html: &str) {
        const VOID: &[&str] = &["meta", "input", "br", "hr", "img", "link"];
        let mut stack: Vec<String> = Vec::new();
        let mut rest = html;
        while let Some(start) = rest.find('<') {
            rest = &rest[start + 1..];
            let end = rest.find('>').expect("unterminated tag");
            let tag = &rest[..end];
            rest = &rest[end + 1..];
            if tag.starts_with('!') {
                continue;
            }
            if let Some(name) = tag.strip_prefix('/') {
                let open = stack.pop().unwrap_or_default();
                assert_eq!(open, name.trim(), "mismatched </{name}>");
                continue;
            }
            let name: String = tag
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            if name == "style" {
                let close = rest.find("</style>").expect("unclosed style");
                rest = &rest[close + "</style>".len()..];
                continue;
            }
            if !VOID.contains(&name.as_str()) && !tag.ends_with('/') {
                stack.push(name);
            }
        }
        assert!(stack.is_empty(), "unclosed elements: {stack:?}");
    }

    #[test]
    fn blank_is_empty_html() {
        let r = about("about:blank").unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.content_type.as_deref(), Some("text/html"));
        assert!(r.body.is_empty());
        assert!(r.is_html());
        assert_eq!(r.url.as_str(), "about:blank");
    }

    #[test]
    fn home_page_content() {
        let r = about("about:home").unwrap();
        assert_eq!(r.content_type.as_deref(), Some("text/html"));
        assert_eq!(r.charset.as_deref(), Some("utf-8"));
        let html = String::from_utf8(r.body).unwrap();
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<title>"));
        assert!(html.contains("xbro"));
        assert!(html.contains("<form action=\"https://html.duckduckgo.com/html/\" method=\"get\""));
        assert!(html.contains("<input name=\"q\""));
        assert!(html.contains("<button"));
        for link in [
            "https://en.wikipedia.org",
            "https://news.ycombinator.com",
            "https://www.rust-lang.org",
            "https://lite.duckduckgo.com",
            "http://info.cern.ch",
            "https://text.npr.org",
        ] {
            assert!(html.contains(&format!("href=\"{link}")), "missing {link}");
        }
        assert!(html.contains("about:help"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("display: grid"));
        assert_balanced(&html);
    }

    #[test]
    fn help_page_content() {
        let r = about("about:help").unwrap();
        let html = String::from_utf8(r.body).unwrap();
        assert!(html.starts_with("<!DOCTYPE html>"));
        for needle in [
            "Ctrl+L",
            "Enter",
            "Alt+←",
            "Alt+→",
            "F5",
            "Ctrl+R",
            "Ctrl+T",
            "Ctrl+W",
            "Ctrl+Tab",
            "Ctrl+F",
            "Ctrl+Plus",
            "Ctrl+Minus",
            "Ctrl+0",
            "Space",
            "Page Up",
            "Page Down",
            "Home",
            "End",
            "<kbd>Tab</kbd>",
        ] {
            assert!(html.contains(needle), "missing {needle}");
        }
        assert!(html.contains("href=\"about:home\""));
        assert!(!html.contains("<script"));
        assert_balanced(&html);
    }

    #[test]
    fn case_insensitive_and_ignores_query() {
        assert!(about("about:HOME").is_ok());
        assert!(about("about:help#keys").is_ok());
        assert!(about("about:blank?x=1").is_ok());
    }

    #[test]
    fn unknown_pages_are_not_found() {
        assert_eq!(about("about:config"), Err(NetError::NotFound));
        assert_eq!(about("about:"), Err(NetError::NotFound));
        assert_eq!(about("about:home/extra"), Err(NetError::NotFound));
    }
}
