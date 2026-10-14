//! A tiny HTTP/1.1 server on 127.0.0.1 serving canned responses, for tests.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use xbro_net::{Fetcher, FetcherOptions, Url};

/// A request as received by the test server.
#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// The first header named `name` (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The path part of the request target (without the query).
    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }
}

type Handler = dyn Fn(&Request) -> Vec<u8> + Send + Sync;

/// A server running on a background thread until the test process exits.
pub struct TestServer {
    pub addr: SocketAddr,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl TestServer {
    /// Starts a server answering every request with `handler`'s raw bytes.
    pub fn start(handler: impl Fn(&Request) -> Vec<u8> + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let log = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let handler = Arc::clone(&handler);
                let log = Arc::clone(&log);
                thread::spawn(move || serve_connection(stream, &*handler, &log));
            }
        });
        TestServer { addr, requests }
    }

    /// `http://127.0.0.1:<port><path>`.
    pub fn url(&self, path: &str) -> Url {
        Url::parse(&format!("http://{}{}", self.addr, path)).expect("test URL")
    }

    /// Every request received so far, in order.
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().expect("request log").clone()
    }
}

fn serve_connection(mut stream: TcpStream, handler: &Handler, log: &Mutex<Vec<Request>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    // One request per connection: every canned response says `Connection: close`.
    if let Some(request) = read_request(&mut stream) {
        log.lock().expect("request log").push(request.clone());
        let response = handler(&request);
        let _ = stream.write_all(&response);
        let _ = stream.flush();
    }
}

/// Reads one request (head + `Content-Length` body) from `stream`.
pub fn read_request(stream: &mut impl Read) -> Option<Request> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_owned();
    let target = request_line.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Some(Request {
        method,
        target,
        headers,
        body,
    })
}

/// A complete response with `Content-Length` and `Connection: close`.
pub fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {status}\r\n");
    for (name, value) in headers {
        out.push_str(&format!("{name}: {value}\r\n"));
    }
    out.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// A fetcher that never uses environment proxies.
pub fn test_fetcher() -> Fetcher {
    Fetcher::with_options(FetcherOptions {
        use_env_proxy: false,
        ..FetcherOptions::default()
    })
}

/// A fresh, empty temporary directory unique to this test.
pub fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("xbro-net-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}
