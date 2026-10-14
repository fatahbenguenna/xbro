//! Proxy selection from the environment (`HTTPS_PROXY`, `HTTP_PROXY`,
//! `ALL_PROXY`, `NO_PROXY`).
//!
//! ureq can read these variables itself, but it uses a single proxy for every
//! scheme and its `NO_PROXY` matching ignores CIDR ranges (`10.0.0.0/8`),
//! ports and surrounding spaces. xbro therefore resolves the proxy for each
//! request (and each redirect hop) here and hands ureq an explicit choice.

use std::net::IpAddr;

use ureq::{Proxy, ProxyProtocol};
use url::{Host, Url};

/// The proxies to use for `http:` and `https:` URLs, and the hosts that bypass them.
#[derive(Clone, Debug, Default)]
pub(crate) struct ProxySettings {
    http: Option<Proxy>,
    https: Option<Proxy>,
    no_proxy: NoProxy,
}

impl ProxySettings {
    /// No proxy at all (direct connections).
    pub(crate) fn none() -> Self {
        Self::default()
    }

    /// Reads the standard proxy environment variables.
    pub(crate) fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Builds the settings from a variable lookup function (testable without
    /// touching the process environment).
    ///
    /// `https:` URLs use `https_proxy`, then `all_proxy`, then `http_proxy`;
    /// `http:` URLs use `http_proxy`, then `all_proxy`, then `https_proxy`
    /// (lowercase names are preferred over uppercase ones, as curl does).
    /// Only HTTP(S) CONNECT proxies are supported; others are ignored.
    pub(crate) fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let var = |names: [&str; 2]| {
            names.iter().find_map(|name| {
                lookup(name)
                    .map(|v| v.trim().to_owned())
                    .filter(|v| !v.is_empty())
            })
        };
        let https_var = var(["https_proxy", "HTTPS_PROXY"]);
        let http_var = var(["http_proxy", "HTTP_PROXY"]);
        let all_var = var(["all_proxy", "ALL_PROXY"]);
        let first_usable = |candidates: [&Option<String>; 3]| {
            candidates
                .into_iter()
                .flatten()
                .find_map(|raw| make_proxy(raw))
        };
        ProxySettings {
            https: first_usable([&https_var, &all_var, &http_var]),
            http: first_usable([&http_var, &all_var, &https_var]),
            no_proxy: var(["no_proxy", "NO_PROXY"])
                .map(|v| NoProxy::parse(&v))
                .unwrap_or_default(),
        }
    }

    /// The proxy to use for `url`, or `None` to connect directly.
    ///
    /// Loopback hosts (`localhost`, `*.localhost`, `127.0.0.0/8`, `::1`) are
    /// always reached directly, as Chrome and Firefox do: a proxy cannot reach
    /// this machine's local servers on the user's behalf.
    pub(crate) fn proxy_for(&self, url: &Url) -> Option<Proxy> {
        let proxy = match url.scheme() {
            "https" => self.https.as_ref(),
            "http" => self.http.as_ref(),
            _ => None,
        }?;
        if is_loopback(url) || self.no_proxy.matches(url) {
            None
        } else {
            Some(proxy.clone())
        }
    }
}

/// Whether `url` points at this machine's loopback interface.
fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            domain == "localhost" || domain.ends_with(".localhost")
        }
        Some(Host::Ipv4(v4)) => v4.is_loopback(),
        Some(Host::Ipv6(v6)) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
        None => false,
    }
}

/// Parses a proxy URL, keeping only protocols this build of ureq can use
/// (a manually configured SOCKS proxy would make ureq panic without its
/// `socks-proxy` feature).
fn make_proxy(raw: &str) -> Option<Proxy> {
    let proxy = Proxy::new(raw).ok()?;
    matches!(proxy.protocol(), ProxyProtocol::Http | ProxyProtocol::Https).then_some(proxy)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NoProxyEntry {
    /// A domain and its subdomains (`example.com`, `.example.com`, `*.example.com`).
    Domain { name: String, port: Option<u16> },
    /// An exact IP address.
    Ip { addr: IpAddr, port: Option<u16> },
    /// An address range (`10.0.0.0/8`, `fd00::/8`).
    Cidr { network: IpAddr, prefix: u8 },
}

/// The parsed `NO_PROXY` list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct NoProxy {
    all: bool,
    entries: Vec<NoProxyEntry>,
}

