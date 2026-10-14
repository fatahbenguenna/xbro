//! Error type shared by every loader.

/// Why a resource could not be loaded.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages() {
        assert_eq!(
            NetError::UnsupportedScheme("ftp".into()).to_string(),
            "unsupported URL scheme: ftp"
        );
        assert_eq!(
            NetError::Io("connection refused by 127.0.0.1:1".into()).to_string(),
            "connection refused by 127.0.0.1:1"
        );
        assert_eq!(NetError::TooManyRedirects.to_string(), "too many redirects");
        assert_eq!(NetError::TooLarge.to_string(), "response too large");
        assert_eq!(
            NetError::Invalid("bad".into()).to_string(),
            "invalid resource: bad"
        );
        assert_eq!(NetError::NotFound.to_string(), "not found");
    }
}
