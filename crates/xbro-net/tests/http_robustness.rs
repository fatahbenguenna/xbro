//! Adversarial HTTP tests: compression bombs, bodiless statuses, odd headers,
//! slow servers and hostile responses (local server only, no internet).

mod common;

use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use common::{read_request, response, test_fetcher, TestServer};
use flate2::write::GzEncoder;
use flate2::Compression;
use xbro_net::{Fetcher, FetcherOptions, NetError, Url};

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn fetcher_with(max_body: usize, timeout: Duration) -> Fetcher {
    Fetcher::with_options(FetcherOptions {
        use_env_proxy: false,
        max_body,
        timeout,
    })
}

/// A server whose connections are handled by `serve` (for streaming replies).
fn streaming_server(serve: fn(TcpStream)) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            thread::spawn(move || serve(stream));
        }
    });
    addr
}

#[test]
fn gzip_bomb_is_rejected_by_the_decompressed_size() {
    // 8 MiB of zeros compress to a few KiB: the limit must apply to the
    // decoded body, not to the bytes on the wire.
    let bomb = gzip(&vec![0u8; 8 * 1024 * 1024]);
    assert!(bomb.len() < 64 * 1024);
    let server = TestServer::start(move |_| {
        response(
            "200 OK",
            &[("Content-Encoding", "gzip"), ("Content-Type", "text/html")],
            &bomb,
        )
    });
    let fetcher = fetcher_with(1024 * 1024, Duration::from_secs(10));
    assert_eq!(fetcher.fetch(&server.url("/bomb")), Err(NetError::TooLarge));
}

#[test]
fn gzip_body_exactly_at_the_limit_is_accepted() {
    let data = vec![b'a'; 4096];
    let gz = gzip(&data);
    let server = TestServer::start(move |req| {
        let body = if req.path() == "/over" {
            gzip(&[b'a'; 4097])
        } else {
            gz.clone()
        };
        response("200 OK", &[("Content-Encoding", "gzip")], &body)
    });
    let fetcher = fetcher_with(4096, Duration::from_secs(10));
    assert_eq!(fetcher.fetch(&server.url("/exact")).unwrap().body, data);
    assert_eq!(fetcher.fetch(&server.url("/over")), Err(NetError::TooLarge));
}

#[test]
fn incompressible_gzip_body_at_the_limit_is_accepted() {
    // Gzip makes incompressible data slightly larger: the raw-byte bound
    // must not reject a body whose decoded size fits.
    let mut state = 0x1234_5678_u32;
    let data: Vec<u8> = (0..32 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state.to_le_bytes()[0]
        })
        .collect();
    let gz = gzip(&data);
    assert!(gz.len() > data.len());
    let server =
        TestServer::start(move |_| response("200 OK", &[("Content-Encoding", "gzip")], &gz));
    let fetcher = fetcher_with(data.len(), Duration::from_secs(10));
    assert_eq!(fetcher.fetch(&server.url("/")).unwrap().body, data);
}

#[test]
fn bodiless_statuses_ignore_content_encoding() {
    let server = TestServer::start(|req| {
        let status = match req.path() {
            "/204" => "204 No Content",
            "/205" => "205 Reset Content",
            _ => "304 Not Modified",
        };
        format!("HTTP/1.1 {status}\r\nContent-Encoding: gzip\r\nConnection: close\r\n\r\n")
            .into_bytes()
    });
    let fetcher = test_fetcher();
    for (path, status) in [("/204", 204), ("/205", 205), ("/304", 304)] {
        let r = fetcher.fetch(&server.url(path)).unwrap();
        assert_eq!(r.status, status);
        assert!(r.body.is_empty());
    }
}

#[test]
fn empty_gzip_body_is_an_empty_document() {
    let server = TestServer::start(|req| {
        if req.path() == "/close" {
            b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n"
                .to_vec()
        } else {
            response(
                "200 OK",
                &[("Content-Encoding", "gzip"), ("Content-Type", "text/html")],
                b"",
            )
        }
    });
    let fetcher = test_fetcher();
    for path in ["/length", "/close"] {
        let r = fetcher.fetch(&server.url(path)).unwrap();
        assert_eq!(r.status, 200, "{path}");
        assert!(r.body.is_empty(), "{path}");
        assert_eq!(r.content_type.as_deref(), Some("text/html"));
    }
}

#[test]
fn corrupt_compressed_bodies_are_readable_errors() {
    let server = TestServer::start(|req| {
        let encoding = if req.path() == "/br" { "br" } else { "gzip" };
        response(
            "200 OK",
            &[("Content-Encoding", encoding)],
            b"definitely not compressed",
        )
    });
    let fetcher = test_fetcher();
    for (path, name) in [("/gz", "gzip"), ("/br", "brotli")] {
        match fetcher.fetch(&server.url(path)) {
            Err(NetError::Io(msg)) => {
                assert!(msg.starts_with("could not decode the"), "{msg}");
                assert!(msg.contains(name), "{msg}");
                assert!(msg.contains("127.0.0.1"), "{msg}");
            }
            other => panic!("expected a decoding error for {path}, got {other:?}"),
        }
    }
}

