//! Address-bar input interpretation (URL, local path, host name or search).

use std::fs;
use std::path::{Component, Path, PathBuf};

use url::Url;

use crate::SEARCH_URL;

/// Schemes accepted verbatim when typed as an absolute URL. Schemes xbro
/// cannot load are kept so that the fetcher reports them as unsupported
/// instead of turning `mailto:a@b.c` into a web address.
const KNOWN_SCHEMES: &[&str] = &[
    "http",
    "https",
    "about",
    "file",
    "data",
    "ftp",
    "mailto",
    "javascript",
    "ws",
    "wss",
    "view-source",
];

/// Parses a URL from a compile-time constant.
fn constant_url(s: &'static str) -> Url {
    // The inputs are valid literals covered by unit tests.
    Url::parse(s).expect("constant URL must be valid")
}

/// See [`crate::parse_user_input`].
pub(crate) fn parse_user_input(input: &str) -> Url {
    let text = input.trim();
    if text.is_empty() {
        return constant_url("about:home");
    }
    known_scheme_url(text)
        .or_else(|| local_path_url(text))
        .or_else(|| host_url(text))
        .unwrap_or_else(|| search_url(text))
}

fn known_scheme_url(text: &str) -> Option<Url> {
    let url = Url::parse(text).ok()?;
    if !KNOWN_SCHEMES.contains(&url.scheme()) {
        return None;
    }
    // "http:foo" parses, but a special scheme always needs a host.
    if matches!(url.scheme(), "http" | "https" | "ws" | "wss" | "ftp")
        && url.host_str().is_none_or(str::is_empty)
    {
        return None;
    }
    Some(url)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Resolves `.` and `..` lexically (without following symlinks).
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `file:` URL for an existing local path written as `/…`, `./…`, `../…`,
/// `~` or `~/…`.
fn local_path_url(text: &str) -> Option<Url> {
    let looks_like_path = text.starts_with('/')
        || text.starts_with("./")
        || text.starts_with("../")
        || text == "."
        || text == ".."
        || text == "~"
        || text.starts_with("~/");
    if !looks_like_path {
        return None;
    }
    let expanded = if text == "~" {
        home_dir()?
    } else if let Some(rest) = text.strip_prefix("~/") {
        home_dir()?.join(rest)
    } else {
        PathBuf::from(text)
    };
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir().ok()?.join(expanded)
    };
    let absolute = normalize_lexically(&absolute);
    let metadata = fs::metadata(&absolute).ok()?;
    if metadata.is_dir() {
        Url::from_directory_path(&absolute).ok()
    } else {
        Url::from_file_path(&absolute).ok()
    }
}

/// Splits `host[:port]`, supporting bracketed IPv6 (`[::1]:8080`).
fn split_host_port(authority: &str) -> Option<(&str, Option<&str>)> {
    if authority.starts_with('[') {
        let close = authority.find(']')?;
        let host = &authority[..=close];
        let rest = &authority[close + 1..];
        return match rest.strip_prefix(':') {
            Some(port) => Some((host, Some(port))),
            None if rest.is_empty() => Some((host, None)),
            None => None,
        };
    }
    match authority.split_once(':') {
        Some((host, port)) => Some((host, Some(port))),
        None => Some((authority, None)),
    }
}

fn is_valid_port(port: &str) -> bool {
    !port.is_empty() && port.len() <= 5 && port.parse::<u16>().is_ok()
}

fn is_localhost(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost")
}

/// Dotted-quad IPv4 (`192.168.0.1`); shorthand forms like `3.14` are not
/// treated as addresses.
fn is_ipv4(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u8>().is_ok()
        })
}

fn is_bracketed_ipv6(host: &str) -> bool {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .is_some_and(|h| h.parse::<std::net::Ipv6Addr>().is_ok())
}

/// A plausible domain name: at least two dot-separated labels made of
/// letters, digits, `-` or `_`, ending with an alphabetic top-level domain
/// (or a punycode one).
fn looks_like_domain(host: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let label_ok = |label: &&str| {
        !label.is_empty()
            && label.chars().count() <= 63
            && label
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            && !label.starts_with('-')
            && !label.ends_with('-')
    };
    if !labels.iter().all(label_ok) {
        return false;
    }
    let tld = labels.last().copied().unwrap_or_default();
    let tld_lower = tld.to_lowercase();
    tld_lower.starts_with("xn--")
        || (tld.chars().count() >= 2 && tld.chars().all(char::is_alphabetic))
}

/// `http://` for localhost and IP addresses, `https://` for host names.
fn host_url(text: &str) -> Option<Url> {
    if text.chars().any(char::is_whitespace) {
        return None;
    }
    let authority_end = text.find(['/', '?', '#']).unwrap_or(text.len());
    let authority = &text[..authority_end];
    if authority.contains('@') {
        // Probably an e-mail address: search for it instead.
        return None;
    }
    let (host, port) = split_host_port(authority)?;
    if port.is_some_and(|p| !is_valid_port(p)) {
        return None;
    }
    let scheme = if is_localhost(host) || is_ipv4(host) || is_bracketed_ipv6(host) {
        "http"
    } else if looks_like_domain(host) {
        "https"
    } else {
        return None;
    };
    Url::parse(&format!("{scheme}://{text}"))
        .ok()
        .filter(|url| url.host_str().is_some_and(|h| !h.is_empty()))
}

