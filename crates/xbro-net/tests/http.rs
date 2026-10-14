//! HTTP integration tests against a local server (no internet access).

mod common;

use std::io::Write;
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use common::{response, test_fetcher, TestServer};
use flate2::write::GzEncoder;
use flate2::Compression;
use xbro_net::{Fetcher, FetcherOptions, NetError, ACCEPT, USER_AGENT};

#[test]
fn ok_with_content_type_and_charset() {
    let server = TestServer::start(|_| {
        response(
            "200 OK",
            &[("Content-Type", "Text/HTML; Charset=\"ISO-8859-1\"")],
            b"<p>caf\xE9</p>",
        )
    });
    let r = test_fetcher().fetch(&server.url("/index.html")).unwrap();
    assert_eq!(r.status, 200);
    assert!(r.is_success());
    assert_eq!(r.url, server.url("/index.html"));
    assert_eq!(r.content_type.as_deref(), Some("text/html"));
    assert_eq!(r.charset.as_deref(), Some("iso-8859-1"));
    assert_eq!(r.body, b"<p>caf\xE9</p>");
    assert_eq!(r.text(), "<p>café</p>");
}

#[test]
fn request_headers() {
    let server = TestServer::start(|_| response("200 OK", &[], b""));
    let fetcher = test_fetcher();
    fetcher.fetch(&server.url("/h?x=1#frag")).unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert_eq!(req.method, "GET");
    // The fragment is never sent.
    assert_eq!(req.target, "/h?x=1");
    assert_eq!(req.header("user-agent"), Some(USER_AGENT));
    assert_eq!(req.header("accept"), Some(ACCEPT));
    assert_eq!(
        req.header("accept-language"),
        Some(fetcher.accept_language())
    );
    let encodings = req.header("accept-encoding").unwrap_or_default();
    assert!(encodings.contains("gzip"), "{encodings}");
    assert!(encodings.contains("br"), "{encodings}");
    assert_eq!(
        req.header("host"),
        Some(format!("127.0.0.1:{}", server.addr.port()).as_str())
    );
}

#[test]
fn not_found_is_returned_with_body() {
    let server = TestServer::start(|_| {
        response(
            "404 Not Found",
            &[("Content-Type", "text/html")],
            b"<h1>Nope</h1>",
        )
    });
    let r = test_fetcher().fetch(&server.url("/missing")).unwrap();
    assert_eq!(r.status, 404);
    assert!(!r.is_success());
    assert_eq!(r.body, b"<h1>Nope</h1>");
    assert_eq!(r.content_type.as_deref(), Some("text/html"));
}

#[test]
fn server_error_is_returned_with_body() {
    let server = TestServer::start(|_| response("503 Service Unavailable", &[], b"busy"));
    let r = test_fetcher().fetch(&server.url("/")).unwrap();
    assert_eq!(r.status, 503);
    assert_eq!(r.body, b"busy");
    assert_eq!(r.content_type.as_deref(), Some("text/plain"));
}

#[test]
fn redirect_chain_reports_final_url() {
    let server = TestServer::start(|req| match req.path() {
        "/start" => response("301 Moved Permanently", &[("Location", "/middle")], b""),
        // Relative to the current URL, with a query.
        "/middle" => response("302 Found", &[("Location", "dir/end?x=1")], b"moved"),
        "/dir/end" => response("200 OK", &[("Content-Type", "text/plain")], b"done"),
        _ => response("500 Internal Server Error", &[], b""),
    });
    let r = test_fetcher().fetch(&server.url("/start#section")).unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"done");
    // The fragment of the original URL is carried over.
    assert_eq!(r.url, server.url("/dir/end?x=1#section"));
    let paths: Vec<String> = server.requests().iter().map(|r| r.target.clone()).collect();
    assert_eq!(paths, ["/start", "/middle", "/dir/end?x=1"]);
}

#[test]
fn redirects_across_servers_and_status_codes() {
    let target = TestServer::start(|_| response("200 OK", &[], b"other server"));
    let target_url = target.url("/final").to_string();
    let server = TestServer::start(move |req| match req.path() {
        "/a" => response("303 See Other", &[("Location", "/b")], b""),
        "/b" => response("307 Temporary Redirect", &[("Location", "/c")], b""),
        "/c" => response("308 Permanent Redirect", &[("Location", &target_url)], b""),
        _ => response("404 Not Found", &[], b""),
    });
    let r = test_fetcher().fetch(&server.url("/a")).unwrap();
    assert_eq!(r.body, b"other server");
    assert_eq!(r.url, target.url("/final"));
}

#[test]
fn redirect_without_location_is_returned() {
    let server = TestServer::start(|_| response("302 Found", &[], b"no location"));
    let r = test_fetcher().fetch(&server.url("/")).unwrap();
    assert_eq!(r.status, 302);
    assert_eq!(r.body, b"no location");
}