#[test]
fn slow_body_hits_the_global_deadline() {
    // Headers arrive at once, then the body trickles in forever.
    let addr = streaming_server(|mut stream| {
        let _ = read_request(&mut stream);
        let _ = stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\nConnection: close\r\n\r\nstart",
        );
        for _ in 0..100 {
            thread::sleep(Duration::from_millis(50));
            if stream.write_all(b"x").is_err() {
                return;
            }
        }
    });
    let fetcher = fetcher_with(1024 * 1024, Duration::from_millis(400));
    let start = Instant::now();
    let err = fetcher
        .fetch(&Url::parse(&format!("http://{addr}/")).unwrap())
        .unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(3));
    match err {
        NetError::Io(msg) => assert!(msg.contains("timed out"), "{msg}"),
        other => panic!("expected a timeout, got {other:?}"),
    }
}

#[test]
fn zero_and_huge_timeouts_disable_the_deadline() {
    let server = TestServer::start(|_| {
        thread::sleep(Duration::from_millis(300));
        response("200 OK", &[], b"eventually")
    });
    for timeout in [Duration::ZERO, Duration::MAX, Duration::from_secs(u64::MAX)] {
        let fetcher = fetcher_with(1024, timeout);
        // Duration::MAX used to overflow `Instant + Duration` inside ureq.
        assert_eq!(fetcher.fetch(&server.url("/")).unwrap().body, b"eventually");
    }
}

#[test]
fn latin1_location_bytes_are_preserved() {
    let server = TestServer::start(|req| {
        if req.path() == "/start" {
            b"HTTP/1.1 302 Found\r\nLocation: /caf\xE9 au lait\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_vec()
        } else {
            response("200 OK", &[], req.target.as_bytes())
        }
    });
    let r = test_fetcher().fetch(&server.url("/start")).unwrap();
    // The raw byte 0xE9 is percent-encoded as is, not replaced by U+FFFD.
    assert_eq!(r.body, b"/caf%E9%20au%20lait");
    assert_eq!(r.url, server.url("/caf%E9%20au%20lait"));
}

#[test]
fn utf8_location_is_percent_encoded() {
    let server = TestServer::start(|req| {
        if req.path() == "/start" {
            response("301 Moved Permanently", &[("Location", "/café?q=ü")], b"")
        } else {
            response("200 OK", &[], req.target.as_bytes())
        }
    });
    let r = test_fetcher().fetch(&server.url("/start")).unwrap();
    assert_eq!(r.body, b"/caf%C3%A9?q=%C3%BC");
}

#[test]
fn post_then_301_or_302_becomes_get_and_308_keeps_the_body() {
    let server = TestServer::start(|req| match req.path() {
        "/301" => response("301 Moved Permanently", &[("Location", "/echo")], b""),
        "/302" => response("302 Found", &[("Location", "/echo")], b""),
        "/308" => response("308 Permanent Redirect", &[("Location", "/echo")], b""),
        _ => {
            let body = format!(
                "{} [{}] {}",
                req.method,
                String::from_utf8_lossy(&req.body),
                req.header("content-type").unwrap_or("-")
            );
            response("200 OK", &[], body.as_bytes())
        }
    });
    let fetcher = test_fetcher();
    let fields = [("k".to_owned(), "v w".to_owned())];
    for path in ["/301", "/302"] {
        let r = fetcher.post_form(&server.url(path), &fields).unwrap();
        assert_eq!(r.body, b"GET [] -", "{path}");
    }
    let r = fetcher.post_form(&server.url("/308"), &fields).unwrap();
    assert_eq!(r.body, b"POST [k=v+w] application/x-www-form-urlencoded");
}

#[test]
fn content_type_follows_the_fetch_extraction_rules() {
    let server = TestServer::start(|req| {
        match req.path() {
        // Two headers: the last valid one wins.
        "/two" => response(
            "200 OK",
            &[("Content-Type", "text/plain"), ("Content-Type", "text/html")],
            b"x",
        ),
        // A comma-separated list, with `*/*` ignored and the charset kept.
        "/list" => response(
            "200 OK",
            &[("Content-Type", "text/html;charset=gbk, text/html, */*")],
            b"x",
        ),
        // Whitespace around '=' (accepted by browsers).
        "/spaced" => response(
            "200 OK",
            &[("Content-Type", "text/html; charset = \"UTF-8\" ")],
            b"x",
        ),
        // Bytes that are not text in the header: sniff instead.
        _ => b"HTTP/1.1 200 OK\r\nContent-Type: text/ht\xFFml\r\nContent-Length: 6\r\nConnection: close\r\n\r\n<html>".to_vec(),
    }
    });
    let f = test_fetcher();
    let get = |path: &str| {
        let r = f.fetch(&server.url(path)).unwrap();
        (r.content_type, r.charset)
    };
    assert_eq!(get("/two"), (Some("text/html".into()), None));
    assert_eq!(get("/list"), (Some("text/html".into()), Some("gbk".into())));
    assert_eq!(
        get("/spaced"),
        (Some("text/html".into()), Some("utf-8".into()))
    );
    assert_eq!(get("/binary"), (Some("text/html".into()), None));
}

