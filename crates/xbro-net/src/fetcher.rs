//! [`Fetcher`]: the scheme dispatcher shared by the whole browser.

use std::time::Duration;

use url::Url;

use crate::error::NetError;
use crate::http::HttpClient;
use crate::response::Response;
use crate::{about, data_url, file, locale};

/// Settings for [`Fetcher::with_options`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetcherOptions {
    /// Deadline for a whole HTTP load, redirects and body included
    /// (`Duration::ZERO`, or a value of more than a century such as
    /// `Duration::MAX`, disables it). Default: 30 s.
    pub timeout: Duration,
    /// Whether to honour the environment's network settings: the proxies in
    /// `HTTPS_PROXY`/`HTTP_PROXY`/`ALL_PROXY` (lowercase names too) with the
    /// `NO_PROXY` exceptions (host names, domain suffixes, IPs, CIDR ranges,
    /// ports; loopback hosts such as `localhost` and `127.0.0.1` are always
    /// reached directly), and `SSL_CERT_FILE`, a PEM bundle that replaces the built-in
    /// Mozilla root certificates (needed behind TLS-intercepting proxies).
    /// Default: `true`.
    pub use_env_proxy: bool,
    /// Largest accepted body in bytes (HTTP, `file:` and `data:`);
    /// larger resources fail with [`NetError::TooLarge`]. Default: 50 MiB.
    pub max_body: usize,
}

impl Default for FetcherOptions {
    fn default() -> Self {
        FetcherOptions {
            timeout: Duration::from_secs(30),
            use_env_proxy: true,
            max_body: 50 * 1024 * 1024,
        }
    }
}

/// Thread-safe resource loader (share it with `Arc`).
#[derive(Debug)]
pub struct Fetcher {
    http: HttpClient,
    max_body: usize,
}

impl Fetcher {
    /// HTTP agent with: 30 s global timeout, 10 redirects, cookies, gzip/brotli,
    /// proxy from the environment (`HTTPS_PROXY`/`NO_PROXY`), a 50 MiB body limit,
    /// `Accept`/`Accept-Language` headers, and HTTP error statuses returned as
    /// normal responses (so error pages can be displayed).
    pub fn new() -> Self {
        Self::with_options(FetcherOptions::default())
    }

    /// Like [`Fetcher::new`] with explicit settings. `Accept-Language` is
    /// derived from `LC_ALL`/`LC_MESSAGES`/`LANG` (e.g. `fr_FR.UTF-8` →
    /// `fr-FR,fr;q=0.9,en;q=0.8`; default `en-US,en;q=0.9`).
    pub fn with_options(options: FetcherOptions) -> Self {
        Fetcher {
            http: HttpClient::new(&options, locale::accept_language_from_env()),
            max_body: options.max_body,
        }
    }

    /// The `Accept-Language` value sent with HTTP requests.
    pub fn accept_language(&self) -> &str {
        self.http.accept_language()
    }

    /// Loads `url`. HTTP 4xx/5xx are returned as `Ok` responses with that status.
    ///
    /// * `http:`/`https:` — network load (redirects followed, cookies kept);
    ///   the MIME type comes from `Content-Type` or is sniffed from the body.
    /// * `file:` — the file (MIME type from the extension, else sniffed) or an
    ///   HTML listing for a directory; a missing path is [`NetError::NotFound`].
    /// * `data:` — decoded per the Fetch standard.
    /// * `about:blank`, `about:home`, `about:help` — internal pages; other
    ///   `about:` pages are [`NetError::NotFound`].
    /// * anything else — [`NetError::UnsupportedScheme`].
    pub fn fetch(&self, url: &Url) -> Result<Response, NetError> {
        match url.scheme() {
            "http" | "https" => self.http.get(url),
            "file" => file::fetch(url, self.max_body),
            "data" => {
                let response = data_url::fetch(url)?;
                if response.body.len() > self.max_body {
                    return Err(NetError::TooLarge);
                }
                Ok(response)
            }
            "about" => about::fetch(url),
            other => Err(NetError::UnsupportedScheme(other.to_owned())),
        }
    }

    /// Submits a form with `application/x-www-form-urlencoded` POST.
    ///
    /// Only `http:`/`https:` can receive a body: for other schemes the fields
    /// are ignored and the URL is loaded as with [`Fetcher::fetch`].
    pub fn post_form(&self, url: &Url, fields: &[(String, String)]) -> Result<Response, NetError> {
        match url.scheme() {
            "http" | "https" => self.http.post_form(url, crate::encode_form(fields)),
            _ => self.fetch(url),
        }
    }
}

impl Default for Fetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fetcher() -> Fetcher {
        Fetcher::with_options(FetcherOptions {
            use_env_proxy: false,
            ..FetcherOptions::default()
        })
    }

    #[test]
    fn fetcher_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Fetcher>();
    }

    #[test]
    fn default_options() {
        let o = FetcherOptions::default();
        assert_eq!(o.timeout, Duration::from_secs(30));
        assert!(o.use_env_proxy);
        assert_eq!(o.max_body, 50 * 1024 * 1024);
        // Constructing the default fetcher (with environment proxies) works.
        let f = Fetcher::default();
        assert!(!f.accept_language().is_empty());
    }

    #[test]
    fn unsupported_schemes() {
        let f = fetcher();
        for (url, scheme) in [
            ("ftp://example.com/file", "ftp"),
            ("mailto:someone@example.com", "mailto"),
            ("javascript:alert(1)", "javascript"),
            ("gopher://example.com", "gopher"),
        ] {
            assert_eq!(
                f.fetch(&Url::parse(url).unwrap()),
                Err(NetError::UnsupportedScheme(scheme.into()))
            );
        }
        assert_eq!(
            f.post_form(&Url::parse("ftp://x.org/").unwrap(), &[]),
            Err(NetError::UnsupportedScheme("ftp".into()))
        );
    }

    #[test]
    fn dispatches_internal_schemes() {
        let f = fetcher();
        let home = f.fetch(&Url::parse("about:home").unwrap()).unwrap();
        assert!(home.is_html());
        assert_eq!(
            f.fetch(&Url::parse("about:nope").unwrap()),
            Err(NetError::NotFound)
        );
        let data = f
            .fetch(&Url::parse("data:text/plain;base64,aGk=").unwrap())
            .unwrap();
        assert_eq!(data.body, b"hi");
        assert!(matches!(
            f.fetch(&Url::parse("data:nocomma").unwrap()),
            Err(NetError::Invalid(_))
        ));
        // POST to a non-HTTP URL just loads it.
        let posted = f
            .post_form(&Url::parse("data:,x").unwrap(), &[("a".into(), "b".into())])
            .unwrap();
        assert_eq!(posted.body, b"x");
    }

    #[test]
    fn data_urls_respect_the_body_limit() {
        let f = Fetcher::with_options(FetcherOptions {
            use_env_proxy: false,
            max_body: 4,
            ..FetcherOptions::default()
        });
        assert!(f.fetch(&Url::parse("data:,1234").unwrap()).is_ok());
        assert_eq!(
            f.fetch(&Url::parse("data:,12345").unwrap()),
            Err(NetError::TooLarge)
        );
    }
}
