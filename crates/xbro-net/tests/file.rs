//! `file:` URL integration tests (temporary files and directory listings).

mod common;

use std::fs;

use common::{temp_dir, test_fetcher};
use xbro_net::{parse_user_input, Fetcher, FetcherOptions, NetError, Url};

#[test]
fn reads_a_file_with_type_from_extension() {
    let dir = temp_dir("read");
    let file = dir.join("page.html");
    fs::write(&file, "<h1>Bonjour</h1>").unwrap();
    let mut url = Url::from_file_path(&file).unwrap();
    url.set_fragment(Some("top"));

    let r = test_fetcher().fetch(&url).unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.url, url);
    assert_eq!(r.content_type.as_deref(), Some("text/html"));
    assert_eq!(r.charset, None);
    assert_eq!(r.body, b"<h1>Bonjour</h1>");

    // The same file through the address bar.
    let typed = parse_user_input(file.to_str().unwrap());
    assert_eq!(test_fetcher().fetch(&typed).unwrap().body, r.body);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn content_types_by_extension_and_sniffing() {
    let dir = temp_dir("types");
    let cases: &[(&str, &[u8], &str)] = &[
        ("style.css", b"a{}", "text/css"),
        ("app.js", b"1", "text/javascript"),
        ("data.json", b"{}", "application/json"),
        ("pic.PNG", b"not really a png", "image/png"),
        ("image.svg", b"<svg/>", "image/svg+xml"),
        ("notes.md", b"# Title", "text/plain"),
        ("main.rs", b"fn main() {}", "text/plain"),
        ("Cargo.toml", b"[package]", "text/plain"),
        ("README", b"plain text", "text/plain"),
        ("index", b"<!DOCTYPE html><p>x", "text/html"),
        ("blob.xyz", b"\x89PNG\r\n\x1a\n....", "image/png"),
        ("raw.bin", b"\0\0\0\x07data", "application/octet-stream"),
    ];
    let fetcher = test_fetcher();
    for (name, content, expected) in cases {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        let r = fetcher.fetch(&Url::from_file_path(&path).unwrap()).unwrap();
        assert_eq!(r.content_type.as_deref(), Some(*expected), "{name}");
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_file_is_not_found() {
    let dir = temp_dir("missing");
    let url = Url::from_file_path(dir.join("nope.html")).unwrap();
    assert_eq!(test_fetcher().fetch(&url), Err(NetError::NotFound));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn file_larger_than_limit_is_rejected() {
    let dir = temp_dir("large");
    let path = dir.join("big.txt");
    fs::write(&path, vec![b'a'; 100]).unwrap();
    let url = Url::from_file_path(&path).unwrap();
    let small = Fetcher::with_options(FetcherOptions {
        use_env_proxy: false,
        max_body: 99,
        ..FetcherOptions::default()
    });
    assert_eq!(small.fetch(&url), Err(NetError::TooLarge));
    let exact = Fetcher::with_options(FetcherOptions {
        use_env_proxy: false,
        max_body: 100,
        ..FetcherOptions::default()
    });
    assert_eq!(exact.fetch(&url).unwrap().body.len(), 100);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn directory_listing() {
    let dir = temp_dir("listing");
    fs::create_dir(dir.join("zeta dir")).unwrap();
    fs::create_dir(dir.join("Alpha")).unwrap();
    fs::write(dir.join("b.txt"), vec![b'x'; 2048]).unwrap();
    fs::write(dir.join("A <weird> & name.html"), "x").unwrap();
    fs::write(dir.join("c.css"), "").unwrap();

    // Without the trailing slash: the response URL is normalized.
    let url = Url::parse(&format!("file://{}", dir.display())).unwrap();
    assert!(!url.as_str().ends_with('/'));
    let r = test_fetcher().fetch(&url).unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.content_type.as_deref(), Some("text/html"));
    assert_eq!(r.charset.as_deref(), Some("utf-8"));
    assert!(r.url.as_str().ends_with('/'));
    assert_eq!(r.url.to_file_path().unwrap(), dir);

    let html = String::from_utf8(r.body).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("<title>Index of "));

    // Parent directory link.
    let parent = Url::from_directory_path(dir.parent().unwrap()).unwrap();
    assert!(html.contains(&format!("<a href=\"{parent}\">../</a>")));

    // Links to every entry, directories with a trailing slash.
    let alpha = Url::from_directory_path(dir.join("Alpha")).unwrap();
    assert!(html.contains(&format!("<a href=\"{alpha}\">Alpha/</a>")));
    let zeta = Url::from_directory_path(dir.join("zeta dir")).unwrap();
    assert!(zeta.as_str().ends_with("zeta%20dir/"));
    assert!(html.contains(&format!("<a href=\"{zeta}\">zeta dir/</a>")));
    let b = Url::from_file_path(dir.join("b.txt")).unwrap();
    assert!(html.contains(&format!("<a href=\"{b}\">b.txt</a>")));

    // Names are escaped, sizes shown.
    assert!(html.contains("A &lt;weird&gt; &amp; name.html"));
    assert!(!html.contains("<weird>"));
    assert!(html.contains("2.0 KiB"));
    assert!(html.contains("0 B"));
    assert!(html.contains("2 directories, 3 files"));

    // Directories first, then files, case-insensitively sorted.
    let order: Vec<usize> = ["Alpha/", "zeta dir/", "A &lt;weird", "b.txt", "c.css"]
        .iter()
        .map(|needle| html.find(needle).unwrap())
        .collect();
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(order, sorted);

    // Following a link from the listing works.
    let child = test_fetcher().fetch(&b).unwrap();
    assert_eq!(child.body.len(), 2048);
    assert_eq!(child.content_type.as_deref(), Some("text/plain"));

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn root_directory_has_no_parent_link() {
    let r = test_fetcher()
        .fetch(&Url::parse("file:///").unwrap())
        .unwrap();
    let html = String::from_utf8(r.body).unwrap();
    assert!(!html.contains("../</a>"));
    assert_eq!(r.url.as_str(), "file:///");
}

#[test]
fn regular_file_with_trailing_slash_is_not_found() {
    let dir = temp_dir("notdir");
    let file = dir.join("page.html");
    fs::write(&file, "x").unwrap();
    let url = Url::parse(&format!("{}/", Url::from_file_path(&file).unwrap())).unwrap();
    assert_eq!(test_fetcher().fetch(&url), Err(NetError::NotFound));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn names_with_url_syntax_round_trip_through_the_listing() {
    let dir = temp_dir("syntax");
    let names = [
        "a#b.txt",
        "q?x=1.txt",
        "100% sure.txt",
        "semi;colon.txt",
        "ünï ✓.txt",
    ];
    for name in names {
        fs::write(dir.join(name), name).unwrap();
    }
    let fetcher = test_fetcher();
    let listing = fetcher
        .fetch(&Url::from_directory_path(&dir).unwrap())
        .unwrap();
    let html = String::from_utf8(listing.body).unwrap();
    for name in names {
        let href = Url::from_file_path(dir.join(name)).unwrap();
        assert!(html.contains(&format!("href=\"{href}\"")), "{name}");
        // The link fragment/query characters are escaped, so it loads the file.
        let r = fetcher.fetch(&href).unwrap();
        assert_eq!(r.body, name.as_bytes(), "{name}");
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn odd_directory_entries_are_listed_without_failing() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;

    let dir = temp_dir("odd");
    // A symlink loop, a dangling symlink, a socket and a non-UTF-8 name.
    symlink(dir.join("loop-b"), dir.join("loop-a")).unwrap();
    symlink(dir.join("loop-a"), dir.join("loop-b")).unwrap();
    symlink(dir.join("missing-target"), dir.join("dangling")).unwrap();
    let socket = dir.join("sock");
    let _listener = UnixListener::bind(&socket).unwrap();
    let latin1 = dir.join(OsStr::from_bytes(b"caf\xE9.txt"));
    let latin1_ok = fs::write(&latin1, "latin1").is_ok();

    let fetcher = test_fetcher();
    let r = fetcher
        .fetch(&Url::from_directory_path(&dir).unwrap())
        .unwrap();
    let html = String::from_utf8(r.body).unwrap();
    for name in ["loop-a", "loop-b", "dangling", "sock"] {
        assert!(html.contains(&format!(">{name}</a>")), "{name}");
    }
    if latin1_ok {
        // Shown lossily, linked with the exact bytes.
        assert!(html.contains("caf\u{FFFD}.txt"));
        let href = Url::from_file_path(&latin1).unwrap();
        assert!(href.as_str().ends_with("caf%E9.txt"));
        assert_eq!(fetcher.fetch(&href).unwrap().body, b"latin1");
    }

    // Loading the special entries fails cleanly (and never blocks).
    assert!(matches!(
        fetcher.fetch(&Url::from_file_path(&socket).unwrap()),
        Err(NetError::Invalid(_))
    ));
    assert_eq!(
        fetcher.fetch(&Url::from_file_path(dir.join("dangling")).unwrap()),
        Err(NetError::NotFound)
    );
    assert!(matches!(
        fetcher.fetch(&Url::from_file_path(dir.join("loop-a")).unwrap()),
        Err(NetError::Io(_))
    ));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn huge_directories_are_truncated() {
    let dir = temp_dir("huge");
    for i in 0..10_005 {
        fs::write(dir.join(format!("f{i:05}.txt")), "").unwrap();
    }
    let r = test_fetcher()
        .fetch(&Url::from_directory_path(&dir).unwrap())
        .unwrap();
    let html = String::from_utf8(r.body).unwrap();
    assert!(html.contains("0 directories, 10000 files (5 more entries not shown)"));
    assert_eq!(html.matches("<tr class=\"file\">").count(), 10_000);
    fs::remove_dir_all(&dir).unwrap();
}