#[test]
fn hostile_cookies_do_not_panic() {
    let server = TestServer::start(|_| {
        response(
            "200 OK",
            &[
                ("Set-Cookie", "a=b; Max-Age=99999999999999999999999999"),
                ("Set-Cookie", "c=d; Expires=Fri, 31 Dec 99999 23:59:59 GMT"),
                ("Set-Cookie", "e=f; Max-Age=-99999999999999999999"),
                ("Set-Cookie", "=; Domain=..; Path="),
                ("Set-Cookie", "g=h; Domain=com"),
                (
                    "Set-Cookie",
                    "weird=\"; Secure; HttpOnly; SameSite=Bogus; Path=/\u{e9}",
                ),
            ],
            b"ok",
        )
    });
    let fetcher = test_fetcher();
    for _ in 0..2 {
        assert_eq!(fetcher.fetch(&server.url("/")).unwrap().body, b"ok");
    }
}

#[test]
fn unusual_urls_reach_the_server() {
    let server = TestServer::start(|req| response("200 OK", &[], req.target.as_bytes()));
    let url = Url::parse(&format!(
        "http://user:p%40ss@{}/a{{b}}|c^d`e?q={{x}}|\"<>#frag",
        server.addr
    ))
    .unwrap();
    let r = test_fetcher().fetch(&url).unwrap();
    assert_eq!(r.body, b"/a%7Bb%7D%7Cc%5Ed%60e?q=%7Bx%7D%7C%22%3C%3E");
    assert_eq!(r.url, url);
}

#[test]
fn hostile_responses_are_errors_not_panics() {
    let truncated = TestServer::start(|_| {
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort".to_vec()
    });
    let silent = TestServer::start(|_| Vec::new());
    let huge_header = TestServer::start(|_| {
        let mut v = b"HTTP/1.1 200 OK\r\nX-Big: ".to_vec();
        v.extend(std::iter::repeat_n(b'a', 200_000));
        v.extend_from_slice(b"\r\nContent-Length: 0\r\n\r\n");
        v
    });
    let bad_chunks = TestServer::start(|_| {
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nZZZ\r\nabc\r\n0\r\n\r\n"
            .to_vec()
    });
    let fetcher = fetcher_with(1024 * 1024, Duration::from_secs(10));
    for (server, needle) in [
        (&truncated, "closed unexpectedly"),
        (&silent, "127.0.0.1"),
        (&huge_header, "too large"),
        (&bad_chunks, "127.0.0.1"),
    ] {
        match fetcher.fetch(&server.url("/")) {
            Err(NetError::Io(msg)) => assert!(msg.contains(needle), "{msg}"),
            other => panic!("expected an I/O error, got {other:?}"),
        }
    }
}

#[test]
fn unusual_status_codes_are_returned() {
    let server = TestServer::start(|req| {
        let status = &req.path()[1..];
        format!("HTTP/1.1 {status} Whatever\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .into_bytes()
    });
    let f = test_fetcher();
    for status in [299_u16, 418, 451, 599, 999] {
        let r = f.fetch(&server.url(&format!("/{status}"))).unwrap();
        assert_eq!(r.status, status);
        assert_eq!(r.body, b"ok");
    }
    // 300 and 305 are not followed even with a Location.
    let server =
        TestServer::start(|_| response("300 Multiple Choices", &[("Location", "/x")], b"pick"));
    let r = f.fetch(&server.url("/")).unwrap();
    assert_eq!((r.status, r.body.as_slice()), (300, b"pick".as_slice()));
}

#[test]
fn brotli_bodies_are_decoded_and_limited_after_decoding() {
    // A complete brotli stream for "hello\n" (one uncompressed meta-block).
    const HELLO_BR: &[u8] = b"\x8f\x02\x80hello\n\x03";
    let server = TestServer::start(|_| {
        response(
            "200 OK",
            &[("Content-Encoding", "br"), ("Content-Type", "text/plain")],
            HELLO_BR,
        )
    });
    let ok = fetcher_with(6, Duration::from_secs(10));
    assert_eq!(ok.fetch(&server.url("/")).unwrap().body, b"hello\n");
    let small = fetcher_with(5, Duration::from_secs(10));
    assert_eq!(small.fetch(&server.url("/")), Err(NetError::TooLarge));
}
