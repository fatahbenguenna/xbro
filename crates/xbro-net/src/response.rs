//! The [`Response`] type returned by every loader.

use url::Url;

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
    /// The (decompressed) body bytes.
    pub body: Vec<u8>,
}

impl Response {
    /// Builds a successful (status 200) response.
    pub(crate) fn ok(
        url: Url,
        content_type: impl Into<String>,
        charset: Option<String>,
        body: Vec<u8>,
    ) -> Self {
        Response {
            url,
            status: 200,
            content_type: Some(content_type.into()),
            charset,
            body,
        }
    }

    /// Whether the resource should be rendered as an HTML document
    /// (`text/html`, `application/xhtml+xml`, or no known type).
    pub fn is_html(&self) -> bool {
        matches!(
            self.content_type.as_deref(),
            None | Some("text/html") | Some("application/xhtml+xml")
        )
    }

    /// Whether the status is in the 2xx range.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body decoded as text with [`crate::decode_text`] and this
    /// response's charset.
    pub fn text(&self) -> String {
        crate::decode_text(&self.body, self.charset.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(content_type: Option<&str>, status: u16) -> Response {
        Response {
            url: Url::parse("https://example.com/").unwrap(),
            status,
            content_type: content_type.map(str::to_owned),
            charset: None,
            body: Vec::new(),
        }
    }

    #[test]
    fn is_html() {
        assert!(response(None, 200).is_html());
        assert!(response(Some("text/html"), 200).is_html());
        assert!(response(Some("application/xhtml+xml"), 200).is_html());
        assert!(!response(Some("text/plain"), 200).is_html());
        assert!(!response(Some("image/png"), 200).is_html());
    }

    #[test]
    fn is_success() {
        assert!(response(None, 200).is_success());
        assert!(response(None, 204).is_success());
        assert!(!response(None, 199).is_success());
        assert!(!response(None, 301).is_success());
        assert!(!response(None, 404).is_success());
    }

    #[test]
    fn text_uses_charset() {
        let mut r = response(Some("text/plain"), 200);
        r.body = vec![0x63, 0x61, 0x66, 0xE9];
        r.charset = Some("iso-8859-1".into());
        assert_eq!(r.text(), "café");
        let ok = Response::ok(r.url.clone(), "text/css", None, b"a{}".to_vec());
        assert_eq!(ok.status, 200);
        assert_eq!(ok.content_type.as_deref(), Some("text/css"));
        assert_eq!(ok.text(), "a{}");
    }
}
