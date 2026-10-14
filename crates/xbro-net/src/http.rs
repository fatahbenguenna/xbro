//! `http:` and `https:` loading on top of ureq.
//!
//! Redirects are followed here rather than inside ureq so that every hop gets
//! its own proxy decision (see [`crate::proxy`]), only HTTP(S) targets are
//! accepted, the fragment is carried over, and the whole chain shares one
//! deadline. Cookies are stored by ureq's agent for every response, including
//! the intermediate redirects.

use std::io::{self, Read};
use std::time::{Duration, Instant};

use ureq::http::{header, Response as HttpResponse};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::{Agent, Body};
use url::Url;

use crate::error::NetError;
use crate::fetcher::FetcherOptions;
use crate::proxy::ProxySettings;
use crate::response::Response;
use crate::{mime, tls};
use crate::{ACCEPT, USER_AGENT};

/// Maximum number of redirects followed for one request.
pub(crate) const MAX_REDIRECTS: u32 = 10;

const FORM_CONTENT_TYPE: &str = "application/x-www-form-urlencoded";

/// Blocking HTTP client shared by all loads of a [`crate::Fetcher`].
pub(crate) struct HttpClient {
    agent: Agent,
    proxies: ProxySettings,
    accept_language: String,
    /// `None` disables the deadline.
    timeout: Option<Duration>,
    max_body: usize,
}

impl std::fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpClient")
            .field("proxies", &self.proxies)
            .field("accept_language", &self.accept_language)
            .field("timeout", &self.timeout)
            .field("max_body", &self.max_body)
            .finish_non_exhaustive()
    }
}

impl HttpClient {
    /// A client configured from `options`; with `use_env_proxy`, proxies and
    /// root certificates come from the environment (see [`crate::proxy`] and
    /// [`crate::tls`]).
    pub(crate) fn new(options: &FetcherOptions, accept_language: String) -> Self {
        if options.use_env_proxy {
            Self::with_environment(
                options,
                accept_language,
                ProxySettings::from_env(),
                tls::root_certs_from_env(),
            )
        } else {
            Self::with_environment(options, accept_language, ProxySettings::none(), None)
        }
    }

    /// Like [`HttpClient::new`] with explicit proxy settings and root
    /// certificates (`None` keeps ureq's built-in Mozilla roots).
    pub(crate) fn with_environment(
        options: &FetcherOptions,
        accept_language: String,
        proxies: ProxySettings,
        root_certs: Option<RootCerts>,
    ) -> Self {
        let timeout = effective_timeout(options.timeout);
        let mut builder = Agent::config_builder()
            .http_status_as_error(false)
            // Redirects are followed by `execute`.
            .max_redirects(0)
            .timeout_global(timeout)
            .user_agent(USER_AGENT)
            .accept(ACCEPT)
            // The proxy is chosen per request; never let ureq pick one itself.
            .proxy(None);
        if let Some(roots) = root_certs {
            builder = builder.tls_config(TlsConfig::builder().root_certs(roots).build());
        }
        let config = builder.build();
        HttpClient {
            agent: Agent::new_with_config(config),
            proxies,
            accept_language,
            timeout,
            max_body: options.max_body,
        }
    }

    /// The `Accept-Language` header value.
    pub(crate) fn accept_language(&self) -> &str {
        &self.accept_language
    }

    /// GET `url`, following redirects.
    pub(crate) fn get(&self, url: &Url) -> Result<Response, NetError> {
        self.execute(url, None)
    }

    /// POST an `application/x-www-form-urlencoded` body to `url`, following redirects.
    pub(crate) fn post_form(&self, url: &Url, body: String) -> Result<Response, NetError> {
        self.execute(url, Some(body))
    }

