//! Resource loading for xbro.
//!
//! Supports `http:`/`https:` (ureq + rustls, redirects, gzip/brotli, cookies),
//! `file:` (files and directory listings), `data:` (base64 and percent-encoded)
//! and the internal `about:` pages (`about:blank`, `about:home`, `about:help`).
//!
//! The entry point is [`Fetcher`]; the free functions handle address-bar input
//! ([`parse_user_input`]), URL resolution ([`resolve`]), text decoding
//! ([`decode_text`]) and form encoding ([`encode_form`]).

mod about;
mod data_url;
mod error;
mod fetcher;
mod file;
mod html;
mod http;
mod input;
mod locale;
mod mime;
mod proxy;
mod response;
mod text;
mod tls;

#[cfg(test)]
mod test_util;

pub use url::Url;

pub use error::NetError;
pub use fetcher::{Fetcher, FetcherOptions};
pub use response::Response;

/// The `User-Agent` sent with every HTTP request.
pub const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (X11; Linux x86_64) xbro/",
    env!("CARGO_PKG_VERSION")
);

/// The `Accept` header sent with every HTTP request (documents and subresources).
pub const ACCEPT: &str =
    "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8";

/// Search engine used for address-bar input that is not a URL (no JavaScript needed).
pub const SEARCH_URL: &str = "https://html.duckduckgo.com/html/?q=";

/// Interprets address-bar input like a browser:
/// - empty → `about:home`;
/// - a valid absolute URL with a known scheme (`https://…`, `about:…`, `file:…`, `data:…`) → itself;
/// - an existing local path (`/tmp/a.html`, `./x.html`, `~/x.html`) → `file://` URL;
/// - `localhost[:port][/…]`, an IPv4 address, or something that looks like a
///   host name (contains a dot, no spaces, e.g. `example.com/path`) → `https://`
///   (`http://` for localhost and IP addresses);
/// - anything else → a search URL ([`SEARCH_URL`] + percent-encoded query).
pub fn parse_user_input(input: &str) -> Url {
    input::parse_user_input(input)
}

/// Resolves `href` against `base` (trimming ASCII whitespace as browsers do).
/// Returns `None` for unparsable URLs. `javascript:` URLs are returned as-is
/// (callers decide to ignore them).
pub fn resolve(base: &Url, href: &str) -> Option<Url> {
    base.join(href.trim_matches(|c: char| c.is_ascii_whitespace()))
        .ok()
}

/// Decodes a text resource (CSS, plain text) given an optional charset label;
/// handles BOMs, defaults to UTF-8, never fails (lossy).
pub fn decode_text(body: &[u8], charset: Option<&str>) -> String {
    text::decode_text(body, charset)
}

/// Percent-encodes form fields as `application/x-www-form-urlencoded`
/// (`a=1&b=hello+world`).
pub fn encode_form(fields: &[(String, String)]) -> String {
    text::encode_form(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_trims_whitespace_and_joins() {
        let base = Url::parse("https://example.com/dir/page.html").unwrap();
        assert_eq!(
            resolve(&base, "  other.html \n").unwrap().as_str(),
            "https://example.com/dir/other.html"
        );
        assert_eq!(
            resolve(&base, "/root").unwrap().as_str(),
            "https://example.com/root"
        );
        assert_eq!(
            resolve(&base, "javascript:void(0)").unwrap().scheme(),
            "javascript"
        );
        assert!(resolve(&base, "http://[::1").is_none());
    }

    #[test]
    fn user_agent_mentions_xbro() {
        assert!(USER_AGENT.starts_with("Mozilla/5.0"));
        assert!(USER_AGENT.contains("xbro/"));
    }
}