#[test]
fn redirect_loop_is_an_error() {
    let server = TestServer::start(|_| response("302 Found", &[("Location", "/loop")], b""));
    let err = test_fetcher().fetch(&server.url("/loop")).unwrap_err();
    assert_eq!(err, NetError::TooManyRedirects);
    // The initial request plus ten redirects.
    assert_eq!(server.requests().len(), 11);
}

#[test]
fn ten_redirects_are_allowed() {
    let server = TestServer::start(|req| {
        let n: u32 = req.path().trim_start_matches("/r").parse().unwrap_or(0);
        if n < 10 {
            let next = format!("/r{}", n + 1);
            response("302 Found", &[("Location", &next)], b"")
        } else {
            response("200 OK", &[], b"made it")
        }
    });
    let r = test_fetcher().fetch(&server.url("/r0")).unwrap();
    assert_eq!(r.body, b"made it");
    assert_eq!(r.url, server.url("/r10"));
}

#[test]
fn redirect_to_non_http_scheme_is_refused() {
    let server =
        TestServer::start(|_| response("302 Found", &[("Location", "file:///etc/passwd")], b""));
    let err = test_fetcher().fetch(&server.url("/")).unwrap_err();
    assert!(matches!(err, NetError::Invalid(_)), "{err:?}");
}

#[test]
fn cookies_are_stored_and_sent_back() {
    let server = TestServer::start(|req| match req.path() {
        "/login" => response(
            "200 OK",
            &[
                ("Set-Cookie", "session=abc123; Path=/"),
                ("Set-Cookie", "theme=dark; Path=/"),
            ],
            b"welcome",
        ),
        // A cookie set on a redirect response must be kept too.
        "/redirect" => response(
            "302 Found",
            &[("Set-Cookie", "hop=1; Path=/"), ("Location", "/check")],
            b"",
        ),
        _ => response("200 OK", &[], b"ok"),
    });
    let fetcher = test_fetcher();
    fetcher.fetch(&server.url("/login")).unwrap();
    fetcher.fetch(&server.url("/redirect")).unwrap();
    let requests = server.requests();
    let check = requests.iter().find(|r| r.path() == "/check").unwrap();
    let cookie = check.header("cookie").unwrap_or_default();
    assert!(cookie.contains("session=abc123"), "{cookie}");
    assert!(cookie.contains("theme=dark"), "{cookie}");
    assert!(cookie.contains("hop=1"), "{cookie}");
    // The first request had no cookie, and another fetcher has its own jar.
    assert_eq!(requests[0].header("cookie"), None);
    test_fetcher().fetch(&server.url("/other")).unwrap();
    let other = server.requests().into_iter().last().unwrap();
    assert_eq!(other.header("cookie"), None);
}

#[test]
fn gzip_body_is_decoded() {
    let original = "<!DOCTYPE html><p>".to_owned() + &"compressed ".repeat(1000) + "</p>";
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(original.as_bytes()).unwrap();
    let gzipped = encoder.finish().unwrap();
    assert!(gzipped.len() < original.len());
    let server = TestServer::start(move |_| {
        response(
            "200 OK",
            &[
                ("Content-Type", "text/html; charset=utf-8"),
                ("Content-Encoding", "gzip"),
            ],
            &gzipped,
        )
    });
    let r = test_fetcher().fetch(&server.url("/gz")).unwrap();
    assert_eq!(r.body, original.as_bytes());
    assert_eq!(r.content_type.as_deref(), Some("text/html"));
}

#[test]
fn chunked_body_is_decoded() {
    let server = TestServer::start(|_| {
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nHello\r\n7\r\n, World\r\n0\r\n\r\n"
            .to_vec()
    });
    let r = test_fetcher().fetch(&server.url("/")).unwrap();
    assert_eq!(r.body, b"Hello, World");
}

#[test]
fn body_without_length_reads_until_close() {
    let server = TestServer::start(|_| {
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nuntil the end"
            .to_vec()
    });
    let r = test_fetcher().fetch(&server.url("/")).unwrap();
    assert_eq!(r.body, b"until the end");
}

#[test]
fn no_content() {
    let server =
        TestServer::start(|_| b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_vec());
    let r = test_fetcher().fetch(&server.url("/")).unwrap();
    assert_eq!(r.status, 204);
    assert!(r.body.is_empty());
}

