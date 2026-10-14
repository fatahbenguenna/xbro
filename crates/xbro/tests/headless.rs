//! Runs the `xbro` binary in its window-less modes on the test fixtures.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn xbro(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xbro"))
        .args(args)
        .output()
        .expect("run xbro")
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// A scratch directory removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("xbro-test-{}-{test}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        Self(dir)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Width and height from a PNG file's IHDR chunk.
fn png_size(path: &Path) -> (u32, u32) {
    let data = std::fs::read(path).expect("read png");
    assert_eq!(&data[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    assert_eq!(&data[12..16], b"IHDR");
    let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    (be(&data[16..20]), be(&data[20..24]))
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn screenshot_has_the_scaled_viewport_size() {
    let tmp = TempDir::new("scaled");
    let out = tmp.file("basic@2x.png");
    let o = xbro(&[
        "--screenshot",
        out.to_str().unwrap(),
        "-w",
        "800",
        "-H",
        "600",
        "--scale",
        "2",
        &fixture("basic.html"),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(png_size(&out), (1600, 1200));
    assert!(stdout(&o).contains("1600x1200"));
}

#[test]
fn full_page_screenshot_is_taller_than_the_viewport() {
    let tmp = TempDir::new("full");
    let out = tmp.file("layout-full.png");
    let o = xbro(&[
        "-s",
        out.to_str().unwrap(),
        "-w",
        "800",
        "-H",
        "0",
        &fixture("layout.html"),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let (w, h) = png_size(&out);
    assert_eq!(w, 800);
    assert!(h > 768, "full page height {h}");
}

#[test]
fn screenshot_of_an_internal_page() {
    let tmp = TempDir::new("home");
    let out = tmp.file("home.png");
    let o = xbro(&[
        "-s",
        out.to_str().unwrap(),
        "-w",
        "640",
        "-H",
        "480",
        "about:home",
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(png_size(&out), (640, 480));
}

#[test]
fn dump_dom_prints_the_tree() {
    let o = xbro(&["--dump-dom", &fixture("basic.html")]);
    assert!(o.status.success(), "{}", stderr(&o));
    let dom = stdout(&o);
    for expected in [
        "#document",
        "<html lang=\"fr\">",
        "<title>",
        "<h1>",
        "Le navigateur xbro",
        "<ul>",
        "<pre>",
    ] {
        assert!(dom.contains(expected), "missing {expected:?} in:\n{dom}");
    }
}

#[test]
fn dump_layout_prints_boxes() {
    let o = xbro(&["--dump-layout", "-w", "640", &fixture("basic.html")]);
    assert!(o.status.success(), "{}", stderr(&o));
    let layout = stdout(&o);
    assert!(layout.starts_with("viewport 640×768"), "{layout}");
    assert!(layout.contains("Block <body>"));
    assert!(layout.contains("Le navigateur xbro"));
}

#[test]
fn missing_files_give_an_error_page() {
    // A path that does not exist is treated as address-bar input; the
    // (offline) result is an error page, not a crash.
    let o = xbro(&["--dump-dom", "file:///nonexistent/xbro-test.html"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("#document"));
}

#[test]
fn invalid_arguments_exit_with_code_2() {
    for args in [
        &["--bogus"][..],
        &["--width", "abc", "about:home"],
        &["--width", "0", "about:home"],
        &["--scale", "0", "-s", "x.png", "about:home"],
        &["--dump-dom"],
        &["-s"],
        &["a", "b"],
    ] {
        let o = xbro(args);
        assert_eq!(o.status.code(), Some(2), "args {args:?}");
        assert!(stderr(&o).contains("USAGE"), "args {args:?}");
    }
}

#[test]
fn help_and_version() {
    let o = xbro(&["--help"]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("--screenshot"));
    let o = xbro(&["-V"]);
    assert!(o.status.success());
    assert!(stdout(&o).starts_with("xbro "));
}

#[test]
fn gui_without_a_display_fails_cleanly() {
    let o = Command::new(env!("CARGO_BIN_EXE_xbro"))
        .arg("about:home")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .expect("run xbro");
    if cfg!(target_os = "linux") {
        assert_eq!(o.status.code(), Some(1));
        let err = stderr(&o);
        assert!(err.contains("cannot open a window"), "{err}");
        assert!(err.contains("--screenshot"), "{err}");
        assert!(!err.contains("panicked"), "{err}");
    }
}
