//! Loading over HTTP from a local server: subresources, redirects, error
//! statuses, POST and non-HTML content types.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::OnceLock;

use common::*;
use xbro_engine::{ClickAction, NavigationRequest, Point, Url};
use xbro_style::Color;

/// Starts (once) a server on 127.0.0.1 and returns its base URL.
fn server() -> &'static Url {
    static BASE: OnceLock<Url> = OnceLock::new();
    BASE.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || {
                    let _ = handle(stream);
                });
            }
        });
        Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap()
    })
}

const INDEX: &str = "<!DOCTYPE html><html><head><title>Local server</title>\
    <link rel=stylesheet href=/style.css></head><body>\
    <h1 id=h>Served over HTTP</h1><img id=logo src=img/logo.png>\
    <div id=bg style='height: 20px'></div>\
    <form action=/echo method=post><input name=who value='Ferris & co'>\
    <input type=submit id=go value=Send></form></body></html>";

const STYLE: &str = "@import url(imported.css);\nh1 { color: rgb(10, 20, 30) }\n\
    #bg { background-image: url(img/tile.png) }";

const IMPORTED: &str = "body { background-color: rgb(250, 240, 230) }";

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, extra: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn handle(mut stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/").to_string();
    let path = path.split('?').next().unwrap_or("/");
    let fixture = |name: &str| std::fs::read(fixtures_dir().join(name)).unwrap_or_default();
    match (method, path) {
        ("GET", "/") => respond(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            "",
            INDEX.as_bytes(),
        ),
        ("GET", "/style.css") => respond(&mut stream, "200 OK", "text/css", "", STYLE.as_bytes()),
        ("GET", "/imported.css") => {
            respond(&mut stream, "200 OK", "text/css", "", IMPORTED.as_bytes())
        }
        ("GET", "/img/logo.png") => respond(
            &mut stream,
            "200 OK",
            "image/png",
            "",
            &fixture("img/logo.png"),
        ),
        ("GET", "/img/tile.png") => respond(
            &mut stream,
            "200 OK",
            "image/png",
            "",
            &fixture("img/tile.png"),
        ),
        ("GET", "/redirect") => respond(
            &mut stream,
            "302 Found",
            "text/html",
            "Location: /\r\n",
            b"",
        ),
        ("GET", "/missing") => respond(
            &mut stream,
            "404 Not Found",
            "text/html",
            "",
            b"<title>Custom 404</title><p>Nothing here, sorry.</p>",
        ),
        ("GET", "/empty404") => respond(&mut stream, "404 Not Found", "text/html", "", b""),
        ("GET", "/latin1") => respond(
            &mut stream,
            "200 OK",
            "text/html; charset=iso-8859-1",
            "",
            b"<title>caf\xe9</title><p>d\xe9j\xe0</p>",
        ),
        ("GET", "/plain") => respond(&mut stream, "200 OK", "text/plain", "", b"<not html>"),
        ("GET", "/broken-css") => respond(
            &mut stream,
            "200 OK",
            "text/html",
            "",
            b"<link rel=stylesheet href=/nope.css><link rel=stylesheet href=/missing><p>x",
        ),
        ("POST", "/echo") => {
            let html = format!(
                "<title>Echo</title><pre id=body>{}</pre>",
                String::from_utf8_lossy(&body)
            );
            respond(&mut stream, "200 OK", "text/html", "", html.as_bytes())
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", "", b""),
    }
    Ok(())
}

#[test]
fn subresources_over_http() {
    let base = server();
    let page = load_url(base, VIEWPORT);
    assert_eq!(page.title(), "Local server");
    assert!(page.console().is_empty(), "{:?}", page.console());
    let h = by_id(&page, "h");
    assert_eq!(page.styles().get(h).unwrap().color, Color::rgb(10, 20, 30));
    // @import from the linked sheet.
    let bg = page.display_list().background;
    assert_eq!((bg.r, bg.g, bg.b), (250, 240, 230));
    let logo = page
        .images()
        .node(by_id(&page, "logo"))
        .expect("logo over http");
    assert_eq!(logo.natural_size().width, 64.0);
    let tile = base.join("img/tile.png").unwrap();
    assert!(
        page.images().url(tile.as_str()).is_some(),
        "CSS background over http"
    );
}

#[test]
fn redirects_give_the_final_url() {
    let base = server();
    let page = load_url(&base.join("redirect").unwrap(), VIEWPORT);
    assert_eq!(page.url(), base);
    assert_eq!(page.title(), "Local server");
}

#[test]
fn http_errors_show_the_body_or_an_error_page() {
    let base = server();
    let page = load_url(&base.join("missing").unwrap(), VIEWPORT);
    assert_eq!(page.title(), "Custom 404");
    assert!(laid_out_text(&page).contains("Nothing here, sorry."));
    assert!(page.console().iter().any(|m| m.contains("404")));
    let page = load_url(&base.join("empty404").unwrap(), VIEWPORT);
    assert_eq!(page.title(), "404 Not Found");
    // Failed stylesheets are reported and skipped.
    let page = load_url(&base.join("broken-css").unwrap(), VIEWPORT);
    assert_eq!(page.console().len(), 2, "{:?}", page.console());
}

#[test]
fn charsets_and_text_content() {
    let base = server();
    let page = load_url(&base.join("latin1").unwrap(), VIEWPORT);
    assert_eq!(page.title(), "café");
    assert!(laid_out_text(&page).contains("déjà"));
    let page = load_url(&base.join("plain").unwrap(), VIEWPORT);
    assert!(laid_out_text(&page).contains("<not html>"));
}

#[test]
fn post_submission_round_trip() {
    let base = server();
    let mut page = load_url(base, VIEWPORT);
    let p = center_of(&page, "go");
    let ClickAction::Navigate(request) = page.click(p, Point::ZERO) else {
        panic!("submit expected");
    };
    assert!(matches!(request, NavigationRequest::Post { .. }));
    let echoed = engine().load(&request, VIEWPORT);
    assert_eq!(echoed.title(), "Echo");
    let body = echoed.document().text_content(by_id(&echoed, "body"));
    assert_eq!(body, "who=Ferris+%26+co");
    assert_eq!(echoed.url().path(), "/echo");
}

#[test]
fn connection_refused_gives_an_error_page() {
    // Bind then drop a listener to get a port nobody listens on.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
    let page = load_url(&url, VIEWPORT);
    assert_eq!(page.url(), &url);
    assert_eq!(page.title(), "Problem loading page");
    assert_eq!(page.console().len(), 1);
}
