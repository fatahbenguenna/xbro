//! Resource loading for xbro.
//!
//! Supports `http:`/`https:` (ureq + rustls, redirects, gzip/brotli, cookies),
//! `file:` (files and directory listings), `data:` (base64 and percent-encoded)
//! and the internal `about:` pages (`about:blank`, `about:home`, `about:help`).

pub use url::Url;

/// A fetched resource.
#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    /// Final URL after redirects.
    pub url: Url,
    /// HTTP status (200 for non-HTTP schemes that succeeded).
    pub status: u16,
    /// Lowercased MIME type without parameters (`"text/html"`), from the
    /// Content-Type header, the data: URL, or sniffed from the file extension /
    /// content for `file:` URLs and missing headers.
    pub content_type: Option<String>,
    /// `charset` parameter of the Content-Type, if any (lowercased).
    pub charset: Option<String>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn is_html(&self) -> bool {
        matches!(
            self.content_type.as_deref(),
            None | Some("text/html") | Some("application/xhtml+xml")
        )
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetError {
    /// The scheme is not one xbro can load (`ftp:`, `mailto:`, `javascript:`…).
    UnsupportedScheme(String),
    /// DNS, connection, TLS or I/O failure; the message is human-readable.
    Io(String),
    /// Too many redirects.
    TooManyRedirects,
    /// Body larger than the configured limit.
    TooLarge,
    /// Malformed `data:` URL or similar.
    Invalid(String),
    /// Unknown `about:` page or missing file.
    NotFound,
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::UnsupportedScheme(s) => write!(f, "unsupported URL scheme: {s}"),
            NetError::Io(msg) => write!(f, "{msg}"),
            NetError::TooManyRedirects => write!(f, "too many redirects"),
            NetError::TooLarge => write!(f, "response too large"),
            NetError::Invalid(msg) => write!(f, "invalid resource: {msg}"),
            NetError::NotFound => write!(f, "not found"),
        }
    }
}

impl std::error::Error for NetError {}

/// The `User-Agent` sent with every HTTP request.
pub const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (X11; Linux x86_64) xbro/",
    env!("CARGO_PKG_VERSION")
);

/// Thread-safe resource loader (share it with `Arc`).
pub struct Fetcher {
    _private: (),
}

impl Fetcher {
    /// HTTP agent with: 30 s global timeout, 10 redirects, cookies, gzip/brotli,
    /// proxy from the environment (`HTTPS_PROXY`/`NO_PROXY`), a 50 MiB body limit,
    /// `Accept`/`Accept-Language` headers, and HTTP error statuses returned as
    /// normal responses (so error pages can be displayed).
    pub fn new() -> Self {
        todo!("xbro-net: implement Fetcher::new")
    }

    /// Loads `url`. HTTP 4xx/5xx are returned as `Ok` responses with that status.
    pub fn fetch(&self, url: &Url) -> Result<Response, NetError> {
        let _ = url;
        todo!("xbro-net: implement Fetcher::fetch")
    }

    /// Submits a form with `application/x-www-form-urlencoded` POST.
    pub fn post_form(&self, url: &Url, fields: &[(String, String)]) -> Result<Response, NetError> {
        let _ = (url, fields);
        todo!("xbro-net: implement Fetcher::post_form")
    }
}

impl Default for Fetcher {
    fn default() -> Self {
        Self::new()
    }
}

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
    let _ = input;
    todo!("xbro-net: implement parse_user_input")
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
    let _ = (body, charset);
    todo!("xbro-net: implement decode_text")
}

/// Percent-encodes form fields as `application/x-www-form-urlencoded`
/// (`a=1&b=hello+world`).
pub fn encode_form(fields: &[(String, String)]) -> String {
    let _ = fields;
    todo!("xbro-net: implement encode_form")
}
