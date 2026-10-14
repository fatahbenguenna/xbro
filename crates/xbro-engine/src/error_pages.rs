//! HTML for error and non-HTML content pages.
//!
//! Every page is a self-contained HTML document with an inline stylesheet
//! (no scripts, no external resources except the image of an image
//! document, which the loader provides already decoded). Every piece of
//! text coming from the outside world (URLs, messages, file contents) is
//! HTML-escaped.

use xbro_net::{NetError, Url};

/// Shared look of the error and information pages.
const CARD_STYLE: &str = "\
html { background: #f3f4f6; }
body { margin: 0; padding: 56px 20px; font-family: system-ui, sans-serif; color: #1f2328; line-height: 1.5; }
.card { max-width: 620px; margin: 0 auto; background: #ffffff; border: 1px solid #d0d7de; border-radius: 12px; padding: 32px 36px 28px; }
.badge { display: inline-block; width: 44px; height: 44px; line-height: 44px; border-radius: 22px; text-align: center; font-size: 26px; font-weight: bold; }
.badge.error { background: #ffe3e3; color: #c92a2a; }
.badge.info { background: #e7f0ff; color: #1a5fd0; }
h1 { font-size: 24px; font-weight: 600; margin: 18px 0 6px; }
.message { font-size: 16px; color: #424a53; margin: 0 0 18px; }
.url { font-family: monospace; font-size: 13px; color: #57606a; background: #f6f8fa; border: 1px solid #d0d7de; border-radius: 6px; padding: 8px 12px; margin: 0 0 18px; overflow-wrap: anywhere; }
.hint { font-size: 14px; color: #57606a; border-top: 1px solid #eaeef2; padding-top: 14px; }
.hint p { margin: 0 0 6px; }
.hint ul { margin: 0; padding-left: 22px; }
.hint li { margin: 3px 0; }
.footer { margin-top: 22px; font-size: 12px; color: #8c959f; }
";

/// Escapes text for HTML content and double- or single-quoted attribute values.
pub(crate) fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            // NUL is not allowed in HTML text; keep the page well-formed.
            '\0' => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out
}

/// A card-style page: badge, heading, message, URL box and optional hint list.
fn card_page(
    title: &str,
    badge: (&str, &str),
    message: &str,
    url: &str,
    hint_intro: &str,
    hints: &[String],
) -> String {
    let mut html = String::with_capacity(4096);
    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>");
    html.push_str(&escape_html(title));
    html.push_str("</title>\n<style>\n");
    html.push_str(CARD_STYLE);
    html.push_str("</style>\n</head>\n<body>\n<div class=\"card\">\n<div class=\"badge ");
    html.push_str(badge.0);
    html.push_str("\">");
    html.push_str(badge.1);
    html.push_str("</div>\n<h1>");
    html.push_str(&escape_html(title));
    html.push_str("</h1>\n<p class=\"message\">");
    html.push_str(&escape_html(message));
    html.push_str("</p>\n");
    if !url.is_empty() {
        html.push_str("<p class=\"url\">");
        html.push_str(&escape_html(url));
        html.push_str("</p>\n");
    }
    if !hints.is_empty() {
        html.push_str("<div class=\"hint\">\n<p>");
        html.push_str(&escape_html(hint_intro));
        html.push_str("</p>\n<ul>\n");
        for hint in hints {
            html.push_str("<li>");
            html.push_str(&escape_html(hint));
            html.push_str("</li>\n");
        }
        html.push_str("</ul>\n</div>\n");
    }
    html.push_str("<p class=\"footer\">xbro</p>\n</div>\n</body>\n</html>\n");
    html
}

/// "Problem loading page" for a request that failed before any response.
pub(crate) fn network_error(url: &Url, error: &NetError) -> String {
    let (message, hints): (String, Vec<String>) = match error {
        NetError::UnsupportedScheme(scheme) => (
            format!("xbro does not know how to open “{scheme}:” addresses."),
            vec![
                "xbro can load http:, https:, file:, data: and about: addresses.".into(),
                "Open this address with another application.".into(),
            ],
        ),
        NetError::NotFound => (
            "The file or page could not be found.".into(),
            vec![
                "Check the address for typing errors.".into(),
                "The file may have been moved, renamed or deleted.".into(),
            ],
        ),
        NetError::Io(msg) => (
            format!("The connection failed: {msg}."),
            vec![
                "Check the address for typing errors such as ww.example.com instead of www.example.com.".into(),
                "Check your network connection.".into(),
                "If your computer or network is protected by a firewall or proxy, make sure xbro is permitted to access the web.".into(),
                "The site may be temporarily unavailable: try again in a few moments.".into(),
            ],
        ),
        NetError::TooManyRedirects => (
            "The page isn’t redirecting properly (too many redirects).".into(),
            vec![
                "The site redirects in a way that will never complete.".into(),
                "Clearing the site’s cookies may help.".into(),
            ],
        ),
        NetError::TooLarge => (
            "The response is too large to be displayed.".into(),
            vec!["Download the resource with another tool.".into()],
        ),
        NetError::Invalid(msg) => (
            format!("The address could not be loaded: {msg}."),
            vec!["Check the address for typing errors.".into()],
        ),
    };
    card_page(
        "Problem loading page",
        ("error", "!"),
        &message,
        url.as_str(),
        "Try:",
        &hints,
    )
}

/// Standard reason phrase of an HTTP status code.
pub(crate) fn reason_phrase(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        418 => "I'm a teapot",
        421 => "Misdirected Request",
        422 => "Unprocessable Content",
        425 => "Too Early",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        507 => "Insufficient Storage",
        508 => "Loop Detected",
        511 => "Network Authentication Required",
        _ => "",
    }
}

/// Page for an HTTP error status whose response had no body to show.
pub(crate) fn http_error(url: &Url, status: u16) -> String {
    let reason = reason_phrase(status);
    let title = if reason.is_empty() {
        format!("Error {status}")
    } else {
        format!("{status} {reason}")
    };
    let (message, hints): (&str, Vec<String>) = match status {
        404 | 410 => (
            "The server could not find the requested page.",
            vec![
                "Check the address for typing errors.".into(),
                "The page may have been moved or deleted.".into(),
            ],
        ),
        401 | 403 => (
            "The server refused to show this page.",
            vec!["You may need to sign in or to have permission to view it.".into()],
        ),
        429 => (
            "The server received too many requests.",
            vec!["Wait a moment before trying again.".into()],
        ),
        s if s >= 500 => (
            "The server encountered an error and could not complete the request.",
            vec!["The site may be temporarily unavailable: try again in a few moments.".into()],
        ),
        _ => (
            "The server answered with an error and an empty page.",
            Vec::new(),
        ),
    };
    card_page(
        &title,
        ("error", "!"),
        message,
        url.as_str(),
        "Try:",
        &hints,
    )
}

/// Human-readable size: `512 bytes`, `1.5 KB`, `3.2 MB`.
pub(crate) fn format_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

/// Page for content xbro cannot render (`application/pdf`, `video/mp4`, …).
pub(crate) fn unsupported_content(url: &Url, content_type: &str, size: usize) -> String {
    let message = format!(
        "xbro cannot display {} ({}).",
        if content_type.is_empty() {
            "this content"
        } else {
            content_type
        },
        format_size(size)
    );
    card_page(
        "Cannot display this file",
        ("info", "?"),
        &message,
        url.as_str(),
        "You can:",
        &["Open the address with an application that supports this type of file.".into()],
    )
}

/// Page for an image that could not be decoded.
pub(crate) fn undecodable_image(url: &Url, content_type: &str, size: usize) -> String {
    let message = format!(
        "The image cannot be displayed because it contains errors or uses an unsupported format ({}, {}).",
        if content_type.is_empty() { "image" } else { content_type },
        format_size(size)
    );
    card_page(
        "Cannot display this image",
        ("error", "!"),
        &message,
        url.as_str(),
        "",
        &[],
    )
}

/// Longest title generated for an image document (in characters).
const MAX_TITLE_CHARS: usize = 100;

/// `text` cut to at most `max` characters (an ellipsis marks the cut).
pub(crate) fn shorten(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/// Last path segment of a URL (percent-decoded), used as a document title.
pub(crate) fn file_name(url: &Url) -> Option<String> {
    let segment = url.path_segments()?.rev().find(|s| !s.is_empty())?;
    let decoded = percent_decode(segment);
    (!decoded.trim().is_empty()).then_some(decoded)
}

/// Lossy UTF-8 percent-decoding (`%20` → space). Invalid escapes are kept.
pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' {
            let hex = |j: usize| bytes.get(j).and_then(|&c| (c as char).to_digit(16));
            if let (Some(h), Some(l)) = (hex(i + 1), hex(i + 2)) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Wraps a non-HTML text resource (plain text, JSON, JavaScript, XML…) in a `<pre>`.
pub(crate) fn plain_text(url: &Url, text: &str) -> String {
    let mut html = String::with_capacity(text.len() + text.len() / 16 + 512);
    html.push_str("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
    if let Some(name) = file_name(url) {
        html.push_str("<title>");
        html.push_str(&escape_html(&name));
        html.push_str("</title>\n");
    }
    html.push_str(
        "<style>\nbody { margin: 8px; background: #ffffff; color: #1f2328; }\n\
         pre { margin: 0; font-family: monospace; font-size: 13px; line-height: 1.45; \
         white-space: pre-wrap; overflow-wrap: anywhere; }\n</style>\n</head>\n<body>\n<pre>",
    );
    // The parser drops one newline right after `<pre>`: keep a leading one.
    if text.starts_with('\n') {
        html.push('\n');
    }
    html.push_str(&escape_html(text));
    html.push_str("</pre>\n</body>\n</html>\n");
    html
}

/// A centered image document. `size` is the decoded natural size, if known.
pub(crate) fn image_document(url: &Url, size: Option<(f32, f32)>) -> String {
    // `data:` URLs have no file name, and can be megabytes long.
    let name = file_name(url).unwrap_or_else(|| shorten(url.as_str(), MAX_TITLE_CHARS));
    let title = match size {
        Some((w, h)) => format!("{name} ({} × {})", w.round(), h.round()),
        None => name,
    };
    let src = escape_html(url.as_str());
    format!(
        "<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n<style>\n\
         html, body {{ margin: 0; height: 100%; background: #1e1f22; }}\n\
         body {{ display: flex; align-items: center; justify-content: center; min-height: 100vh; }}\n\
         img {{ display: block; max-width: 100%; background: #ffffff; }}\n\
         </style>\n</head>\n<body>\n<img src=\"{src}\" alt=\"{src}\">\n</body>\n</html>\n",
        escape_html(&title)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn escaping() {
        assert_eq!(
            escape_html("<a href=\"x\">'&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
        assert_eq!(escape_html("a\0b"), "a\u{FFFD}b");
    }

    #[test]
    fn network_error_page_mentions_url_and_message() {
        let u = url("https://example.org/a?b=<c>");
        let html = network_error(&u, &NetError::Io("connection <refused>".into()));
        assert!(html.contains("Problem loading page"));
        assert!(html.contains("connection &lt;refused&gt;"));
        assert!(!html.contains("<refused>"));
        assert!(!html.contains("<script"));
        let html = network_error(&url("ftp://x"), &NetError::UnsupportedScheme("ftp".into()));
        assert!(html.contains("ftp:"));
    }

    #[test]
    fn sizes_and_names() {
        assert_eq!(format_size(12), "12 bytes");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(
            file_name(&url("https://e.org/dir/my%20file.txt")).as_deref(),
            Some("my file.txt")
        );
        assert_eq!(file_name(&url("https://e.org/")), None);
        assert_eq!(percent_decode("%e2%82%ac%zz%"), "€%zz%");
    }

    #[test]
    fn text_and_image_wrappers() {
        let u = url("file:///tmp/a.txt");
        let html = plain_text(&u, "\nfn main() { a < b && c > d }");
        assert!(html.contains("<pre>\n\nfn main() { a &lt; b &amp;&amp; c &gt; d }</pre>"));
        let html = image_document(&url("https://e.org/i.png?\"x"), Some((64.0, 32.0)));
        assert!(html.contains("i.png (64 × 32)"));
        assert!(html.contains("src=\"https://e.org/i.png?%22x\""));
        let long = url(&format!("data:image/png;base64,{}", "A".repeat(5000)));
        let html = image_document(&long, Some((1.0, 1.0)));
        let title = html
            .split("<title>")
            .nth(1)
            .unwrap()
            .split("</title>")
            .next()
            .unwrap();
        assert!(title.chars().count() < 120, "{title}");
        assert!(title.starts_with("data:image/png;base64,AAA") && title.contains('…'));
        assert_eq!(shorten("héllo", 2), "hé…");
        assert_eq!(shorten("hé", 2), "hé");
        let html = unsupported_content(&url("https://e.org/f.pdf"), "application/pdf", 2048);
        assert!(html.contains("application/pdf (2.0 KB)"));
        let html = http_error(&url("https://e.org/x"), 404);
        assert!(html.contains("404 Not Found"));
    }
}
