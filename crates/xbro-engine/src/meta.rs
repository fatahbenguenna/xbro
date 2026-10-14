//! Document metadata: `<base href>` and `<meta http-equiv="refresh">`.

use xbro_dom::Document;
use xbro_net::{resolve, Url};

/// The document base URL: the first `<base href>` resolved against the
/// document URL (ignored when it is a `data:` or `javascript:` URL), else
/// the document URL.
pub(crate) fn base_url(doc: &Document, document_url: &Url) -> Url {
    doc.descendants(doc.root())
        .find(|&n| doc.is_element_named(n, "base") && doc.attr(n, "href").is_some())
        .and_then(|n| doc.attr(n, "href"))
        .and_then(|href| resolve(document_url, href))
        .filter(|u| !matches!(u.scheme(), "data" | "javascript"))
        .unwrap_or_else(|| document_url.clone())
}

/// Parses the `content` of a refresh `<meta>`: `"5"`, `"0; url=/next"`,
/// `"3;URL='page.html'"`, `"1, url=x"`. Returns the delay in seconds and the
/// raw target, if any (HTML "shared declarative refresh steps", lenient).
pub(crate) fn parse_refresh(content: &str) -> Option<(f32, Option<String>)> {
    let s = content.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let digits_end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let number = &s[..digits_end];
    if !number.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let integer: String = number.chars().take_while(char::is_ascii_digit).collect();
    let mut delay: f32 = integer.parse().unwrap_or(f32::MAX);
    if let Some(frac) = number
        .strip_prefix(integer.as_str())
        .and_then(|f| f.strip_prefix('.'))
    {
        if let Ok(f) = format!("0.{}", frac.trim_end_matches('.')).parse::<f32>() {
            delay += f;
        }
    }
    let delay = if delay.is_finite() {
        delay.max(0.0)
    } else {
        f32::MAX
    };
    let mut rest = s[digits_end..].trim_start_matches(|c: char| c.is_ascii_whitespace());
    if rest.is_empty() {
        return Some((delay, None));
    }
    match rest.chars().next() {
        Some(';') | Some(',') => rest = &rest[1..],
        // Anything else after the number: invalid unless it is only whitespace.
        _ => return None,
    }
    rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
    if let Some(after) = rest
        .get(..3)
        .filter(|p| p.eq_ignore_ascii_case("url"))
        .and_then(|_| rest.get(3..))
    {
        let after = after.trim_start_matches(|c: char| c.is_ascii_whitespace());
        if let Some(value) = after.strip_prefix('=') {
            rest = value.trim_start_matches(|c: char| c.is_ascii_whitespace());
        }
    }
    let target = match rest.chars().next() {
        Some(q @ ('"' | '\'')) => {
            let inner = &rest[1..];
            match inner.find(q) {
                Some(end) => &inner[..end],
                None => inner,
            }
        }
        _ => rest,
    };
    let target = target.trim_matches(|c: char| c.is_ascii_whitespace());
    Some((delay, (!target.is_empty()).then(|| target.to_string())))
}

/// The first valid refresh `<meta>` of `doc`: delay and absolute target
/// (`document_url` when the content has no URL).
pub(crate) fn meta_refresh(doc: &Document, base: &Url, document_url: &Url) -> Option<(f32, Url)> {
    for n in doc.descendants(doc.root()) {
        if !doc.is_element_named(n, "meta") {
            continue;
        }
        let refresh = doc
            .attr(n, "http-equiv")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("refresh"));
        if !refresh {
            continue;
        }
        let Some((delay, target)) = doc.attr(n, "content").and_then(parse_refresh) else {
            continue;
        };
        let url = match target {
            Some(t) => match resolve(base, &t) {
                Some(u) => u,
                None => continue,
            },
            None => document_url.clone(),
        };
        return Some((delay, url));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_syntax() {
        assert_eq!(parse_refresh("5"), Some((5.0, None)));
        assert_eq!(
            parse_refresh(" 0; url=/next"),
            Some((0.0, Some("/next".into())))
        );
        assert_eq!(
            parse_refresh("3;URL='page.html?a=1' junk"),
            Some((3.0, Some("page.html?a=1".into())))
        );
        assert_eq!(
            parse_refresh("1, Url = \"x y\""),
            Some((1.0, Some("x y".into())))
        );
        assert_eq!(
            parse_refresh("2.5;http://e.org/"),
            Some((2.5, Some("http://e.org/".into())))
        );
        assert_eq!(parse_refresh("0;"), Some((0.0, None)));
        assert_eq!(parse_refresh("url=x"), None);
        assert_eq!(parse_refresh(""), None);
        assert_eq!(parse_refresh("5x"), None);
        assert_eq!(parse_refresh("1;é"), Some((1.0, Some("é".into()))));
        assert_eq!(
            parse_refresh("1; 日本語"),
            Some((1.0, Some("日本語".into())))
        );
        assert!(parse_refresh("99999999999999999999999999999999999999999").is_some());
    }

    #[test]
    fn base_and_meta() {
        let doc = xbro_html::parse_document(
            "<head><base target=_blank><base href='/sub/'><base href='/other/'>\
             <meta http-equiv=REFRESH content='bad'>\
             <meta http-equiv=Refresh content='7; url=next.html'></head>",
        );
        let url = Url::parse("https://e.org/dir/page.html").unwrap();
        let base = base_url(&doc, &url);
        assert_eq!(base.as_str(), "https://e.org/sub/");
        let (delay, target) = meta_refresh(&doc, &base, &url).unwrap();
        assert_eq!(delay, 7.0);
        assert_eq!(target.as_str(), "https://e.org/sub/next.html");
        let js = xbro_html::parse_document("<base href='javascript:alert(1)'>");
        assert_eq!(base_url(&js, &url), url);
        let plain = xbro_html::parse_document("<meta http-equiv=refresh content=3>");
        assert_eq!(meta_refresh(&plain, &url, &url), Some((3.0, url.clone())));
    }
}