    fn execute(&self, url: &Url, mut form: Option<String>) -> Result<Response, NetError> {
        let start = Instant::now();
        let mut current = url.clone();
        let mut redirects = 0;
        loop {
            let remaining = match self.timeout {
                Some(total) => Some(
                    total
                        .checked_sub(start.elapsed())
                        .filter(|d| !d.is_zero())
                        .ok_or_else(|| timed_out(&current))?,
                ),
                None => None,
            };
            let response = self.send_once(&current, form.as_deref(), remaining)?;
            let status = response.status().as_u16();
            if let Some(next) = redirect_target(&response, &current)? {
                if redirects >= MAX_REDIRECTS {
                    return Err(NetError::TooManyRedirects);
                }
                redirects += 1;
                // 303 always becomes GET; 301/302 turn a POST into a GET
                // (as browsers do); 307/308 replay the request.
                if matches!(status, 301..=303) {
                    form = None;
                }
                current = next;
                continue;
            }
            return self.read_response(current, response);
        }
    }

    fn send_once(
        &self,
        url: &Url,
        form: Option<&str>,
        timeout: Option<Duration>,
    ) -> Result<HttpResponse<Body>, NetError> {
        let target = request_target(url)?;
        let proxy = self.proxies.proxy_for(url);
        let result = match form {
            None => self
                .agent
                .get(target)
                .header(header::ACCEPT_LANGUAGE, self.accept_language.as_str())
                .config()
                .proxy(proxy)
                .timeout_global(timeout)
                .build()
                .call(),
            Some(body) => self
                .agent
                .post(target)
                .header(header::ACCEPT_LANGUAGE, self.accept_language.as_str())
                .content_type(FORM_CONTENT_TYPE)
                .config()
                .proxy(proxy)
                .timeout_global(timeout)
                .build()
                .send(body.as_bytes()),
        };
        result.map_err(|err| map_error(err, url))
    }

    fn read_response(
        &self,
        url: Url,
        mut response: HttpResponse<Body>,
    ) -> Result<Response, NetError> {
        let status = response.status().as_u16();
        let content_type_values: Vec<String> = response
            .headers()
            .get_all(header::CONTENT_TYPE)
            .iter()
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect();
        let body = if status_has_no_body(status) {
            // 1xx/204/205/304 never carry content, whatever the headers say
            // (a `Content-Encoding` on them must not trigger a decoding error).
            Vec::new()
        } else {
            read_body(response.body_mut(), self.max_body, &url)?
        };
        let parsed = mime::extract_mime_type(content_type_values.iter().map(String::as_str));
        let (content_type, charset) = match parsed {
            Some(m) if !mime::is_unknown_type(&m.essence) => (m.essence, m.charset),
            Some(m) => (mime::sniff(&body).to_owned(), m.charset),
            None => (mime::sniff(&body).to_owned(), None),
        };
        Ok(Response {
            url,
            status,
            content_type: Some(content_type),
            charset,
            body,
        })
    }
}

/// Deadlines longer than this are treated as "no deadline": ureq adds the
/// timeout to `Instant::now()`, which panics when the sum overflows (for
/// example with `Duration::MAX`).
const MAX_TIMEOUT: Duration = Duration::from_secs(100 * 365 * 24 * 3600);

/// The deadline to give ureq for `timeout` (`None` for zero or absurdly long values).
fn effective_timeout(timeout: Duration) -> Option<Duration> {
    Some(timeout).filter(|t| !t.is_zero() && *t <= MAX_TIMEOUT)
}

/// Statuses whose responses have no content by definition (RFC 9110).
fn status_has_no_body(status: u16) -> bool {
    matches!(status, 100..=199 | 204 | 205 | 304)
}