fn search_url(query: &str) -> Url {
    let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
    Url::parse(&format!("{SEARCH_URL}{encoded}")).unwrap_or_else(|_| constant_url("about:blank"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::Rng;

    fn parse(s: &str) -> String {
        parse_user_input(s).to_string()
    }

    #[test]
    fn constants_are_valid() {
        assert_eq!(constant_url("about:home").as_str(), "about:home");
        assert_eq!(constant_url("about:blank").as_str(), "about:blank");
        assert!(Url::parse(SEARCH_URL).is_ok());
    }

    #[test]
    fn empty_input_is_home() {
        assert_eq!(parse(""), "about:home");
        assert_eq!(parse("   \t\n"), "about:home");
    }

    #[test]
    fn absolute_urls_with_known_schemes() {
        assert_eq!(parse("https://x.y"), "https://x.y/");
        assert_eq!(
            parse("  https://example.com/a b  "),
            "https://example.com/a%20b"
        );
        assert_eq!(parse("HTTP://Example.COM/Path"), "http://example.com/Path");
        assert_eq!(parse("about:home"), "about:home");
        assert_eq!(parse("about:blank"), "about:blank");
        assert_eq!(parse("file:///tmp"), "file:///tmp");
        assert_eq!(
            parse("data:text/html,<b>hi</b>"),
            "data:text/html,<b>hi</b>"
        );
        assert_eq!(
            parse("mailto:someone@example.com"),
            "mailto:someone@example.com"
        );
        assert_eq!(parse("javascript:alert(1)"), "javascript:alert(1)");
    }

    #[test]
    fn bare_host_names_get_https() {
        assert_eq!(parse("example.com"), "https://example.com/");
        assert_eq!(parse("example.com/a?b"), "https://example.com/a?b");
        assert_eq!(parse("www.rust-lang.org"), "https://www.rust-lang.org/");
        assert_eq!(
            parse("example.com:8443/x#frag"),
            "https://example.com:8443/x#frag"
        );
        assert_eq!(parse("sub.example.co.uk."), "https://sub.example.co.uk./");
        assert_eq!(parse("münchen.de"), "https://xn--mnchen-3ya.de/");
        assert_eq!(parse("xn--mnchen-3ya.de"), "https://xn--mnchen-3ya.de/");
    }

    #[test]
    fn localhost_and_ips_get_http() {
        assert_eq!(parse("localhost"), "http://localhost/");
        assert_eq!(parse("localhost:8080"), "http://localhost:8080/");
        assert_eq!(parse("LocalHost:3000/app"), "http://localhost:3000/app");
        assert_eq!(parse("app.localhost"), "http://app.localhost/");
        assert_eq!(parse("127.0.0.1:3000/x"), "http://127.0.0.1:3000/x");
        assert_eq!(parse("192.168.1.1"), "http://192.168.1.1/");
        assert_eq!(parse("[::1]:8080/a"), "http://[::1]:8080/a");
    }

    #[test]
    fn everything_else_is_a_search() {
        assert_eq!(
            parse("rust programming"),
            "https://html.duckduckgo.com/html/?q=rust+programming"
        );
        assert_eq!(parse("rust"), "https://html.duckduckgo.com/html/?q=rust");
        assert_eq!(
            parse("what is 1+1? & more"),
            "https://html.duckduckgo.com/html/?q=what+is+1%2B1%3F+%26+more"
        );
        assert_eq!(parse("3.14"), "https://html.duckduckgo.com/html/?q=3.14");
        assert_eq!(
            parse("someone@example.com"),
            "https://html.duckduckgo.com/html/?q=someone%40example.com"
        );
        assert_eq!(
            parse("café crème"),
            "https://html.duckduckgo.com/html/?q=caf%C3%A9+cr%C3%A8me"
        );
        assert_eq!(
            parse("example.com:notaport"),
            "https://html.duckduckgo.com/html/?q=example.com%3Anotaport"
        );
        assert_eq!(
            parse("localhost:99999"),
            "https://html.duckduckgo.com/html/?q=localhost%3A99999"
        );
        assert_eq!(
            parse("1.2.3.256"),
            "https://html.duckduckgo.com/html/?q=1.2.3.256"
        );
        assert_eq!(
            parse("https://"),
            "https://html.duckduckgo.com/html/?q=https%3A%2F%2F"
        );
        assert_eq!(
            parse("-bad-.com"),
            "https://html.duckduckgo.com/html/?q=-bad-.com"
        );
        assert_eq!(parse("a..b"), "https://html.duckduckgo.com/html/?q=a..b");
        assert_eq!(
            parse("[not-ipv6]:80"),
            "https://html.duckduckgo.com/html/?q=%5Bnot-ipv6%5D%3A80"
        );
    }

    #[test]
    fn nonexistent_paths_are_searched() {
        let url = parse_user_input("/definitely/not/a/real/xbro/path.html");
        assert!(url.as_str().starts_with(SEARCH_URL));
    }

    #[test]
    fn existing_paths_become_file_urls() {
        let dir = std::env::temp_dir().join(format!("xbro-input-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("page one.html");
        fs::write(&file, "<p>hi</p>").unwrap();

        let url = parse_user_input(file.to_str().unwrap());
        assert_eq!(url.scheme(), "file");
        assert_eq!(url.to_file_path().unwrap(), file);
        assert!(url.as_str().ends_with("page%20one.html"));

        // Directories get a trailing slash; `..` is resolved lexically.
        let with_dots = format!(
            "{}/../{}/",
            dir.display(),
            dir.file_name().unwrap().to_str().unwrap()
        );
        let url = parse_user_input(&with_dots);
        assert_eq!(url.scheme(), "file");
        assert!(url.as_str().ends_with('/'));
        assert_eq!(url.to_file_path().unwrap(), dir);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn root_and_home() {
        assert_eq!(parse("/"), "file:///");
        if let Some(home) = home_dir().filter(|h| h.is_dir()) {
            let url = parse_user_input("~");
            assert_eq!(url.scheme(), "file");
            assert_eq!(url.to_file_path().unwrap(), normalize_lexically(&home));
        }
    }

    #[test]
    fn lexical_normalization() {
        assert_eq!(
            normalize_lexically(Path::new("/a/./b/../c")),
            PathBuf::from("/a/c")
        );
        assert_eq!(normalize_lexically(Path::new("/../..")), PathBuf::from("/"));
    }

    #[test]
    fn helper_predicates() {
        assert!(is_ipv4("0.0.0.0"));
        assert!(!is_ipv4("1.2.3"));
        assert!(!is_ipv4("1.2.3.4.5"));
        assert!(!is_ipv4("01234.1.1.1"));
        assert!(is_bracketed_ipv6("[fe80::1]"));
        assert!(!is_bracketed_ipv6("fe80::1"));
        assert!(looks_like_domain("a.io"));
        assert!(!looks_like_domain("a.1"));
        assert!(!looks_like_domain("com"));
        assert!(!looks_like_domain(".com"));
        assert_eq!(split_host_port("[::1]x"), None);
        assert_eq!(split_host_port("h:1"), Some(("h", Some("1"))));
    }

    /// The query a search URL carries, decoded.
    fn search_query(url: &Url) -> Option<String> {
        if !url.as_str().starts_with(SEARCH_URL) {
            return None;
        }
        url.query_pairs()
            .find(|(k, _)| k == "q")
            .map(|(_, v)| v.into_owned())
    }

    #[test]
    fn search_queries_round_trip() {
        for input in [
            "rust programming",
            "a+b=c&d",
            "100%",
            "#hashtag",
            "C:\\x",
            "日本語 テキスト",
        ] {
            let url = parse_user_input(input);
            assert_eq!(search_query(&url).as_deref(), Some(input), "{input:?}");
        }
    }

    #[test]
    fn fuzz_inputs_give_a_loadable_url() {
        let alphabet = [
            "a",
            "b",
            "1",
            "9",
            ".",
            ":",
            "/",
            "?",
            "#",
            "@",
            "[",
            "]",
            " ",
            "-",
            "_",
            "~",
            "%",
            "com",
            "localhost",
            "http",
            "https://",
            "about:",
            "file:",
            "data:",
            "é",
            "日",
            "\\",
            "\t",
            "\u{0}",
            "\u{FEFF}",
            "8080",
            "::1",
            "xn--",
            "+",
        ];
        let mut rng = Rng::new(2024);
        for _ in 0..20_000 {
            let input = rng.string_from(&alphabet, 10);
            let url = parse_user_input(&input);
            let trimmed = input.trim();
            if trimmed.is_empty() {
                assert_eq!(url.as_str(), "about:home");
                continue;
            }
            assert!(KNOWN_SCHEMES.contains(&url.scheme()), "{input:?} -> {url}");
            if let Some(query) = search_query(&url) {
                // A search keeps the whole query, unless the input itself was
                // a URL pointing at the search engine.
                if !trimmed.contains("duckduckgo") {
                    assert_eq!(query, trimmed, "{input:?} -> {url}");
                }
            }
            if matches!(url.scheme(), "http" | "https") {
                assert!(
                    url.host_str().is_some_and(|h| !h.is_empty()),
                    "{input:?} -> {url}"
                );
            }
        }
    }

    #[test]
    fn hostile_inputs_do_not_panic() {
        for input in [
            "[", "]", "[:", "::::", "%", "%zz", "?", "#", "/?#", "a:b:c", "\u{0}", "\u{FEFF}",
            "....", ".", "..", "~", "~user", "http://[", "data:", "about:", ":8080",
        ] {
            let _ = parse_user_input(input);
        }
        let long = "a".repeat(100_000) + ".com";
        let _ = parse_user_input(&long);
    }
}
