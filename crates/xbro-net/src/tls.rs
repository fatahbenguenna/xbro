//! Root certificates from the environment (`SSL_CERT_FILE`).
//!
//! ureq trusts Mozilla's root store by default. Behind a TLS-intercepting
//! proxy (corporate networks, sandboxes) the proxy's CA is only known through
//! a PEM bundle named by `SSL_CERT_FILE` (the OpenSSL convention, also used by
//! curl and Python); when that variable points to a readable bundle, its
//! certificates replace the built-in roots, as they do for OpenSSL.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use ureq::tls::{parse_pem, PemItem, RootCerts};

/// Bundles larger than this are ignored (a system bundle is ~200 KiB).
const MAX_BUNDLE_SIZE: u64 = 16 * 1024 * 1024;

/// Upper bound on the number of certificates taken from one bundle.
const MAX_CERTIFICATES: usize = 10_000;

/// Roots from the bundle named by `SSL_CERT_FILE`, if set and usable.
pub(crate) fn root_certs_from_env() -> Option<RootCerts> {
    let path = std::env::var_os("SSL_CERT_FILE").filter(|p| !p.is_empty())?;
    root_certs_from_file(Path::new(&path))
}

/// Roots from a PEM file; `None` when unreadable, not a regular file (a FIFO
/// would block forever), too large or without certificates.
pub(crate) fn root_certs_from_file(path: &Path) -> Option<RootCerts> {
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = File::open(path).ok()?;
    let mut pem = Vec::new();
    file.take(MAX_BUNDLE_SIZE + 1).read_to_end(&mut pem).ok()?;
    if pem.len() as u64 > MAX_BUNDLE_SIZE {
        return None;
    }
    root_certs_from_pem(&pem)
}

/// Every `CERTIFICATE` section of a PEM bundle (parsing stops at the first
/// malformed section); `None` when there is none.
pub(crate) fn root_certs_from_pem(pem: &[u8]) -> Option<RootCerts> {
    let certs: Vec<_> = parse_pem(pem)
        .map_while(Result::ok)
        .filter_map(|item| match item {
            PemItem::Certificate(cert) => Some(cert),
            _ => None,
        })
        .take(MAX_CERTIFICATES)
        .collect();
    if certs.is_empty() {
        None
    } else {
        Some(RootCerts::from(certs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEM: &str = "\
-----BEGIN CERTIFICATE-----
MAMCAQE=
-----END CERTIFICATE-----
some comment between sections
-----BEGIN CERTIFICATE-----
MAMCAQI=
-----END CERTIFICATE-----
";

    fn count(roots: Option<RootCerts>) -> usize {
        match roots {
            Some(RootCerts::Specific(certs)) => certs.len(),
            Some(_) => usize::MAX,
            None => 0,
        }
    }

    #[test]
    fn parses_certificates() {
        assert_eq!(count(root_certs_from_pem(PEM.as_bytes())), 2);
    }

    #[test]
    fn no_certificates_means_none() {
        assert_eq!(count(root_certs_from_pem(b"")), 0);
        assert_eq!(count(root_certs_from_pem(b"not a pem file at all")), 0);
        assert_eq!(
            count(root_certs_from_pem(
                b"-----BEGIN CERTIFICATE-----\n!!!!\n-----END CERTIFICATE-----\n"
            )),
            0
        );
        assert_eq!(
            count(root_certs_from_pem(
                b"-----BEGIN CERTIFICATE-----\nMAMCAQE=\n(truncated"
            )),
            0
        );
    }

    #[test]
    fn stops_at_the_first_malformed_section() {
        let pem = format!("{PEM}-----BEGIN CERTIFICATE-----\n!!\n-----END CERTIFICATE-----\n{PEM}");
        assert_eq!(count(root_certs_from_pem(pem.as_bytes())), 2);
    }

    #[test]
    fn special_files_are_ignored() {
        assert_eq!(count(root_certs_from_file(&std::env::temp_dir())), 0);
        if Path::new("/dev/null").exists() {
            assert_eq!(count(root_certs_from_file(Path::new("/dev/null"))), 0);
        }
    }

    #[test]
    fn files() {
        assert_eq!(
            count(root_certs_from_file(Path::new("/definitely/missing.pem"))),
            0
        );
        let path = std::env::temp_dir().join(format!("xbro-net-roots-{}.pem", std::process::id()));
        std::fs::write(&path, PEM).unwrap();
        assert_eq!(count(root_certs_from_file(&path)), 2);
        std::fs::remove_file(&path).unwrap();
    }
}