/// Reads a response body, decompressing it, and fails with
/// [`NetError::TooLarge`] when the **decompressed** size exceeds `max_body`.
///
/// ureq's own limit counts the bytes received *before* decompression, so it
/// cannot stop a compression bomb (a few KiB of gzip expanding to gigabytes);
/// it is kept as a looser bound on the raw stream (which also stops an endless
/// stream of empty gzip members), and the decoded bytes are capped here.
///
/// An empty or truncated-at-the-start compressed body (`Content-Encoding:
/// gzip` with no content, as some servers send) decodes to an empty body
/// instead of an error, as in browsers.
fn read_body(body: &mut Body, max_body: usize, url: &Url) -> Result<Vec<u8>, NetError> {
    let max = u64::try_from(max_body).unwrap_or(u64::MAX);
    let raw_limit = max.saturating_mul(2).saturating_add(64 * 1024);
    let mut reader = body.with_config().limit(raw_limit).reader();
    let mut decoded = Vec::new();
    // One byte over the limit tells "exactly at the limit" from "too large".
    let result = (&mut reader)
        .take(max.saturating_add(1))
        .read_to_end(&mut decoded);
    if let Err(err) = result {
        match ureq::Error::from(err) {
            ureq::Error::Decompress(_, inner)
                if decoded.is_empty() && inner.kind() == io::ErrorKind::UnexpectedEof => {}
            other => return Err(map_error(other, url)),
        }
    }
    if decoded.len() > max_body {
        return Err(NetError::TooLarge);
    }
    Ok(decoded)
}

/// The URL to put on the request line: no fragment, and any byte that
/// `http::Uri` rejects percent-encoded (the `url` crate leaves a few of them,
/// such as `{` or `|`, unescaped).
fn request_target(url: &Url) -> Result<String, NetError> {
    let mut without_fragment = url.clone();
    without_fragment.set_fragment(None);
    let raw = without_fragment.as_str();
    let mut target = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_graphic()
            && !matches!(
                b,
                b'"' | b'<' | b'>' | b'\\' | b'^' | b'`' | b'{' | b'|' | b'}'
            )
        {
            target.push(char::from(b));
        } else {
            target.push_str(&format!("%{b:02X}"));
        }
    }
    // `http::Uri` cannot represent longer URIs.
    if target.len() >= usize::from(u16::MAX) {
        return Err(NetError::Invalid("URL too long".into()));
    }
    Ok(target)
}

/// The next URL when `response` is a redirect with a usable `Location`.
fn redirect_target(response: &HttpResponse<Body>, current: &Url) -> Result<Option<Url>, NetError> {
    if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
        return Ok(None);
    }
    let Some(location) = response.headers().get(header::LOCATION) else {
        return Ok(None);
    };
    let location = decode_location(location.as_bytes());
    let location = location.trim_matches(|c: char| c.is_ascii_whitespace());
    if location.is_empty() {
        return Ok(None);
    }
    let mut next = current.join(location).map_err(|_| {
        NetError::Invalid(format!(
            "invalid redirect location: {}",
            shorten(location, 200)
        ))
    })?;
    if !matches!(next.scheme(), "http" | "https") {
        return Err(NetError::Invalid(format!(
            "refusing to follow a redirect to a {}: URL",
            next.scheme()
        )));
    }
    if next.fragment().is_none() {
        next.set_fragment(current.fragment());
    }
    Ok(Some(next))
}