#[test]
fn missing_content_type_is_sniffed() {
    let server = TestServer::start(|req| match req.path() {
        "/html" => response("200 OK", &[], b"\n  <!doctype html><title>x</title>"),
        "/png" => response("200 OK", &[], b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"),
        "/bin" => response("200 OK", &[], b"\0\x01\x02binary"),
        "/unknown" => response(
            "200 OK",
            &[("Content-Type", "unknown/unknown; charset=utf-8")],
            b"<html>",
        ),
        "/garbage" => response("200 OK", &[("Content-Type", "garbage")], b"plain words"),
        _ => response("200 OK", &[], b"just text"),
    });
    let f = test_fetcher();
    let ty = |path: &str| f.fetch(&server.url(path)).unwrap().content_type;
    assert_eq!(ty("/html").as_deref(), Some("text/html"));
    assert_eq!(ty("/png").as_deref(), Some("image/png"));
    assert_eq!(ty("/bin").as_deref(), Some("application/octet-stream"));
    assert_eq!(ty("/unknown").as_deref(), Some("text/html"));
    assert_eq!(ty("/garbage").as_deref(), Some("text/plain"));
    assert_eq!(ty("/text").as_deref(), Some("text/plain"));
    let unknown = f.fetch(&server.url("/unknown")).unwrap();
    assert_eq!(unknown.charset.as_deref(), Some("utf-8"));
}

#[test]
fn post_form_sends_urlencoded_body() {
    let server = TestServer::start(|req| {
        let body = format!("{} {}", req.method, String::from_utf8_lossy(&req.body));
        response("200 OK", &[("Content-Type", "text/plain")], body.as_bytes())
    });
    let fields = vec![
        ("name".to_owned(), "Zoë Smith".to_owned()),
        ("q".to_owned(), "a&b=c".to_owned()),
    ];
    let r = test_fetcher()
        .post_form(&server.url("/submit"), &fields)
        .unwrap();
    assert_eq!(r.body, b"POST name=Zo%C3%AB+Smith&q=a%26b%3Dc");
    let req = &server.requests()[0];
    assert_eq!(req.method, "POST");
    assert_eq!(
        req.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(req.header("content-length"), Some("31"));
    assert_eq!(req.header("user-agent"), Some(USER_AGENT));
}

#[test]
fn post_then_303_becomes_get() {
    let server = TestServer::start(|req| match req.path() {
        "/submit" => response("303 See Other", &[("Location", "/thanks")], b""),
        _ => {
            let body = format!("{} [{}]", req.method, String::from_utf8_lossy(&req.body));
            response("200 OK", &[], body.as_bytes())
        }
    });
    let r = test_fetcher()
        .post_form(&server.url("/submit"), &[("a".into(), "1".into())])
        .unwrap();
    assert_eq!(r.body, b"GET []");
    assert_eq!(r.url, server.url("/thanks"));
}

#[test]
fn post_then_307_keeps_method_and_body() {
    let server = TestServer::start(|req| match req.path() {
        "/old" => response("307 Temporary Redirect", &[("Location", "/new")], b""),
        _ => {
            let body = format!("{} [{}]", req.method, String::from_utf8_lossy(&req.body));
            response("200 OK", &[], body.as_bytes())
        }
    });
    let r = test_fetcher()
        .post_form(&server.url("/old"), &[("a".into(), "1".into())])
        .unwrap();
    assert_eq!(r.body, b"POST [a=1]");
}

#[test]
fn connection_refused_has_a_readable_message() {
    // Reserve a port, then close the listener so nothing accepts on it.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let url = xbro_net::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
    match test_fetcher().fetch(&url) {
        Err(NetError::Io(msg)) => {
            assert_eq!(msg, format!("connection refused by 127.0.0.1:{port}"));
        }
        other => panic!("expected a connection error, got {other:?}"),
    }
}

#[test]
fn body_limit_is_enforced() {
    let server = TestServer::start(|req| {
        let size = if req.path() == "/big" { 2048 } else { 1024 };
        response("200 OK", &[], &vec![b'x'; size])
    });
    let fetcher = Fetcher::with_options(FetcherOptions {
        use_env_proxy: false,
        max_body: 1024,
        ..FetcherOptions::default()
    });
    assert_eq!(
        fetcher.fetch(&server.url("/exact")).unwrap().body.len(),
        1024
    );
    assert_eq!(fetcher.fetch(&server.url("/big")), Err(NetError::TooLarge));
}

#[test]
fn slow_server_times_out() {
    let server = TestServer::start(|_| {
        thread::sleep(Duration::from_secs(3));
        response("200 OK", &[], b"too late")
    });
    let fetcher = Fetcher::with_options(FetcherOptions {
        use_env_proxy: false,
        timeout: Duration::from_millis(300),
        ..FetcherOptions::default()
    });
    let start = Instant::now();
    let err = fetcher.fetch(&server.url("/slow")).unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(2));
    match err {
        NetError::Io(msg) => assert!(msg.contains("timed out"), "{msg}"),
        other => panic!("expected a timeout, got {other:?}"),
    }
}

#[test]
fn malformed_response_is_an_error() {
    let server = TestServer::start(|_| b"this is not HTTP at all\r\n\r\n".to_vec());
    let err = test_fetcher().fetch(&server.url("/")).unwrap_err();
    assert!(matches!(err, NetError::Io(_)), "{err:?}");
}

#[test]
fn fetcher_is_shared_between_threads() {
    let server = Arc::new(TestServer::start(|req| {
        response("200 OK", &[], req.target.as_bytes())
    }));
    let fetcher = Arc::new(test_fetcher());
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let fetcher = Arc::clone(&fetcher);
            let server = Arc::clone(&server);
            thread::spawn(move || {
                let path = format!("/t{i}");
                let r = fetcher.fetch(&server.url(&path)).unwrap();
                assert_eq!(r.body, path.as_bytes());
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(server.requests().len(), 8);
}