/// Splits `host:port` when the suffix is a valid port number.
fn split_port(entry: &str) -> (&str, Option<u16>) {
    if let Some((host, port)) = entry.rsplit_once(':') {
        if !host.contains(':') || host.ends_with(']') {
            if let Ok(port) = port.parse::<u16>() {
                return (host, Some(port));
            }
        }
    }
    (entry, None)
}

fn parse_ip(s: &str) -> Option<IpAddr> {
    let s = s
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(s);
    s.parse().ok()
}

impl NoProxy {
    /// Parses a comma- (or space-) separated list.
    pub(crate) fn parse(list: &str) -> Self {
        let mut no_proxy = NoProxy::default();
        for token in list
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
        {
            if token == "*" {
                no_proxy.all = true;
                continue;
            }
            if let Some((addr, prefix)) = token.split_once('/') {
                if let (Some(network), Ok(prefix)) = (parse_ip(addr), prefix.parse::<u8>()) {
                    let max = if network.is_ipv4() { 32 } else { 128 };
                    if prefix <= max {
                        no_proxy
                            .entries
                            .push(NoProxyEntry::Cidr { network, prefix });
                    }
                }
                continue;
            }
            if let Some(addr) = parse_ip(token) {
                no_proxy.entries.push(NoProxyEntry::Ip { addr, port: None });
                continue;
            }
            let (host, port) = split_port(token);
            if let Some(addr) = parse_ip(host) {
                no_proxy.entries.push(NoProxyEntry::Ip { addr, port });
                continue;
            }
            let name = host
                .trim_start_matches('*')
                .trim_start_matches('.')
                .trim_end_matches('.')
                .to_ascii_lowercase();
            if !name.is_empty() {
                no_proxy.entries.push(NoProxyEntry::Domain { name, port });
            }
        }
        no_proxy
    }

    /// Whether `url` must bypass the proxy.
    pub(crate) fn matches(&self, url: &Url) -> bool {
        if self.all {
            return true;
        }
        let port = url.port_or_known_default();
        let port_ok = |wanted: &Option<u16>| wanted.is_none() || *wanted == port;
        match url.host() {
            Some(Host::Domain(domain)) => {
                let domain = domain.trim_end_matches('.').to_ascii_lowercase();
                self.entries.iter().any(|entry| match entry {
                    NoProxyEntry::Domain { name, port } => {
                        port_ok(port)
                            && (domain == *name
                                || domain
                                    .strip_suffix(name.as_str())
                                    .is_some_and(|prefix| prefix.ends_with('.')))
                    }
                    _ => false,
                })
            }
            Some(Host::Ipv4(v4)) => self.matches_ip(IpAddr::V4(v4), port),
            Some(Host::Ipv6(v6)) => self.matches_ip(IpAddr::V6(v6), port),
            None => false,
        }
    }

    fn matches_ip(&self, ip: IpAddr, url_port: Option<u16>) -> bool {
        self.entries.iter().any(|entry| match entry {
            NoProxyEntry::Ip { addr, port } => *addr == ip && (port.is_none() || *port == url_port),
            NoProxyEntry::Cidr { network, prefix } => in_network(ip, *network, *prefix),
            NoProxyEntry::Domain { .. } => false,
        })
    }
}