/// The text of a `Location` header. UTF-8 is used as is; a header that is not
/// valid UTF-8 (a legacy server sending Latin-1 bytes) has its non-ASCII bytes
/// percent-encoded unchanged, so the server receives the bytes it sent, as
/// browsers do (instead of U+FFFD replacement characters).
fn decode_location(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(bytes.len() * 3);
    for &b in bytes {
        if b.is_ascii() {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Truncates `s` to at most `max` characters for display.
fn shorten(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

fn host_of(url: &Url) -> String {
    url.host_str().unwrap_or("the server").to_owned()
}

fn host_port_of(url: &Url) -> String {
    match url.port_or_known_default() {
        Some(port) => format!("{}:{port}", host_of(url)),
        None => host_of(url),
    }
}

fn timed_out(url: &Url) -> NetError {
    NetError::Io(format!("connection to {} timed out", host_of(url)))
}

/// Whether an I/O error message comes from a failed DNS lookup.
fn is_dns_failure(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "failed to lookup address",
        "name or service not known",
        "nodename nor servname",
        "no address associated",
        "temporary failure in name resolution",
        "no such host",
        "name resolution",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

/// Maps a ureq error to a human-readable [`NetError`].
pub(crate) fn map_error(err: ureq::Error, url: &Url) -> NetError {
    let host = host_of(url);
    match err {
        ureq::Error::HostNotFound => NetError::Io(format!("could not resolve host {host}")),
        ureq::Error::Timeout(ureq::Timeout::Resolve) => {
            NetError::Io(format!("could not resolve host {host} (timed out)"))
        }
        ureq::Error::Timeout(ureq::Timeout::Connect) => {
            NetError::Io(format!("connection to {} timed out", host_port_of(url)))
        }
        ureq::Error::Timeout(_) => timed_out(url),
        ureq::Error::Io(e) => map_io_error(&e, url),
        ureq::Error::TooManyRedirects => NetError::TooManyRedirects,
        ureq::Error::BodyExceedsLimit(_) => NetError::TooLarge,
        ureq::Error::BadUri(uri) => NetError::Invalid(format!("bad URL: {}", shorten(&uri, 200))),
        ureq::Error::Http(e) => NetError::Invalid(format!("could not build the request: {e}")),
        ureq::Error::Protocol(e) => NetError::Io(format!("invalid HTTP response from {host}: {e}")),
        ureq::Error::Tls(msg) => NetError::Io(format!("TLS error with {host}: {msg}")),
        ureq::Error::ConnectProxyFailed(msg) => NetError::Io(format!(
            "could not reach {} through the proxy: {msg}",
            host_port_of(url)
        )),
        ureq::Error::LargeResponseHeader(..) => {
            NetError::Io(format!("response headers from {host} are too large"))
        }
        ureq::Error::Decompress(encoding, err) => NetError::Io(format!(
            "could not decode the {encoding} response from {host}: {err}"
        )),
        ureq::Error::ConnectionFailed | ureq::Error::TlsRequired => {
            NetError::Io(format!("could not connect to {}", host_port_of(url)))
        }
        other => {
            let message = other.to_string();
            if let Some(tls) = message.strip_prefix("rustls: ") {
                NetError::Io(format!("TLS error with {host}: {tls}"))
            } else {
                NetError::Io(format!("error while loading {host}: {message}"))
            }
        }
    }
}

fn map_io_error(err: &io::Error, url: &Url) -> NetError {
    let host = host_of(url);
    let target = host_port_of(url);
    let message = match err.kind() {
        io::ErrorKind::ConnectionRefused => format!("connection refused by {target}"),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
            format!("connection to {host} timed out")
        }
        io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::BrokenPipe => format!("connection to {target} was reset"),
        io::ErrorKind::UnexpectedEof => format!("connection to {target} closed unexpectedly"),
        io::ErrorKind::HostUnreachable | io::ErrorKind::NetworkUnreachable => {
            format!("{target} is unreachable")
        }
        io::ErrorKind::AddrNotAvailable => format!("could not connect to {target}"),
        io::ErrorKind::InvalidData if url.scheme() == "https" => {
            format!("TLS error with {host}: {err}")
        }
        _ if is_dns_failure(&err.to_string()) => format!("could not resolve host {host}"),
        _ => format!("network error while loading {target}: {err}"),
    };
    NetError::Io(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn io_message(err: NetError) -> String {
        match err {
            NetError::Io(msg) => msg,
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn request_target_strips_fragment_and_escapes() {
        assert_eq!(
            request_target(&url("https://example.com/a?b=1#frag")).unwrap(),
            "https://example.com/a?b=1"
        );
        assert_eq!(
            request_target(&url("https://example.com/{x}?q=a|b^c`d")).unwrap(),
            "https://example.com/%7Bx%7D?q=a%7Cb%5Ec%60d"
        );
        let uri: ureq::http::Uri = request_target(&url("https://example.com/{}|?{}|\\"))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(uri.host(), Some("example.com"));
        let long = format!("https://example.com/{}", "a".repeat(70_000));
        assert!(matches!(
            request_target(&url(&long)),
            Err(NetError::Invalid(_))
        ));
    }

    #[test]
    fn error_messages() {
        let u = url("https://example.org/page");
        assert_eq!(
            io_message(map_error(ureq::Error::HostNotFound, &u)),
            "could not resolve host example.org"
        );
        let refused = io::Error::new(io::ErrorKind::ConnectionRefused, "refused");
        assert_eq!(
            io_message(map_error(
                ureq::Error::Io(refused),
                &url("http://127.0.0.1:9/")
            )),
            "connection refused by 127.0.0.1:9"
        );
        assert!(
            io_message(map_error(ureq::Error::Timeout(ureq::Timeout::Global), &u))
                .contains("timed out")
        );
        assert!(
            io_message(map_error(ureq::Error::Timeout(ureq::Timeout::Connect), &u))
                .contains("example.org:443 timed out")
        );
        assert!(
            io_message(map_error(ureq::Error::Timeout(ureq::Timeout::Resolve), &u))
                .starts_with("could not resolve host example.org")
        );
        assert!(io_message(map_error(ureq::Error::Tls("bad cert"), &u))
            .starts_with("TLS error with example.org"));
        let dns =
            io::Error::other("failed to lookup address information: Name or service not known");
        assert_eq!(
            io_message(map_error(ureq::Error::Io(dns), &u)),
            "could not resolve host example.org"
        );
        let tls = io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid peer certificate: UnknownIssuer",
        );
        assert!(io_message(map_error(ureq::Error::Io(tls), &u)).starts_with("TLS error"));
        let reset = io::Error::new(io::ErrorKind::ConnectionReset, "reset");
        assert!(io_message(map_error(ureq::Error::Io(reset), &u)).contains("was reset"));
        assert!(
            io_message(map_error(ureq::Error::ConnectProxyFailed("502".into()), &u))
                .contains("through the proxy")
        );
        assert_eq!(
            map_error(ureq::Error::TooManyRedirects, &u),
            NetError::TooManyRedirects
        );
        assert_eq!(
            map_error(ureq::Error::BodyExceedsLimit(10), &u),
            NetError::TooLarge
        );
        assert!(matches!(
            map_error(ureq::Error::BadUri("x".into()), &u),
            NetError::Invalid(_)
        ));
        assert!(io_message(map_error(ureq::Error::ConnectionFailed, &u))
            .contains("could not connect to example.org:443"));
    }

    #[test]
    fn dns_failure_detection() {
        assert!(is_dns_failure(
            "failed to lookup address information: Name or service not known"
        ));
        assert!(is_dns_failure("Temporary failure in name resolution"));
        assert!(!is_dns_failure("connection refused"));
    }

    #[test]
    fn hosts_that_http_uri_rejects_are_errors() {
        let options = FetcherOptions {
            use_env_proxy: false,
            timeout: Duration::from_secs(5),
            ..FetcherOptions::default()
        };
        let client = HttpClient::new(&options, "en".to_owned());
        let mut parsed = 0;
        for raw in [
            "http://a{b}.test/",
            "http://a`b.test/",
            "http://a\"b.test/x",
        ] {
            let Ok(u) = Url::parse(raw) else { continue };
            parsed += 1;
            match client.get(&u) {
                Err(NetError::Invalid(_) | NetError::Io(_)) => {}
                other => panic!("{u}: expected an error, got {other:?}"),
            }
        }
        // The `url` crate accepts these host names, so the path is exercised.
        assert!(parsed > 0);
    }

    #[test]
    fn location_bytes() {
        assert_eq!(decode_location(b"/plain?q=1"), "/plain?q=1");
        assert_eq!(decode_location("/café".as_bytes()), "/café");
        assert_eq!(decode_location(b"/caf\xE9"), "/caf%E9");
        assert_eq!(decode_location(b"\xFF\xFE/x"), "%FF%FE/x");
        let joined = url("http://a.org/dir/x")
            .join(&decode_location(b"caf\xE9"))
            .unwrap();
        assert_eq!(joined.as_str(), "http://a.org/dir/caf%E9");
    }

    #[test]
    fn bodiless_statuses() {
        for status in [100, 101, 103, 199, 204, 205, 304] {
            assert!(status_has_no_body(status), "{status}");
        }
        for status in [200, 201, 206, 301, 302, 400, 404, 500] {
            assert!(!status_has_no_body(status), "{status}");
        }
    }

    #[test]
    fn timeouts_are_clamped() {
        assert_eq!(effective_timeout(Duration::ZERO), None);
        assert_eq!(effective_timeout(Duration::MAX), None);
        assert_eq!(
            effective_timeout(MAX_TIMEOUT + Duration::from_nanos(1)),
            None
        );
        assert_eq!(effective_timeout(MAX_TIMEOUT), Some(MAX_TIMEOUT));
        assert_eq!(
            effective_timeout(Duration::from_secs(30)),
            Some(Duration::from_secs(30))
        );
        // The largest accepted deadline is representable from now.
        assert!(Instant::now().checked_add(MAX_TIMEOUT * 2).is_some());
    }

    #[test]
    fn decompression_errors_are_readable() {
        let err = ureq::Error::Decompress("gzip", io::Error::other("invalid gzip header"));
        assert_eq!(
            io_message(map_error(err, &url("https://example.org/"))),
            "could not decode the gzip response from example.org: invalid gzip header"
        );
    }

    #[test]
    fn fuzz_request_targets_are_valid_uris() {
        let alphabet = [
            "a", "/", "?", "#", "{", "}", "|", "^", "`", "\"", "<", ">", "\\", "%", "%2", " ", "é",
            "[", "]", "@", ":", "'", "!", "$", "&", "(", ")", "*", "+", ",", ";", "=", "~",
        ];
        let mut rng = crate::test_util::Rng::new(77);
        for _ in 0..10_000 {
            let path = rng.string_from(&alphabet, 20);
            let Ok(u) = Url::parse(&format!("http://example.com/{path}")) else {
                continue;
            };
            let target = request_target(&u).unwrap();
            let uri: ureq::http::Uri = target
                .parse()
                .unwrap_or_else(|e| panic!("{u} -> {target}: {e}"));
            assert_eq!(uri.host(), Some("example.com"));
            assert!(!target.contains('#'));
        }
    }

    #[test]
    fn shorten_respects_char_boundaries() {
        assert_eq!(shorten("ééé", 2), "éé…");
        assert_eq!(shorten("abc", 5), "abc");
    }

    /// Reads an HTTP head byte by byte (so nothing after it is consumed).
    fn read_head(stream: &mut impl io::Read) -> Option<String> {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).ok()? == 0 {
                return None;
            }
            head.push(byte[0]);
        }
        Some(String::from_utf8_lossy(&head).into_owned())
    }

    fn reply(stream: &mut impl io::Write, status: &str, extra: &str, body: &str) {
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }

    #[test]
    fn env_proxy_and_no_proxy_are_honoured_per_hop() {
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        // Direct server, reachable without the proxy.
        let direct = TcpListener::bind("127.0.0.1:0").unwrap();
        let direct_addr = direct.local_addr().unwrap();
        std::thread::spawn(move || {
            for mut stream in direct.incoming().flatten() {
                if read_head(&mut stream).is_some() {
                    reply(&mut stream, "200 OK", "", "direct");
                }
            }
        });

        // Fake CONNECT proxy that records the CONNECT line and then plays
        // the origin server inside the tunnel.
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_addr = proxy.local_addr().unwrap();
        let connects = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = Arc::clone(&connects);
        std::thread::spawn(move || {
            for mut stream in proxy.incoming().flatten() {
                let Some(head) = read_head(&mut stream) else {
                    continue;
                };
                log.lock()
                    .unwrap()
                    .push(head.lines().next().unwrap_or_default().to_owned());
                let _ = stream.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n");
                let Some(request) = read_head(&mut stream) else {
                    continue;
                };
                if request.starts_with("GET /redirect ") {
                    let location = format!("Location: http://{direct_addr}/direct\r\n");
                    reply(&mut stream, "302 Found", &location, "");
                } else {
                    reply(&mut stream, "200 OK", "", "via proxy");
                }
            }
        });

        let proxies = ProxySettings::from_lookup(|name| match name {
            "HTTPS_PROXY" => Some(format!("http://{proxy_addr}")),
            "NO_PROXY" => Some("localhost, 127.0.0.0/8".to_owned()),
            _ => None,
        });
        let options = FetcherOptions {
            timeout: Duration::from_secs(10),
            ..FetcherOptions::default()
        };
        let client = HttpClient::with_environment(&options, "en".to_owned(), proxies, None);

        // A public host goes through the proxy (it resolves the name).
        let r = client.get(&url("http://example.test/page")).unwrap();
        assert_eq!(r.body, b"via proxy");
        assert_eq!(
            connects.lock().unwrap().as_slice(),
            ["CONNECT example.test:80 HTTP/1.1"]
        );

        // A NO_PROXY host is reached directly.
        let direct_url = url(&format!("http://{direct_addr}/x"));
        assert_eq!(client.get(&direct_url).unwrap().body, b"direct");
        assert_eq!(connects.lock().unwrap().len(), 1);

        // Each redirect hop gets its own decision: proxied, then direct.
        let r = client.get(&url("http://example.test/redirect")).unwrap();
        assert_eq!(r.body, b"direct");
        assert_eq!(r.url, url(&format!("http://{direct_addr}/direct")));
        assert_eq!(connects.lock().unwrap().len(), 2);
    }

    #[test]
    fn loopback_requests_skip_a_configured_proxy() {
        use std::net::TcpListener;

        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        std::thread::spawn(move || {
            for mut stream in server.incoming().flatten() {
                if read_head(&mut stream).is_some() {
                    reply(&mut stream, "200 OK", "", "local");
                }
            }
        });
        // A proxy that refuses every connection, and no NO_PROXY at all.
        let dead_port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let proxies = ProxySettings::from_lookup(|name| {
            (name == "HTTPS_PROXY").then(|| format!("http://127.0.0.1:{dead_port}"))
        });
        let options = FetcherOptions {
            timeout: Duration::from_secs(10),
            ..FetcherOptions::default()
        };
        let client = HttpClient::with_environment(&options, "en".to_owned(), proxies, None);
        for host in ["127.0.0.1", "localhost"] {
            let r = client
                .get(&url(&format!("http://{host}:{}/", addr.port())))
                .unwrap();
            assert_eq!(r.body, b"local");
        }
    }

    #[test]
    fn redirect_targets() {
        let current = url("https://example.com/dir/page#top");
        let redirect = |status: u16, location: Option<&str>| {
            let mut builder = HttpResponse::builder().status(status);
            if let Some(l) = location {
                builder = builder.header(header::LOCATION, l);
            }
            let response = builder.body(Body::builder().data("")).unwrap();
            redirect_target(&response, &current)
        };
        assert_eq!(
            redirect(301, Some("other")).unwrap().unwrap().as_str(),
            "https://example.com/dir/other#top"
        );
        assert_eq!(
            redirect(302, Some("/x#new")).unwrap().unwrap().as_str(),
            "https://example.com/x#new"
        );
        assert_eq!(
            redirect(308, Some("http://b.org"))
                .unwrap()
                .unwrap()
                .as_str(),
            "http://b.org/#top"
        );
        assert_eq!(redirect(200, Some("/x")).unwrap(), None);
        assert_eq!(redirect(304, Some("/x")).unwrap(), None);
        assert_eq!(redirect(302, None).unwrap(), None);
        assert_eq!(redirect(302, Some("   ")).unwrap(), None);
        assert!(matches!(
            redirect(302, Some("file:///etc/passwd")),
            Err(NetError::Invalid(_))
        ));
        assert!(matches!(
            redirect(302, Some("http://[::1")),
            Err(NetError::Invalid(_))
        ));
    }
}