/// Whether `ip` belongs to `network/prefix` (same address family only).
fn in_network(ip: IpAddr, network: IpAddr, prefix: u8) -> bool {
    match (ip, network) {
        (IpAddr::V4(ip), IpAddr::V4(net)) => {
            let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            u32::from(ip) & mask == u32::from(net) & mask
        }
        (IpAddr::V6(ip), IpAddr::V6(net)) => {
            let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            u128::from(ip) & mask == u128::from(net) & mask
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn settings(vars: &[(&str, &str)]) -> ProxySettings {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        ProxySettings::from_lookup(|name| map.get(name).cloned())
    }

    fn proxy_host(s: &ProxySettings, url: &str) -> Option<String> {
        s.proxy_for(&Url::parse(url).unwrap())
            .map(|p| format!("{}:{}", p.host(), p.port()))
    }

    fn bypass(list: &str, url: &str) -> bool {
        NoProxy::parse(list).matches(&Url::parse(url).unwrap())
    }

    #[test]
    fn no_variables_means_direct() {
        let s = settings(&[]);
        assert_eq!(proxy_host(&s, "https://example.com/"), None);
        assert_eq!(proxy_host(&s, "http://example.com/"), None);
        assert_eq!(proxy_host(&ProxySettings::none(), "https://a.b/"), None);
    }

    #[test]
    fn loopback_hosts_always_bypass_the_proxy() {
        let s = settings(&[("HTTPS_PROXY", "http://proxy.corp:3128")]);
        for url in [
            "http://localhost:8000/",
            "https://LOCALHOST./",
            "http://app.localhost/",
            "http://127.0.0.1:3000/",
            "http://127.5.6.7/",
            "http://[::1]:8080/",
            "http://[::ffff:127.0.0.1]/",
        ] {
            assert_eq!(proxy_host(&s, url), None, "{url}");
        }
        for url in [
            "http://localhost.example.com/",
            "http://128.0.0.1/",
            "http://[::2]/",
            "http://mylocalhost/",
        ] {
            assert!(proxy_host(&s, url).is_some(), "{url}");
        }
    }

    #[test]
    fn https_proxy_is_used_for_both_schemes_as_fallback() {
        let s = settings(&[("HTTPS_PROXY", "http://127.0.0.1:45677")]);
        assert_eq!(
            proxy_host(&s, "https://example.com/").as_deref(),
            Some("127.0.0.1:45677")
        );
        assert_eq!(
            proxy_host(&s, "http://example.com/").as_deref(),
            Some("127.0.0.1:45677")
        );
        assert_eq!(proxy_host(&s, "file:///tmp").as_deref(), None);
    }

    #[test]
    fn scheme_specific_proxies() {
        let s = settings(&[
            ("https_proxy", "http://secure:3128"),
            ("http_proxy", "plain:8080"),
            ("ALL_PROXY", "http://all:1"),
        ]);
        assert_eq!(
            proxy_host(&s, "https://example.com/").as_deref(),
            Some("secure:3128")
        );
        assert_eq!(
            proxy_host(&s, "http://example.com/").as_deref(),
            Some("plain:8080")
        );
        let s = settings(&[("all_proxy", "http://all:1")]);
        assert_eq!(proxy_host(&s, "https://x.org/").as_deref(), Some("all:1"));
        assert_eq!(proxy_host(&s, "http://x.org/").as_deref(), Some("all:1"));
    }

    #[test]
    fn lowercase_wins_and_blank_values_are_ignored() {
        let s = settings(&[
            ("https_proxy", "http://lower:1"),
            ("HTTPS_PROXY", "http://upper:2"),
        ]);
        assert_eq!(proxy_host(&s, "https://x.org/").as_deref(), Some("lower:1"));
        let s = settings(&[("https_proxy", "  "), ("HTTPS_PROXY", "http://upper:2")]);
        assert_eq!(proxy_host(&s, "https://x.org/").as_deref(), Some("upper:2"));
    }

    #[test]
    fn unsupported_or_invalid_proxies_are_skipped() {
        let s = settings(&[
            ("https_proxy", "socks5://127.0.0.1:1080"),
            ("all_proxy", "http://fallback:3128"),
        ]);
        assert_eq!(
            proxy_host(&s, "https://x.org/").as_deref(),
            Some("fallback:3128")
        );
        let s = settings(&[("https_proxy", "ftp://nope:21")]);
        assert_eq!(proxy_host(&s, "https://x.org/"), None);
    }

    #[test]
    fn no_proxy_is_honoured() {
        let s = settings(&[
            ("HTTPS_PROXY", "http://127.0.0.1:45677"),
            (
                "NO_PROXY",
                "localhost,127.0.0.1,::1,127.0.0.0/8,10.0.0.0/8, .svc.cluster.local,*.internal.com",
            ),
        ]);
        assert_eq!(proxy_host(&s, "http://localhost:8080/"), None);
        assert_eq!(proxy_host(&s, "http://127.0.0.1:3000/x"), None);
        assert_eq!(proxy_host(&s, "http://127.4.5.6/"), None);
        assert_eq!(proxy_host(&s, "http://[::1]:8000/"), None);
        assert_eq!(proxy_host(&s, "https://10.1.2.3/"), None);
        assert_eq!(proxy_host(&s, "https://api.svc.cluster.local/"), None);
        assert_eq!(proxy_host(&s, "https://a.b.internal.com/"), None);
        assert!(proxy_host(&s, "https://example.com/").is_some());
        assert!(proxy_host(&s, "https://11.0.0.1/").is_some());
    }

    #[test]
    fn no_proxy_domain_matching() {
        assert!(bypass("example.com", "https://example.com/"));
        assert!(bypass("example.com", "https://www.example.com/"));
        assert!(bypass("EXAMPLE.com", "https://WWW.Example.COM./"));
        assert!(bypass(".example.com", "https://example.com/"));
        assert!(bypass("*.example.com", "https://a.example.com/"));
        assert!(!bypass("example.com", "https://notexample.com/"));
        assert!(!bypass("example.com", "https://example.com.evil.org/"));
        assert!(!bypass("", "https://example.com/"));
        assert!(bypass("*", "https://anything.org/"));
        assert!(bypass("foo.org , example.com", "https://example.com/"));
    }

    #[test]
    fn no_proxy_ports() {
        assert!(bypass("example.com:8080", "http://example.com:8080/"));
        assert!(!bypass("example.com:8080", "http://example.com/"));
        assert!(bypass("example.com:443", "https://example.com/"));
        assert!(bypass("127.0.0.1:3000", "http://127.0.0.1:3000/"));
        assert!(!bypass("127.0.0.1:3000", "http://127.0.0.1:3001/"));
        assert!(bypass("[::1]:8000", "http://[::1]:8000/"));
    }

    #[test]
    fn no_proxy_prefix_extremes() {
        assert!(bypass("10.1.2.3/32", "http://10.1.2.3/"));
        assert!(!bypass("10.1.2.3/32", "http://10.1.2.4/"));
        assert!(bypass("::/0", "http://[2001:db8::1]/"));
        assert!(bypass("2001:db8::1/128", "http://[2001:db8::1]/"));
        assert!(!bypass("2001:db8::1/128", "http://[2001:db8::2]/"));
        assert!(bypass("[2001:db8::]/32", "http://[2001:db8:ffff::1]/"));
        assert!(!bypass("::/129", "http://[::1]/"));
        assert!(!bypass("10.0.0.0/-1", "http://10.0.0.1/"));
        assert!(!bypass("10.0.0.0/", "http://10.0.0.1/"));
    }

    #[test]
    fn fuzz_no_proxy_lists_never_panic() {
        let alphabet = [
            "a",
            ".",
            "*",
            ",",
            " ",
            ":",
            "/",
            "[",
            "]",
            "1",
            "0",
            "255",
            "8",
            "::",
            "f",
            "example.com",
            "\t",
            "é",
            "-",
            "65536",
            "99999999999",
        ];
        let urls = [
            "http://example.com/",
            "https://a.b.example.com:8443/",
            "http://10.0.0.1/",
            "http://[::1]:80/",
            "http://[2001:db8::1]/",
            "http://localhost/",
        ]
        .map(|u| Url::parse(u).unwrap());
        let mut rng = crate::test_util::Rng::new(5);
        for _ in 0..10_000 {
            let list = rng.string_from(&alphabet, 12);
            let no_proxy = NoProxy::parse(&list);
            for url in &urls {
                let _ = no_proxy.matches(url);
            }
            let settings = ProxySettings::from_lookup(|name| {
                (name == "HTTPS_PROXY" || name == "no_proxy").then(|| list.clone())
            });
            for url in &urls {
                let _ = settings.proxy_for(url);
            }
        }
    }

    #[test]
    fn no_proxy_cidr() {
        assert!(bypass("192.168.0.0/16", "http://192.168.44.1/"));
        assert!(!bypass("192.168.0.0/16", "http://192.169.0.1/"));
        assert!(bypass("0.0.0.0/0", "http://8.8.8.8/"));
        assert!(bypass("fd00::/8", "http://[fd12::1]/"));
        assert!(!bypass("fd00::/8", "http://[fe80::1]/"));
        assert!(!bypass("10.0.0.0/8", "http://[::1]/"));
        // Invalid prefixes are ignored.
        assert!(!bypass("10.0.0.0/33", "http://10.0.0.1/"));
        assert!(!bypass("garbage/8", "http://10.0.0.1/"));
        // A CIDR never matches host names.
        assert!(!bypass("10.0.0.0/8", "http://example.com/"));
    }
}
