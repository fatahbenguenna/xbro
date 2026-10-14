//! Deterministic end-to-end fuzzing of the whole pipeline.
//!
//! A seeded generator (xorshift) writes random but plausible HTML
//! documents with random CSS: selectors (`:nth-child(… of …)`, `:has()`,
//! `:is()`, attribute selectors, pseudo-elements), properties from a long
//! list with random values (`calc()`, `var()`, huge, negative and
//! NaN-ish numbers, percentages, viewport units), display / position /
//! float / flex / grid / table mixes, deep nesting (up to 300 elements),
//! long words, many floats, `rowspan`/`colspan`, inline SVG, forms and
//! `data:` images, plus rules nested hundreds of levels deep (`@media`,
//! `:is()`, `calc()`, CSS nesting, `var()` chains), cyclic `@import`s and
//! hostile SVG images (huge, deeply nested, blurred). One document in ten
//! is then damaged byte by byte. Each document is loaded from a `file:`
//! URL in a temporary directory (which holds the stylesheets and images
//! it refers to) at a random viewport, resized (sometimes to 0, NaN or
//! huge sizes), poked (hit tests, clicks, find, focus, hover, typing) and
//! rendered into a small pixmap at random scroll offsets and scales.
//!
//! Each document runs on its own thread with the stack size of the
//! browser's loader thread. A panic anywhere (even one caught by the
//! engine's guards), a document taking longer than [`DOC_TIME_LIMIT`],
//! more than [`MEMORY_LIMIT`] of heap at any time, non-finite geometry
//! (boxes, lines, find and focus rectangles, the display list) or an
//! "Internal error" in the page console fails the test, which names the
//! document and writes its HTML to `$TMPDIR/xbro-fuzz-failures/`. Debug
//! runs also catch the debug assertions of dependencies (tiny-skia's
//! caught a rasteriser bug that release builds hide).
//!
//! Environment variables:
//! * `XBRO_FUZZ_SEED=<n>`: base seed (default: fixed);
//! * `XBRO_FUZZ_DOCS=<n>`: number of documents;
//! * `XBRO_FUZZ_ONLY=<i>`: run only document `i`;
//! * `XBRO_FUZZ_DUMP=<dir>`: write each document there before running it
//!   (finds the culprit of a crash that kills the process, like a stack
//!   overflow);
//! * `XBRO_FUZZ_KEEP=1`: keep the temporary site (stylesheets and images
//!   the documents refer to) after the run;
//! * `XBRO_FUZZ_VERBOSE=1`: print the time of every step;
//! * `XBRO_FUZZ_REPLAY=<file>`: run that HTML file with the viewports and
//!   operations of document `XBRO_FUZZ_ONLY` (default 0), to minimise a
//!   failing input by hand.
//!
//! `cargo test -p xbro-engine --test fuzz_pipeline` runs 300 documents;
//! `cargo test -p xbro-engine --release --test fuzz_pipeline -- --ignored
//! --nocapture` runs 5 000.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use common::{engine, fixtures_dir};
use xbro_engine::{Page, Point, RasterOptions, Size, Url};
use xbro_layout::{Fragment, LayoutBox, Rect};
use xbro_paint::tiny_skia::Pixmap;

// ---- resource accounting ------------------------------------------------

/// Heap allocator that tracks the live and peak number of bytes, and
/// refuses allocations past a hard cap (so that a runaway document aborts
/// the test instead of the machine).
struct CountingAlloc;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// Allocations that would take the live heap past this fail (and abort).
const HARD_HEAP_CAP: usize = 6 << 30;

/// Peak heap growth allowed while one document is processed.
const MEMORY_LIMIT: usize = 1 << 30;

fn charge(bytes: usize) -> bool {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    if live > HARD_HEAP_CAP {
        LIVE.fetch_sub(bytes, Ordering::Relaxed);
        return false;
    }
    PEAK.fetch_max(live, Ordering::Relaxed);
    true
}

// SAFETY: every method forwards to `System` with the same arguments and
// only adds bookkeeping.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !charge(layout.size()) {
            return std::ptr::null_mut();
        }
        let p = System.alloc(layout);
        if p.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !charge(layout.size()) {
            return std::ptr::null_mut();
        }
        let p = System.alloc_zeroed(layout);
        if p.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let old = layout.size();
        if new_size > old && !charge(new_size - old) {
            return std::ptr::null_mut();
        }
        let p = System.realloc(ptr, layout, new_size);
        if p.is_null() {
            if new_size > old {
                LIVE.fetch_sub(new_size - old, Ordering::Relaxed);
            }
        } else if new_size < old {
            LIVE.fetch_sub(old - new_size, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: CountingAlloc = CountingAlloc;

/// Panics seen by the process since the last [`take_panics`] (the engine
/// catches its own, so they would go unnoticed otherwise).
static PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn install_panic_recorder() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            let entry = format!("thread {:?}: {info}", thread.name().unwrap_or("?"));
            if let Ok(mut panics) = PANICS.lock() {
                panics.push(entry);
            }
            default(info);
        }));
    });
}

fn take_panics() -> Vec<String> {
    match PANICS.lock() {
        Ok(mut p) => std::mem::take(&mut *p),
        Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
    }
}

/// Longest a document may take (load, interactions and renders): 5 s in
/// release builds, three times that in debug builds (several times
/// slower), so that a loaded machine does not make the default run flaky.
const DOC_TIME_LIMIT: Duration = if cfg!(debug_assertions) {
    Duration::from_secs(15)
} else {
    Duration::from_secs(5)
};

/// Stack of the worker threads: the default of spawned threads, which is
/// what the browser's loader thread gets.
const WORKER_STACK: usize = 2 << 20;

// ---- random numbers -----------------------------------------------------

/// xorshift64* generator.
#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // Avoid the all-zero state and decorrelate nearby seeds.
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        for _ in 0..4 {
            r.next();
        }
        r
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi.saturating_sub(lo) + 1)
    }

    /// True with probability `percent`/100.
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn f32(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.unit() * (hi - lo)
    }

    fn pick<'a, T: ?Sized>(&mut self, items: &[&'a T]) -> &'a T {
        items[self.below(items.len())]
    }
}

// ---- vocabulary ---------------------------------------------------------

const WORDS: &[&str] = &[
    "lorem",
    "ipsum",
    "dolor",
    "sit",
    "amet",
    "the",
    "browser",
    "layout",
    "of",
    "a",
    "x",
    "consectetur",
    "Wikipedia",
    "article",
    "table",
    "float",
    "flex",
    "grid",
    "and",
    "is",
    "123",
    "4.5",
    "—",
    "«quoted»",
    "naïve",
    "café",
    "日本語",
    "中文",
    "한국어",
    "עברית",
    "العربية",
    "Ελληνικά",
    "русский",
    "🙂",
    "👍🏽",
    "e\u{301}",
    "a\u{200b}b",
    "soft\u{ad}hyphen",
    "no\u{a0}break",
    "tab\there",
    "&amp;",
    "&lt;b&gt;",
    "&nbsp;",
    "&#x1F600;",
    "&#0;",
    "&#xD800;",
    "&#1114112;",
    "&notanentity;",
    "&copy",
    "http://example.com/a/b?c=d",
];

const CLASSES: usize = 12;
const IDS: usize = 20;
const VARS: usize = 8;

const UNITS: &[&str] = &[
    "px", "px", "px", "em", "rem", "%", "%", "vw", "vh", "vmin", "vmax", "ex", "ch", "pt", "pc",
    "cm", "mm", "in", "Q", "lh", "svh", "dvw", "fr", "deg", "s", "",
];

const NUMBERS: &[&str] = &[
    "0",
    "-0",
    "1",
    "-1",
    "2",
    "0.5",
    ".5",
    "-.25",
    "+3",
    "1e3",
    "1E3",
    "-1e3",
    "1e-7",
    "99999",
    "-99999",
    "2147483647",
    "-2147483648",
    "4294967296",
    "1e10",
    "1e20",
    "1e38",
    "3.4e38",
    "-3.4e38",
    "1e39",
    "-1e39",
    "1e308",
    "1e400",
    "1e-45",
    "0.0000001",
    "123456789.123",
];

const COLORS: &[&str] = &[
    "red",
    "blue",
    "green",
    "black",
    "white",
    "transparent",
    "currentcolor",
    "currentColor",
    "rebeccapurple",
    "#f00",
    "#0f08",
    "#123456",
    "#12345678",
    "#abc",
    "#12",
    "#ggg",
    "rgb(0 0 0)",
    "rgb(255, 0, 0)",
    "rgba(0,0,255,.5)",
    "rgb(300 -5 1e9)",
    "rgb(10% 20% 30% / 40%)",
    "rgb(calc(infinity) 0 0)",
    "rgba(0,0,0,NaN)",
    "hsl(120 100% 50%)",
    "hsla(-720, 50%, 50%, 2)",
    "hsl(1e39deg 0% 0%)",
    "hwb(90 10% 10%)",
    "lab(50% 40 59)",
    "oklch(70% 0.1 120)",
    "color-mix(in srgb, red, blue)",
    "inherit",
    "initial",
    "unset",
    "Canvas",
    "CanvasText",
    "LinkText",
    "nonsense",
    "rgb(",
    "#",
];

const DISPLAYS: &[&str] = &[
    "block",
    "inline",
    "inline-block",
    "flex",
    "inline-flex",
    "grid",
    "inline-grid",
    "table",
    "inline-table",
    "table-row",
    "table-cell",
    "table-row-group",
    "table-header-group",
    "table-footer-group",
    "table-column",
    "table-column-group",
    "table-caption",
    "list-item",
    "contents",
    "none",
    "flow-root",
    "run-in",
    "ruby",
    "block flow",
    "inline flow-root",
    "block flex",
    "math",
    "-webkit-box",
    "bogus",
];

const POSITIONS: &[&str] = &[
    "static",
    "relative",
    "absolute",
    "fixed",
    "sticky",
    "-webkit-sticky",
];

const FLOATS: &[&str] = &[
    "left",
    "right",
    "none",
    "inline-start",
    "inline-end",
    "center",
];

const FONTS: &[&str] = &[
    "serif",
    "sans-serif",
    "monospace",
    "cursive",
    "fantasy",
    "system-ui",
    "Arial, sans-serif",
    "'DejaVu Sans'",
    "\"Times New Roman\", Times, serif",
    "Nonexistent Font",
    "emoji",
    "math",
    "inherit",
];

const BORDER_STYLES: &[&str] = &[
    "solid", "dashed", "dotted", "double", "groove", "ridge", "inset", "outset", "none", "hidden",
];

/// Element names used as type selectors.
const TAGS: &[&str] = &[
    "div", "p", "span", "a", "li", "ul", "table", "td", "tr", "th", "img", "input", "b", "h1",
    "h2", "section", "svg", "rect", "button", "form", "label", "select", "html", "body",
];

// ---- CSS generation -----------------------------------------------------

struct Gen<'a> {
    rng: &'a mut Rng,
    images: &'a [String],
    /// Percentage of lengths drawn from the hostile numbers (per document).
    extreme: usize,
}

impl Gen<'_> {
    fn number(&mut self) -> String {
        if self.rng.chance(70) {
            let n = self.rng.below(200) as f32 / [1.0, 2.0, 10.0][self.rng.below(3)];
            format!("{n}")
        } else {
            self.rng.pick(NUMBERS).to_string()
        }
    }

    fn length(&mut self) -> String {
        if self.rng.chance(self.extreme) {
            return format!("{}{}", self.rng.pick(NUMBERS), self.rng.pick(UNITS));
        }
        match self.rng.below(20) {
            0 => self.calc(0),
            1 => self.var_ref(),
            2 => "0".to_string(),
            3 => format!("{}{}", self.rng.pick(NUMBERS), self.rng.pick(UNITS)),
            _ => {
                let unit = if self.rng.chance(60) {
                    "px"
                } else {
                    self.rng.pick(UNITS)
                };
                let n = self.rng.below(400) as i32 - if self.rng.chance(15) { 200 } else { 0 };
                format!("{n}{unit}")
            }
        }
    }

    fn length_or(&mut self, keywords: &[&str]) -> String {
        if self.rng.chance(25) {
            self.rng.pick(keywords).to_string()
        } else {
            self.length()
        }
    }

    fn calc(&mut self, depth: usize) -> String {
        let f = ["calc", "calc", "min", "max", "clamp", "calc"][self.rng.below(6)];
        let args = match f {
            "clamp" => 3,
            "min" | "max" => self.rng.range(1, 3),
            _ => 1,
        };
        let mut parts = Vec::new();
        for _ in 0..args {
            let mut expr = self.calc_term(depth);
            for _ in 0..self.rng.below(3) {
                let op = [" + ", " - ", " * ", " / ", "+", " "][self.rng.below(6)];
                let _ = write!(expr, "{op}{}", self.calc_term(depth));
            }
            parts.push(expr);
        }
        format!("{f}({})", parts.join(", "))
    }

    fn calc_term(&mut self, depth: usize) -> String {
        match self.rng.below(12) {
            0 if depth < 3 => self.calc(depth + 1),
            1 if depth < 3 => format!("({})", self.calc_term(depth + 1)),
            2 => ["infinity", "-infinity", "NaN", "e", "pi", "-INFINITY"][self.rng.below(6)]
                .to_string(),
            3 => self.var_ref(),
            4 | 5 => self.number(),
            6 => format!("{}%", self.number()),
            _ => format!("{}{}", self.number(), self.rng.pick(UNITS)),
        }
    }

    fn var_ref(&mut self) -> String {
        let v = self.rng.below(VARS + 2);
        match self.rng.below(4) {
            0 => format!("var(--v{v})"),
            1 => format!("var(--v{v}, {})", self.simple_value()),
            2 => format!("var(--v{v},)"),
            _ => format!("var(--v{v}, var(--v{}, 7px))", self.rng.below(VARS)),
        }
    }

    fn simple_value(&mut self) -> String {
        match self.rng.below(5) {
            0 => self.color(),
            1 => self.number(),
            2 => self.rng.pick(DISPLAYS).to_string(),
            _ => format!("{}px", self.rng.below(100)),
        }
    }

    fn color(&mut self) -> String {
        match self.rng.below(12) {
            0 => self.var_ref(),
            1 => format!(
                "rgb({} {} {})",
                self.rng.below(256),
                self.rng.below(256),
                self.rng.below(256)
            ),
            2 => format!("#{:06x}", self.rng.next() & 0xff_ffff),
            _ => self.rng.pick(COLORS).to_string(),
        }
    }

    fn image(&mut self) -> String {
        match self.rng.below(8) {
            0 => format!(
                "linear-gradient({}, {})",
                [
                    "to right",
                    "45deg",
                    "to bottom left",
                    "1e9deg",
                    "-90deg",
                    "0.25turn"
                ][self.rng.below(6)],
                (0..self.rng.range(1, 6))
                    .map(|_| {
                        let mut s = self.color();
                        if self.rng.chance(50) {
                            let _ = write!(s, " {}", self.length());
                        }
                        s
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            1 => "repeating-linear-gradient(red 0 1px, blue 1px 2px)".to_string(),
            2 => "radial-gradient(circle, red, blue)".to_string(),
            3 => "none".to_string(),
            4 => "url()".to_string(),
            _ => format!("url(\"{}\")", self.rng.pick_owned(self.images)),
        }
    }

    fn keyword(&mut self, words: &[&str]) -> String {
        self.rng.pick(words).to_string()
    }

    fn grid_tracks(&mut self) -> String {
        let n = self.rng.range(1, 5);
        let mut tracks = Vec::new();
        for _ in 0..n {
            let t = match self.rng.below(12) {
                0 => format!("repeat({}, {})", self.rng.range(0, 30), self.track()),
                1 => format!("repeat(auto-fill, minmax({}, 1fr))", self.length()),
                2 => format!("repeat(auto-fit, {})", self.length()),
                3 => format!("repeat({}, 1fr)", self.rng.pick(NUMBERS)),
                _ => self.track(),
            };
            tracks.push(t);
        }
        tracks.join(" ")
    }

    fn track(&mut self) -> String {
        match self.rng.below(9) {
            0 => format!("minmax({}, {})", self.length(), self.track_breadth()),
            1 => format!("fit-content({})", self.length()),
            2 => "[a] 1fr [b]".to_string(),
            _ => self.track_breadth(),
        }
    }

    fn track_breadth(&mut self) -> String {
        match self.rng.below(7) {
            0 => format!("{}fr", self.number()),
            1 => "auto".to_string(),
            2 => "min-content".to_string(),
            3 => "max-content".to_string(),
            _ => self.length(),
        }
    }

    fn grid_line(&mut self) -> String {
        match self.rng.below(7) {
            0 => "auto".to_string(),
            1 => format!(
                "span {}",
                self.rng.pick(&["1", "2", "3", "0", "-1", "1000", "99999"])
            ),
            2 => format!(
                "{} / {}",
                self.rng.below(8) as i32 - 3,
                self.rng.below(8) as i32 - 3
            ),
            3 => format!("1 / span {}", self.rng.range(1, 40)),
            4 => self.rng.pick(NUMBERS).to_string(),
            _ => format!("{}", self.rng.below(10) as i32 - 4),
        }
    }

    fn content(&mut self) -> String {
        match self.rng.below(10) {
            0 => "none".to_string(),
            1 => "normal".to_string(),
            2 => format!("url(\"{}\")", self.rng.pick_owned(self.images)),
            3 => "attr(class)".to_string(),
            4 => "counter(c) \". \"".to_string(),
            5 => "open-quote \"q\" close-quote".to_string(),
            6 => format!("\"{}\"", "x".repeat(self.rng.range(0, 300))),
            7 => "\"\\A line\\A break\"".to_string(),
            _ => format!("\"{} \"", self.word()),
        }
    }

    fn word(&mut self) -> String {
        let w = self.rng.pick(WORDS);
        w.replace(['"', '\\'], "")
    }

    /// A value for `property`; sometimes a value meant for another one.
    fn value(&mut self, property: &str) -> String {
        if self.rng.chance(8) {
            return match self.rng.below(8) {
                0 => "inherit".into(),
                1 => "initial".into(),
                2 => "unset".into(),
                3 => "revert".into(),
                4 => self.var_ref(),
                5 => self.calc(0),
                6 => ["", ";", "!important", "}", "{", "(", "url(", "\"", "/*"][self.rng.below(9)]
                    .into(),
                _ => self.simple_value(),
            };
        }
        let r = &mut *self;
        match property {
            "display" => r.keyword(DISPLAYS),
            "position" => r.keyword(POSITIONS),
            "float" => r.keyword(FLOATS),
            "clear" => r.keyword(&["left", "right", "both", "none", "inline-start"]),
            "top" | "right" | "bottom" | "left" | "inset" | "inset-inline-start"
            | "inset-block-end" => r.length_or(&["auto"]),
            "z-index" | "order" => r
                .rng
                .pick(&[
                    "0",
                    "1",
                    "-1",
                    "auto",
                    "2147483647",
                    "-2147483648",
                    "99999999999",
                    "1.5",
                    "5",
                ])
                .into(),
            "box-sizing" => r.keyword(&["border-box", "content-box", "padding-box"]),
            "width" | "height" | "inline-size" | "block-size" | "flex-basis" => r.length_or(&[
                "auto",
                "min-content",
                "max-content",
                "fit-content",
                "fit-content(50px)",
                "stretch",
                "-webkit-fill-available",
                "content",
            ]),
            "min-width" | "min-height" => r.length_or(&["auto", "min-content", "max-content"]),
            "max-width" | "max-height" => r.length_or(&["none", "min-content", "max-content"]),
            "margin"
            | "padding"
            | "margin-inline"
            | "padding-block"
            | "border-width"
            | "border-radius"
            | "gap"
            | "border-spacing"
            | "background-position"
            | "background-size"
            | "object-position"
            | "inset-block" => {
                let n = r.rng.range(1, 4);
                let extra = if property == "border-radius" && r.rng.chance(20) {
                    format!(" / {}", r.length())
                } else {
                    String::new()
                };
                let values: Vec<String> = (0..n)
                    .map(|_| r.length_or(&["auto", "cover", "contain", "thin", "thick", "center"]))
                    .collect();
                values.join(" ") + &extra
            }
            "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "text-indent"
            | "letter-spacing"
            | "word-spacing"
            | "row-gap"
            | "column-gap"
            | "outline-offset"
            | "border-top-left-radius"
            | "border-bottom-right-radius"
            | "column-width"
            | "tab-size"
            | "vertical-align"
            | "line-height" => r.length_or(&[
                "auto",
                "normal",
                "middle",
                "baseline",
                "top",
                "text-bottom",
                "sub",
                "super",
                "1.5",
                "0",
                "-1",
                "1e9",
            ]),
            "border" | "border-top" | "border-right" | "border-bottom" | "border-left"
            | "outline" | "border-inline" => {
                format!("{} {} {}", r.length(), r.keyword(BORDER_STYLES), r.color())
            }
            "border-style" | "border-top-style" | "border-left-style" | "outline-style" => {
                r.keyword(BORDER_STYLES)
            }
            "border-top-width" | "border-left-width" | "outline-width" => {
                r.length_or(&["thin", "medium", "thick"])
            }
            "color"
            | "background-color"
            | "border-color"
            | "border-top-color"
            | "outline-color"
            | "text-decoration-color"
            | "caret-color"
            | "accent-color" => r.color(),
            "background" => {
                let mut v = String::new();
                if r.rng.chance(50) {
                    v.push_str(&r.image());
                    v.push(' ');
                }
                if r.rng.chance(40) {
                    v.push_str(r.rng.pick(&[
                        "no-repeat ",
                        "repeat-x ",
                        "repeat ",
                        "space ",
                        "round ",
                    ]));
                }
                if r.rng.chance(30) {
                    let _ = write!(
                        v,
                        "{} {} / {} ",
                        r.length(),
                        r.length(),
                        r.length_or(&["cover", "contain", "auto"])
                    );
                }
                v + &r.color()
            }
            "background-image" | "list-style-image" | "border-image" => r.image(),
            "background-repeat" => r.keyword(&[
                "repeat",
                "no-repeat",
                "repeat-x",
                "repeat-y",
                "space",
                "round",
                "repeat no-repeat",
            ]),
            "background-attachment" => r.keyword(&["fixed", "scroll", "local"]),
            "overflow" | "overflow-x" | "overflow-y" => r.keyword(&[
                "visible",
                "hidden",
                "scroll",
                "auto",
                "clip",
                "hidden visible",
                "overlay",
            ]),
            "visibility" => r.keyword(&["visible", "hidden", "collapse"]),
            "opacity" => r
                .rng
                .pick(&[
                    "0",
                    "0.5",
                    "1",
                    "-1",
                    "2",
                    "50%",
                    "1e39",
                    "0.01",
                    "calc(1/0)",
                ])
                .into(),
            "aspect-ratio" => r
                .rng
                .pick(&[
                    "1",
                    "16 / 9",
                    "auto",
                    "0",
                    "1 / 0",
                    "auto 2/1",
                    "-1",
                    "1e30 / 1e-30",
                ])
                .into(),
            "font" => format!(
                "{} {} {}/{} {}",
                r.keyword(&["italic", "normal", "oblique", ""]),
                r.keyword(&["bold", "100", "900", "normal", "bolder", ""]),
                r.length_or(&["small", "large", "xx-large", "smaller", "larger"]),
                r.length_or(&["normal", "1.2", "0"]),
                r.keyword(FONTS)
            ),
            "font-family" => r.keyword(FONTS),
            "font-size" => r.length_or(&[
                "medium",
                "small",
                "x-large",
                "xxx-large",
                "smaller",
                "larger",
                "0",
                "1e6px",
                "-5px",
                "100000px",
            ]),
            "font-weight" => r
                .rng
                .pick(&[
                    "bold", "normal", "100", "950", "1000", "1", "0", "bolder", "lighter", "550",
                ])
                .into(),
            "font-style" => r.keyword(&["italic", "oblique", "normal", "oblique 10deg"]),
            "font-variant" | "font-stretch" => {
                r.keyword(&["small-caps", "condensed", "normal", "150%"])
            }
            "text-align" => r.keyword(&[
                "left",
                "right",
                "center",
                "justify",
                "start",
                "end",
                "match-parent",
                "-webkit-center",
                "justify-all",
            ]),
            "text-transform" => {
                r.keyword(&["uppercase", "lowercase", "capitalize", "none", "full-width"])
            }
            "white-space" | "white-space-collapse" | "text-wrap" | "text-wrap-mode" => {
                r.keyword(&[
                    "normal",
                    "pre",
                    "nowrap",
                    "pre-wrap",
                    "pre-line",
                    "break-spaces",
                    "wrap",
                    "balance",
                    "collapse",
                    "preserve",
                ])
            }
            "overflow-wrap" | "word-wrap" | "word-break" | "line-break" | "hyphens" => {
                r.keyword(&[
                    "normal",
                    "break-word",
                    "anywhere",
                    "break-all",
                    "keep-all",
                    "auto",
                    "manual",
                ])
            }
            "list-style" | "list-style-type" | "list-style-position" => r.keyword(&[
                "disc",
                "circle",
                "square",
                "decimal",
                "lower-roman",
                "upper-alpha",
                "none",
                "inside",
                "outside",
                "decimal-leading-zero",
                "georgian",
                "\"→\"",
                "square inside",
                "hebrew",
                "cjk-ideographic",
            ]),
            "border-collapse" => r.keyword(&["collapse", "separate"]),
            "caption-side" => r.keyword(&["top", "bottom", "left"]),
            "table-layout" => r.keyword(&["auto", "fixed"]),
            "empty-cells" => r.keyword(&["show", "hide"]),
            "text-decoration" | "text-decoration-line" => r.keyword(&[
                "underline",
                "line-through",
                "overline",
                "none",
                "underline overline",
                "underline wavy red",
                "blink",
            ]),
            "flex" => r
                .rng
                .pick(&[
                    "1",
                    "auto",
                    "none",
                    "0 0 auto",
                    "1 1 0",
                    "2 0 100px",
                    "1e9",
                    "-1",
                    "0 1 50%",
                    "1 1 calc(100% - 1e9px)",
                    "initial",
                ])
                .into(),
            "flex-direction" => r.keyword(&["row", "column", "row-reverse", "column-reverse"]),
            "flex-wrap" => r.keyword(&["wrap", "nowrap", "wrap-reverse"]),
            "flex-flow" => r.keyword(&["row wrap", "column nowrap", "column-reverse wrap-reverse"]),
            "flex-grow" | "flex-shrink" => r.number(),
            "justify-content" | "align-content" | "place-content" => r.keyword(&[
                "flex-start",
                "flex-end",
                "center",
                "space-between",
                "space-around",
                "space-evenly",
                "stretch",
                "start",
                "end",
                "normal",
                "safe center",
                "left",
                "baseline",
            ]),
            "align-items" | "align-self" | "place-items" | "justify-items" | "justify-self" => r
                .keyword(&[
                    "stretch",
                    "flex-start",
                    "center",
                    "baseline",
                    "flex-end",
                    "auto",
                    "start",
                    "end",
                    "last baseline",
                    "normal",
                ]),
            "grid-template-columns"
            | "grid-template-rows"
            | "grid-auto-columns"
            | "grid-auto-rows" => r.grid_tracks(),
            "grid-template" => format!("{} / {}", r.grid_tracks(), r.grid_tracks()),
            "grid-column" | "grid-row" | "grid-area" | "grid-column-start" | "grid-row-end" => {
                r.grid_line()
            }
            "grid-auto-flow" => r.keyword(&["row", "column", "dense", "row dense"]),
            "grid-template-areas" => "\"a b\" \"c c\"".into(),
            "content" => r.content(),
            "counter-reset" | "counter-increment" => {
                r.keyword(&["c", "c 5", "c -2147483648", "none", "c 1e10"])
            }
            "quotes" => r.keyword(&["none", "auto", "'«' '»'", "\"a\""]),
            "cursor" => r.keyword(&[
                "pointer",
                "text",
                "default",
                "wait",
                "url(x.cur), auto",
                "help",
            ]),
            "transform" => r.keyword(&[
                "rotate(45deg)",
                "scale(1e9)",
                "translate(10px, 10px)",
                "none",
                "matrix(1,0,0,1,0,0)",
            ]),
            "columns" | "column-count" => r.keyword(&["2", "3 100px", "auto", "1000"]),
            "writing-mode" => {
                r.keyword(&["horizontal-tb", "vertical-rl", "vertical-lr", "sideways-rl"])
            }
            "direction" => r.keyword(&["ltr", "rtl"]),
            "unicode-bidi" => {
                r.keyword(&["normal", "embed", "bidi-override", "isolate", "plaintext"])
            }
            "box-shadow" | "text-shadow" | "filter" => r.keyword(&[
                "0 0 5px red",
                "none",
                "1e9px 1e9px 1e9px black",
                "blur(10px)",
                "inset 0 0 0 1px blue",
            ]),
            "object-fit" => r.keyword(&["contain", "cover", "fill", "none", "scale-down"]),
            "text-overflow" => r.keyword(&["ellipsis", "clip", "\"…\""]),
            "user-select" | "pointer-events" | "resize" | "isolation" | "contain"
            | "will-change" | "mix-blend-mode" => r.keyword(&[
                "none",
                "auto",
                "all",
                "multiply",
                "both",
                "layout paint",
                "isolate",
            ]),
            "all" => r.keyword(&["initial", "unset", "revert", "inherit"]),
            _ => r.simple_value(),
        }
    }

    fn declaration(&mut self) -> String {
        let name = if self.rng.chance(10) {
            format!("--v{}", self.rng.below(VARS))
        } else {
            self.rng.pick(PROPERTIES).to_string()
        };
        let value = if name.starts_with("--") {
            self.custom_value()
        } else {
            self.value(&name)
        };
        let important = if self.rng.chance(5) {
            " !important"
        } else {
            ""
        };
        format!("{name}: {value}{important}")
    }

    fn custom_value(&mut self) -> String {
        match self.rng.below(10) {
            0 => {
                // Exponential expansion: each variable uses the previous one many times.
                let v = self.rng.below(VARS);
                format!("var(--v{v}) var(--v{v}) var(--v{v}) var(--v{v})")
            }
            1 => format!("var(--v{})", self.rng.below(VARS)),
            2 => self.color(),
            3 => self.calc(0),
            4 => "{ a: b }".to_string(),
            5 => "".to_string(),
            6 => self.rng.pick(DISPLAYS).to_string(),
            _ => self.length(),
        }
    }

    fn declarations(&mut self, n: usize) -> String {
        (0..n)
            .map(|_| self.declaration())
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn an_plus_b(&mut self) -> String {
        match self.rng.below(12) {
            0 => "odd".into(),
            1 => "even".into(),
            2 => format!("{}", self.rng.below(6) as i32 - 1),
            3 => format!("{}n+{}", self.rng.below(5) as i32 - 2, self.rng.below(6)),
            4 => "-n+3".into(),
            5 => "n".into(),
            6 => "2147483647n+2147483647".into(),
            7 => "-2147483648n-2147483648".into(),
            8 => format!("2n+1 of {}", self.compound(2)),
            9 => "99999999999999999999".into(),
            10 => "3n - 1".into(),
            _ => "garbage".into(),
        }
    }

    fn compound(&mut self, depth: usize) -> String {
        // Mostly simple selectors, so that rules apply to many elements.
        match self.rng.below(10) {
            0..=2 => return format!(".c{}", self.rng.below(CLASSES)),
            3 => return self.rng.pick(TAGS).to_string(),
            4 => return format!("{}.c{}", self.rng.pick(TAGS), self.rng.below(CLASSES)),
            _ => {}
        }
        let mut s = String::new();
        match self.rng.below(5) {
            0 => s.push_str(self.rng.pick(TAGS)),
            1 => s.push('*'),
            _ => {}
        }
        for _ in 0..self.rng.range(if s.is_empty() { 1 } else { 0 }, 2) {
            match self.rng.below(16) {
                0..=4 => {
                    let _ = write!(s, ".c{}", self.rng.below(CLASSES));
                }
                5 => {
                    let _ = write!(s, "#i{}", self.rng.below(IDS));
                }
                6 => s.push_str(self.rng.pick(&[
                    "[href]",
                    "[type=checkbox]",
                    "[class~=c1]",
                    "[id^=i1]",
                    "[lang|=en]",
                    "[title*='a' i]",
                    "[data-x$=\"\"]",
                    "[style]",
                ])),
                7 => {
                    let _ = write!(s, ":nth-child({})", self.an_plus_b());
                }
                8 => {
                    let f =
                        [":nth-last-child", ":nth-of-type", ":nth-last-of-type"][self.rng.below(3)];
                    let _ = write!(s, "{f}({})", self.an_plus_b());
                }
                9 if depth < 3 => {
                    let f = [":is", ":where", ":not", ":has"][self.rng.below(4)];
                    let inner = self.complex(depth + 1);
                    let prefix = if f == ":has" {
                        ["", "> ", "+ ", "~ "][self.rng.below(4)]
                    } else {
                        ""
                    };
                    let _ = write!(s, "{f}({prefix}{inner})");
                }
                _ => s.push_str(self.rng.pick(&[
                    ":first-child",
                    ":last-child",
                    ":only-child",
                    ":first-of-type",
                    ":empty",
                    ":root",
                    ":hover",
                    ":focus",
                    ":focus-within",
                    ":focus-visible",
                    ":checked",
                    ":disabled",
                    ":enabled",
                    ":link",
                    ":visited",
                    ":any-link",
                    ":placeholder-shown",
                    ":target",
                    ":lang(en)",
                    ":dir(rtl)",
                    ":unknown-pseudo",
                ])),
            }
        }
        if s.is_empty() {
            s.push('*');
        }
        s
    }

    fn complex(&mut self, depth: usize) -> String {
        let mut s = self.compound(depth);
        for _ in 0..[0, 0, 1, 1, 2][self.rng.below(5)] {
            let comb = [" ", " > ", " + ", " ~ ", " "][self.rng.below(5)];
            s.push_str(comb);
            s.push_str(&self.compound(depth));
        }
        s
    }

    fn selector_list(&mut self) -> String {
        let n = self.rng.range(1, 3);
        let mut list: Vec<String> = (0..n).map(|_| self.complex(0)).collect();
        if self.rng.chance(15) {
            let pseudo = [
                "::before",
                "::after",
                "::marker",
                "::first-line",
                "::first-letter",
                "::placeholder",
                "::selection",
                ":before",
            ][self.rng.below(8)];
            if let Some(last) = list.last_mut() {
                last.push_str(pseudo);
            }
        }
        list.join(", ")
    }

    fn media_query(&mut self) -> String {
        let n = self.rng.range(1, 3);
        (0..n)
            .map(|_| match self.rng.below(12) {
                0 => "print".to_string(),
                1 => "screen".to_string(),
                2 => "not screen and (color)".to_string(),
                3 => format!("(min-width: {}px)", self.rng.below(4000)),
                4 => format!("(max-width: {}px)", self.rng.below(4000)),
                5 => format!(
                    "({}px < width <= {}px)",
                    self.rng.below(2000),
                    self.rng.below(4000)
                ),
                6 => "(orientation: portrait)".to_string(),
                7 => "(prefers-color-scheme: dark)".to_string(),
                8 => format!(
                    "(min-aspect-ratio: {}/{})",
                    self.rng.below(5),
                    self.rng.below(5)
                ),
                9 => format!("(max-height: {})", self.length()),
                10 => "only screen and (min-resolution: 2dppx)".to_string(),
                _ => "(garbage".to_string(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn rule(&mut self, depth: usize) -> String {
        match self.rng.below(30) {
            0 if depth < 3 => {
                let inner = { let n = self.rng.range(1, 4); self.rules(n, depth + 1) };
                format!("@media {} {{ {inner} }}", self.media_query())
            }
            1 if depth < 3 => {
                let inner = { let n = self.rng.range(1, 3); self.rules(n, depth + 1) };
                let cond = [
                    "(display: grid)",
                    "not (display: bogus)",
                    "(color: red) and (width: calc(1px))",
                    "selector(:has(a))",
                    "(--x: y)",
                    "garbage",
                ][self.rng.below(6)];
                format!("@supports {cond} {{ {inner} }}")
            }
            2 if depth < 3 => {
                let inner = { let n = self.rng.range(1, 3); self.rules(n, depth + 1) };
                format!("@layer l{} {{ {inner} }}", self.rng.below(3))
            }
            3 => format!(
                "@import url({}) {};",
                ["style.css", "style2.css", "missing.css", "img/logo.png", "data:text/css,p{color:red}"]
                    [self.rng.below(5)],
                ["", "screen", "print", "(min-width: 500px)", "layer(x)"][self.rng.below(5)]
            ),
            4 => "@font-face { font-family: X; src: url(nope.woff2) }".to_string(),
            5 => "@keyframes k { from { opacity: 0 } to { opacity: 1 } }".to_string(),
            6 => "@page { margin: 1in } @charset \"utf-8\"; @namespace svg url(http://www.w3.org/2000/svg);".to_string(),
            7 if depth < 3 => {
                // CSS nesting.
                let n = self.rng.range(0, 3);
                let decls = self.declarations(n);
                let nested = self.rule(depth + 1);
                format!(
                    "{} {{ {decls}; & {} {{ {} }} {nested} }}",
                    self.selector_list(),
                    self.compound(1),
                    self.declarations(2)
                )
            }
            8 => ["}}", "{", "@media {", "a { b: c", "/* unterminated", "<!-- -->", ";;"]
                [self.rng.below(7)]
            .to_string(),
            9 if self.rng.chance(20) => self.deep_rule(),
            _ => {
                let n = self.rng.range(1, 6);
                format!("{} {{ {} }}", self.selector_list(), self.declarations(n))
            }
        }
    }

    /// A rule nested hundreds of levels deep in one way or another.
    fn deep_rule(&mut self) -> String {
        let n = self.rng.range(20, 600);
        let rep = |open: &str, close: &str, inner: &str| {
            format!("{}{inner}{}", open.repeat(n), close.repeat(n))
        };
        match self.rng.below(9) {
            0 => rep("@media screen { ", "} ", ".c1 { color: red }"),
            1 => rep("@supports (display: block) { ", "} ", ".c2 { color: blue }"),
            2 => format!("{} {{ color: green }}", rep(":is(", ")", ".c3")),
            3 => format!("{} {{ color: green }}", rep(":not(:has(", "))", ".c4")),
            4 => format!(".c5 {{ width: {} }}", rep("calc(1px + ", ")", "1%")),
            5 => format!(".c6 {{ {} }}", rep("div { ", "} ", "margin: 1px")),
            6 => {
                // A long chain of custom properties.
                let mut css = String::from(":root { --p0: 3px; ");
                for i in 1..n {
                    let _ = write!(css, "--p{i}: var(--p{}) ", i - 1);
                    css.push_str(if i % 2 == 0 { "; " } else { "+ 1px; " });
                }
                let _ = write!(
                    css,
                    "}} .c7 {{ width: calc(var(--p{}) * 1); padding: var(--p{}) }}",
                    n - 1,
                    n / 2
                );
                css
            }
            7 => format!(
                ".c8 {{ color: {} }}",
                rep("color-mix(in srgb, red, ", ")", "blue")
            ),
            _ => format!(
                ".c9 {{ background: {} }}",
                rep("linear-gradient(red, ", ")", "blue")
            ),
        }
    }

    fn rules(&mut self, n: usize, depth: usize) -> String {
        (0..n)
            .map(|_| self.rule(depth))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn stylesheet(&mut self, rules: usize) -> String {
        let mut css = String::new();
        // Custom properties on the root, possibly cyclic.
        let _ = write!(css, ":root {{ ");
        for v in 0..VARS {
            if self.rng.chance(70) {
                let _ = write!(css, "--v{v}: {}; ", self.custom_value());
            }
        }
        css.push_str("}\n");
        css.push_str(&self.rules(rules, 0));
        css
    }
}

/// Properties set by the generator (shorthands, longhands, logical and
/// unsupported ones).
const PROPERTIES: &[&str] = &[
    "display",
    "display",
    "display",
    "position",
    "position",
    "float",
    "float",
    "clear",
    "top",
    "right",
    "bottom",
    "left",
    "inset",
    "inset-inline-start",
    "inset-block-end",
    "z-index",
    "box-sizing",
    "width",
    "width",
    "height",
    "height",
    "min-width",
    "min-height",
    "max-width",
    "max-height",
    "inline-size",
    "block-size",
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "margin-inline",
    "padding",
    "padding-top",
    "padding-left",
    "padding-right",
    "padding-bottom",
    "padding-block",
    "border",
    "border-top",
    "border-right",
    "border-bottom",
    "border-left",
    "border-inline",
    "border-width",
    "border-top-width",
    "border-left-width",
    "border-style",
    "border-top-style",
    "border-left-style",
    "border-color",
    "border-top-color",
    "border-radius",
    "border-top-left-radius",
    "border-bottom-right-radius",
    "border-image",
    "overflow",
    "overflow-x",
    "overflow-y",
    "aspect-ratio",
    "visibility",
    "opacity",
    "background",
    "background",
    "background-color",
    "background-image",
    "background-repeat",
    "background-size",
    "background-position",
    "background-attachment",
    "outline",
    "outline-width",
    "outline-style",
    "outline-color",
    "outline-offset",
    "cursor",
    "color",
    "color",
    "font",
    "font-family",
    "font-size",
    "font-size",
    "font-weight",
    "font-style",
    "font-variant",
    "font-stretch",
    "line-height",
    "text-align",
    "text-indent",
    "text-transform",
    "letter-spacing",
    "word-spacing",
    "white-space",
    "white-space-collapse",
    "text-wrap",
    "text-wrap-mode",
    "overflow-wrap",
    "word-wrap",
    "word-break",
    "line-break",
    "hyphens",
    "list-style",
    "list-style-type",
    "list-style-position",
    "list-style-image",
    "border-collapse",
    "border-spacing",
    "caption-side",
    "table-layout",
    "empty-cells",
    "text-decoration",
    "text-decoration-line",
    "text-decoration-color",
    "vertical-align",
    "flex",
    "flex",
    "flex-direction",
    "flex-wrap",
    "flex-flow",
    "flex-grow",
    "flex-shrink",
    "flex-basis",
    "justify-content",
    "align-items",
    "align-self",
    "align-content",
    "place-content",
    "place-items",
    "justify-items",
    "justify-self",
    "order",
    "gap",
    "row-gap",
    "column-gap",
    "grid-template-columns",
    "grid-template-columns",
    "grid-template-rows",
    "grid-template",
    "grid-auto-columns",
    "grid-auto-rows",
    "grid-auto-flow",
    "grid-template-areas",
    "grid-column",
    "grid-row",
    "grid-area",
    "grid-column-start",
    "grid-row-end",
    "content",
    "content",
    "counter-reset",
    "counter-increment",
    "quotes",
    "transform",
    "columns",
    "column-count",
    "column-width",
    "writing-mode",
    "direction",
    "unicode-bidi",
    "box-shadow",
    "text-shadow",
    "filter",
    "object-fit",
    "object-position",
    "text-overflow",
    "user-select",
    "pointer-events",
    "resize",
    "isolation",
    "contain",
    "will-change",
    "mix-blend-mode",
    "tab-size",
    "caret-color",
    "accent-color",
    "all",
    "-webkit-box-flex",
    "zoom",
    "unknown-property",
];

impl Rng {
    fn pick_owned<'a>(&mut self, items: &'a [String]) -> &'a str {
        if items.is_empty() {
            return "";
        }
        &items[self.below(items.len())]
    }
}

// ---- HTML generation ----------------------------------------------------

/// Appends formatted text to a generator's output (the arguments may use
/// the generator's random numbers).
macro_rules! emit {
    ($gen:expr, $($arg:tt)*) => {{
        let text = format!($($arg)*);
        $gen.out.push_str(&text);
    }};
}

/// Escapes text for a double-quoted attribute value.
fn attr_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

struct HtmlGen<'a> {
    css: Gen<'a>,
    out: String,
    /// Elements still to be generated.
    budget: usize,
    max_depth: usize,
}

const LINKS: &[&str] = &[
    "#i1",
    "#i7",
    "#",
    "#nowhere",
    "page2.html",
    "?q=1&r=2",
    "javascript:void(0)",
    "mailto:a@b.c",
    "data:text/html,hi",
    "",
    "%zz",
    "http://127.0.0.1:1/x",
    "//",
    "../../up",
    "#%E6%97%A5",
];

impl<'a> HtmlGen<'a> {
    fn rng(&mut self) -> &mut Rng {
        self.css.rng
    }

    fn text(&mut self) {
        let n = self.rng().range(1, 12);
        for i in 0..n {
            if i > 0 {
                self.out.push(' ');
            }
            if self.rng().chance(2) {
                // A long word without break opportunities.
                let len = self.rng().range(50, 3000);
                let c = ["w", "m", "日", "-", "\u{ad}", "/"][self.rng().below(6)];
                self.out.push_str(&c.repeat(len));
            } else {
                let w = self.rng().pick(WORDS);
                self.out.push_str(w);
            }
        }
        if self.rng().chance(5) {
            let ws = ["\n\n   ", "\t", "\u{2028}", "\r\n", "\u{0}"][self.rng().below(5)];
            self.out.push_str(ws);
        }
    }

    /// Global attributes (class, id, style…) for an element.
    fn global_attrs(&mut self) {
        if self.rng().chance(55) {
            let n = self.rng().range(1, 3);
            let classes: Vec<String> = (0..n)
                .map(|_| format!("c{}", self.rng().below(CLASSES)))
                .collect();
            emit!(self, " class=\"{}\"", classes.join(" "));
        }
        if self.rng().chance(20) {
            let id = self.rng().below(IDS);
            emit!(self, " id=i{id}");
        }
        if self.rng().chance(25) {
            let n = self.rng().range(1, 5);
            let decls = self.css.declarations(n);
            emit!(self, " style=\"{}\"", attr_escape(&decls));
        }
        if self.rng().chance(5) {
            let v = ["0", "-1", "1", "5", "99999999999", "x"][self.rng().below(6)];
            emit!(self, " tabindex={v}");
        }
        if self.rng().chance(3) {
            let attr = [
                " hidden",
                " dir=rtl",
                " dir=auto",
                " lang=en",
                " title=\"tip\"",
                " align=center",
                " bgcolor=red",
                " data-x=\"1\"",
            ][self.rng().below(8)];
            self.out.push_str(attr);
        }
    }

    fn open(&mut self, tag: &str) {
        emit!(self, "<{tag}");
        self.global_attrs();
        self.out.push('>');
    }

    fn close(&mut self, tag: &str) {
        // Sometimes leave elements unclosed (implied end tags, error recovery).
        if !self.rng().chance(3) {
            emit!(self, "</{tag}>");
        }
    }

    fn children(&mut self, depth: usize) {
        let n = self.rng().range(0, 6);
        for _ in 0..n {
            if self.budget == 0 {
                break;
            }
            if self.rng().chance(35) {
                self.text();
            } else {
                self.element(depth + 1);
            }
        }
    }

    fn container(&mut self, tag: &str, depth: usize) {
        self.open(tag);
        self.children(depth);
        self.close(tag);
    }

    fn image_url(&mut self) -> String {
        let images = self.css.images;
        self.rng().pick_owned(images).to_string()
    }

    fn element(&mut self, depth: usize) {
        if self.budget == 0 {
            return;
        }
        self.budget -= 1;
        if depth >= self.max_depth {
            self.text();
            return;
        }
        match self.rng().below(40) {
            0..=5 => {
                let tag = self.rng().pick(&[
                    "div",
                    "div",
                    "div",
                    "section",
                    "article",
                    "p",
                    "blockquote",
                    "main",
                    "nav",
                    "aside",
                    "header",
                    "footer",
                    "figure",
                    "center",
                    "address",
                    "pre",
                    "h1",
                    "h2",
                    "h3",
                    "h6",
                    "figcaption",
                    "dialog",
                    "marquee",
                ]);
                self.container(tag, depth);
            }
            6..=10 => {
                let tag = self.rng().pick(&[
                    "span", "span", "b", "i", "em", "strong", "code", "small", "big", "sub", "sup",
                    "u", "s", "mark", "q", "abbr", "font", "nobr", "kbd", "bdi", "bdo", "ruby",
                    "rt", "var", "cite", "del", "ins", "label",
                ]);
                self.container(tag, depth);
            }
            11 | 12 => {
                let href = self.rng().pick(LINKS);
                emit!(self, "<a href=\"{}\"", attr_escape(href));
                if self.rng().chance(20) {
                    emit!(self, " name=n{}", self.rng().below(5));
                }
                self.global_attrs();
                self.out.push('>');
                self.children(depth);
                self.close("a");
            }
            13 | 14 => self.list(depth),
            15 | 16 => self.table(depth),
            17 => self.form(depth),
            18 | 19 => self.img(),
            20 => self.svg(),
            21 => self.control(),
            22 => self.floats(depth),
            23 => self.flex_or_grid(depth),
            24 => {
                let n = self.rng().range(1, 300);
                self.deep_chain(n);
            }
            25 => {
                let tag = self.rng().pick(&[
                    "br",
                    "hr",
                    "wbr",
                    "br",
                    "hr width=50% size=1e9 align=right",
                ]);
                emit!(self, "<{tag}>");
            }
            26 => {
                self.open("details");
                if self.rng().chance(50) {
                    self.out.push_str("<summary>sum</summary>");
                }
                self.children(depth);
                self.close("details");
            }
            27 => {
                self.open("fieldset");
                if self.rng().chance(70) {
                    self.out.push_str("<legend>legend ");
                    self.text();
                    self.out.push_str("</legend>");
                }
                self.children(depth);
                self.close("fieldset");
            }
            28 => {
                self.open("dl");
                for _ in 0..self.rng().range(1, 4) {
                    self.out.push_str("<dt>");
                    self.text();
                    self.out.push_str("<dd>");
                    self.children(depth + 1);
                }
                self.close("dl");
            }
            29 => {
                let tag = self.rng().pick(&[
                    "iframe src=page2.html",
                    "video",
                    "video poster=img/photo.png",
                    "canvas width=1e9 height=-4",
                    "object data=img/icon.svg type=image/svg+xml",
                    "embed src=img/logo.png",
                    "audio controls",
                    "noscript",
                ]);
                emit!(self, "<{tag}>");
                self.text();
                let name = tag.split(' ').next().unwrap_or("x").to_string();
                self.close(&name);
            }
            30 => {
                // A styled element with a <style> of its own (scoped nowhere: global).
                let n = self.rng().range(1, 4);
                let css = self.css.rules(n, 0);
                let media = [
                    "",
                    " media=print",
                    " media=\"(max-width: 600px)\"",
                    " media=\"screen and (min-width: 2000px)\"",
                ][self.rng().below(4)];
                emit!(self, "<style{media}>{css}</style>");
            }
            31 => {
                let tag = self.rng().pick(&[
                    "template",
                    "slot",
                    "math",
                    "xmp",
                    "plaintext-not",
                    "listing",
                    "noembed",
                    "textarea",
                    "title",
                ]);
                emit!(self, "<{tag}>");
                self.text();
                emit!(self, "</{tag}>");
            }
            32 => {
                self.out.push_str("<picture>");
                for _ in 0..self.rng().below(3) {
                    let srcset = self.srcset();
                    emit!(
                        self,
                        "<source srcset=\"{}\" media=\"{}\" type=\"{}\">",
                        attr_escape(&srcset),
                        attr_escape(&self.css.media_query()),
                        ["image/png", "image/webp", "image/avif", ""][self.rng().below(4)]
                    );
                }
                self.img();
                self.out.push_str("</picture>");
            }
            33 => {
                let p = self.rng().pick(&["progress", "meter"]);
                let v = self.rng().pick(&["0.5", "1e999", "-1", "NaN", "", "7"]);
                let m = self.rng().pick(&["1", "0", "-5", "1e-300", "abc", "100"]);
                emit!(self, "<{p} value=\"{v}\" max=\"{m}\" min=\"{m}\"></{p}>");
            }
            34 | 35 => self.odd_element(depth),
            _ => {
                let tag = self
                    .rng()
                    .pick(&["div", "p", "span", "li", "td", "section"]);
                self.container(tag, depth);
            }
        }
    }

    /// An element whose kind and formatting disagree: replaced elements,
    /// controls and table parts with a random display, position, float or
    /// overflow, around children of their own.
    fn odd_element(&mut self, depth: usize) {
        let tag = self.rng().pick(&[
            "img", "input", "select", "textarea", "button", "table", "tr", "td", "thead",
            "caption", "col", "li", "fieldset", "legend", "br", "hr", "svg", "video", "iframe",
            "summary", "details", "progress", "option", "ul", "body", "html", "head", "title",
        ]);
        let mut style = String::new();
        for property in [
            "display",
            "position",
            "float",
            "overflow",
            "width",
            "height",
            "content",
            "list-style",
        ] {
            if self.rng().chance(40) {
                let value = self.css.value(property);
                let _ = write!(style, "{property}: {value}; ");
            }
        }
        emit!(self, "<{tag} style=\"{}\"", attr_escape(&style));
        self.global_attrs();
        self.out.push('>');
        self.children(depth);
        if self.rng().chance(80) {
            emit!(self, "</{tag}>");
        }
    }

    fn list(&mut self, depth: usize) {
        let tag = if self.rng().chance(50) { "ul" } else { "ol" };
        emit!(self, "<{tag}");
        if tag == "ol" && self.rng().chance(50) {
            let start =
                self.rng()
                    .pick(&["0", "-5", "2147483647", "-2147483648", "99999999999", "x"]);
            emit!(self, " start={start}");
            if self.rng().chance(30) {
                self.out.push_str(" reversed");
            }
            if self.rng().chance(30) {
                emit!(
                    self,
                    " type={}",
                    self.rng().pick(&["a", "A", "i", "I", "1", "?"])
                );
            }
        }
        self.global_attrs();
        self.out.push('>');
        for _ in 0..self.rng().range(1, 6) {
            if self.budget == 0 {
                break;
            }
            self.budget -= 1;
            self.out.push_str("<li");
            if self.rng().chance(15) {
                emit!(
                    self,
                    " value={}",
                    self.rng().pick(&["3", "-1", "1e3", "2147483648"])
                );
            }
            self.global_attrs();
            self.out.push('>');
            self.children(depth + 1);
        }
        self.close(tag);
    }

    fn table(&mut self, depth: usize) {
        self.out.push_str("<table");
        if self.rng().chance(40) {
            emit!(
                self,
                " border={} cellpadding={} cellspacing={} width={}",
                self.rng().pick(&["1", "0", "5", "-3", "1e9"]),
                self.rng().pick(&["2", "0", "100", "-1"]),
                self.rng().pick(&["0", "3", "1000", "x"]),
                self.rng().pick(&["100%", "500", "0", "-1", "99999", "50%"]),
            );
        }
        self.global_attrs();
        self.out.push('>');
        if self.rng().chance(20) {
            self.out.push_str("<caption>caption ");
            self.text();
            self.out.push_str("</caption>");
        }
        if self.rng().chance(15) {
            emit!(
                self,
                "<colgroup><col span={}><col width=40></colgroup>",
                self.rng().pick(&["2", "0", "1000", "-1"])
            );
        }
        let rows = self.rng().range(1, 6);
        let cols = self.rng().range(1, 6);
        for r in 0..rows {
            if r == 0 && self.rng().chance(30) {
                self.out.push_str("<thead>");
            } else if r == 1 && self.rng().chance(30) {
                self.out.push_str("<tbody>");
            } else if r == rows - 1 && self.rng().chance(20) {
                self.out.push_str("<tfoot>");
            }
            self.out.push_str("<tr>");
            for _ in 0..cols {
                if self.budget == 0 {
                    break;
                }
                self.budget -= 1;
                let cell = if self.rng().chance(20) { "th" } else { "td" };
                emit!(self, "<{cell}");
                if self.rng().chance(20) {
                    let span = self
                        .rng()
                        .pick(&["2", "3", "0", "1000", "-1", "65535", "x", "2"]);
                    emit!(self, " colspan={span}");
                }
                if self.rng().chance(20) {
                    let span = self.rng().pick(&["2", "3", "0", "70000", "-1", "x", "2"]);
                    emit!(self, " rowspan={span}");
                }
                if self.rng().chance(10) {
                    emit!(
                        self,
                        " width={} height={} nowrap valign=top",
                        self.rng().pick(&["10", "50%", "0", "1e9"]),
                        self.rng().pick(&["10", "-5", "1e9"])
                    );
                }
                self.global_attrs();
                self.out.push('>');
                if self.rng().chance(70) {
                    self.text();
                } else {
                    self.children(depth + 1);
                }
                if self.rng().chance(70) {
                    emit!(self, "</{cell}>");
                }
            }
        }
        self.close("table");
    }

    fn srcset(&mut self) -> String {
        match self.rng().below(6) {
            0 => ",, 2x,".to_string(),
            1 => format!("{} 1e309x, {} 0w", self.image_url(), self.image_url()),
            2 => format!("{} 100w, {} 2000w", self.image_url(), self.image_url()),
            3 => format!(
                "{} 1x, {} 2x, {} -1x",
                self.image_url(),
                self.image_url(),
                self.image_url()
            ),
            4 => "a.png 1.5x, a.png 1.5x, (((".to_string(),
            _ => self.image_url(),
        }
    }

    fn img(&mut self) {
        let src = self.image_url();
        emit!(self, "<img src=\"{}\"", attr_escape(&src));
        if self.rng().chance(30) {
            emit!(self, " srcset=\"{}\"", attr_escape(&self.srcset()));
        }
        if self.rng().chance(20) {
            emit!(
                self,
                " sizes=\"{}\"",
                self.rng().pick(&[
                    "(max-width: 100px) 50vw, 100px",
                    "calc(1e38px)",
                    "garbage",
                    "0",
                    "-10vw"
                ])
            );
        }
        if self.rng().chance(40) {
            let w = self.rng().pick(&[
                "10", "100", "0", "-5", "50%", "1e9", "99999", "abc", "3.5", "200",
            ]);
            let h = self
                .rng()
                .pick(&["10", "100", "0", "-5", "50%", "1e9", "99999", "abc", "40"]);
            emit!(self, " width=\"{w}\" height=\"{h}\"");
        }
        if self.rng().chance(40) {
            emit!(
                self,
                " alt=\"{}\"",
                self.rng().pick(&[
                    "alt text",
                    "",
                    "a very long alternative text that wraps",
                    "日本"
                ])
            );
        }
        if self.rng().chance(10) {
            self.out.push_str(" usemap=#m><map name=m><area shape=rect coords=\"0,0,10,10\" href=page2.html><area shape=poly coords=\"1,2,3\" href=x");
            self.out.push_str("></map");
        }
        self.global_attrs();
        self.out.push('>');
    }

    fn svg(&mut self) {
        let w = self
            .rng()
            .pick(&["100", "10", "0", "-1", "1e9", "50%", "", "3000", "1e38"]);
        let h = self
            .rng()
            .pick(&["100", "10", "0", "-1", "1e9", "50%", "", "2000"]);
        let vb = self.rng().pick(&[
            "0 0 10 10",
            "0 0 0 0",
            "-1e9 -1e9 2e9 2e9",
            "garbage",
            "0 0 1 1e-30",
            "",
        ]);
        emit!(self, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"{vb}\"");
        self.global_attrs();
        self.out.push('>');
        for _ in 0..self.rng().range(0, 6) {
            let shape = match self.rng().below(10) {
                0 => format!("<rect x=\"{}\" y=\"1\" width=\"{}\" height=\"8\" fill=\"{}\" rx=\"2\"/>", self.rng().pick(&["0", "-5", "1e30"]), self.rng().pick(&["5", "1e9", "-3", "NaN"]), self.css.color()),
                1 => "<circle cx=\"5\" cy=\"5\" r=\"4\" stroke=\"blue\" stroke-width=\"1e6\"/>".to_string(),
                2 => format!("<path d=\"{}\"/>", self.rng().pick(&["M0 0 L10 10 Z", "M 0 0 C 1e30 1e30 -1e30 2 5 5", "garbage", "M0,0 A 5 5 0 1 1 10 10", "M"])),
                3 => format!("<text x=\"1\" y=\"8\" font-size=\"{}\">{}</text>", self.rng().pick(&["8", "1e6", "0", "-4"]), self.rng().pick(WORDS)),
                4 => "<g transform=\"scale(1e9)\"><rect width=\"1\" height=\"1\"/></g>".to_string(),
                5 => "<linearGradient id=\"g\"><stop offset=\"0\" stop-color=\"red\"/><stop offset=\"2\" stop-color=\"blue\"/></linearGradient><rect width=\"10\" height=\"10\" fill=\"url(#g)\"/>".to_string(),
                6 => "<use href=\"#g\"/><use href=\"#self\" id=\"self\"/>".to_string(),
                7 => "<foreignObject width=\"10\" height=\"10\"><p>html <b>inside</b></p></foreignObject>".to_string(),
                8 => format!("<image href=\"{}\" width=\"10\" height=\"10\"/>", attr_escape(&self.image_url())),
                _ => "<title>svg title</title><style>rect { fill: green }</style>".to_string(),
            };
            self.out.push_str(&shape);
        }
        self.out.push_str("</svg>");
    }

    fn control(&mut self) {
        match self.rng().below(5) {
            0 => {
                let n = self.rng().below(3);
                emit!(
                    self,
                    "<select name=s{n}{}",
                    self.rng().pick(&[
                        "",
                        " multiple",
                        " size=5",
                        " size=0",
                        " size=99999",
                        " disabled"
                    ])
                );
                self.global_attrs();
                self.out.push('>');
                for i in 0..self.rng().range(0, 6) {
                    if i == 2 && self.rng().chance(40) {
                        self.out.push_str("<optgroup label=g>");
                    }
                    let selected = if self.rng().chance(20) {
                        " selected"
                    } else {
                        ""
                    };
                    emit!(self, "<option value=\"o{i}\"{selected}>");
                    self.text();
                }
                self.out.push_str("</select>");
            }
            1 => {
                emit!(
                    self,
                    "<textarea name=t rows=\"{}\" cols=\"{}\"",
                    self.rng().pick(&["2", "0", "-1", "100000", "x"]),
                    self.rng().pick(&["20", "0", "1e9", "99999"])
                );
                if self.rng().chance(30) {
                    self.out.push_str(" placeholder=\"place holder\"");
                }
                self.global_attrs();
                self.out.push('>');
                if self.rng().chance(70) {
                    self.text();
                }
                self.out.push_str("</textarea>");
            }
            2 => {
                let kind = self.rng().pick(&["submit", "reset", "button", "menu"]);
                emit!(self, "<button type={kind} name=b value=v");
                self.global_attrs();
                self.out.push('>');
                self.text();
                self.out.push_str("</button>");
            }
            _ => self.input(),
        }
    }

    fn input(&mut self) {
        let ty = self.rng().pick(&[
            "text",
            "text",
            "password",
            "checkbox",
            "checkbox",
            "radio",
            "radio",
            "submit",
            "reset",
            "button",
            "image",
            "hidden",
            "number",
            "range",
            "color",
            "date",
            "email",
            "file",
            "search",
            "tel",
            "url",
            "datetime-local",
            "month",
            "week",
            "time",
            "bogus",
        ]);
        emit!(
            self,
            "<input type={ty} name={}",
            ["q", "c", "r", "x y", "日本", ""][self.rng().below(6)]
        );
        if self.rng().chance(50) {
            let v = match self.rng().below(5) {
                0 => "x".repeat(self.rng().range(100, 2000)),
                1 => "a&b=c d%".to_string(),
                2 => "#ff0000".to_string(),
                3 => "50".to_string(),
                _ => self.rng().pick(WORDS).to_string(),
            };
            emit!(self, " value=\"{}\"", attr_escape(&v));
        }
        for attr in [
            " checked",
            " disabled",
            " readonly",
            " placeholder=\"type here\"",
            " required",
            " autofocus",
        ] {
            if self.rng().chance(15) {
                self.out.push_str(attr);
            }
        }
        if self.rng().chance(15) {
            emit!(
                self,
                " size={} maxlength={} min={} max={} step={}",
                self.rng().pick(&["1", "0", "99999", "-4"]),
                self.rng().pick(&["0", "3", "-1"]),
                self.rng().pick(&["0", "-1e9", "x"]),
                self.rng().pick(&["100", "1e999", "-5"]),
                self.rng().pick(&["any", "0", "-1", "1e-300"])
            );
        }
        if ty == "image" {
            emit!(self, " src=\"{}\"", attr_escape(&self.image_url()));
        }
        self.global_attrs();
        self.out.push('>');
    }

    fn form(&mut self, depth: usize) {
        emit!(
            self,
            "<form action=\"{}\" method={} enctype=\"{}\"",
            attr_escape(self.rng().pick(&[
                "page2.html",
                "?a=1#f",
                "",
                "javascript:x",
                "http://127.0.0.1:1/",
                "%zz"
            ])),
            self.rng().pick(&["get", "post", "POST", "dialog", "put"]),
            self.rng().pick(&["", "multipart/form-data", "text/plain"]),
        );
        if self.rng().chance(10) {
            self.out.push_str(" accept-charset=\"iso-8859-1\"");
        }
        self.global_attrs();
        self.out.push('>');
        for _ in 0..self.rng().range(1, 6) {
            if self.budget == 0 {
                break;
            }
            self.budget -= 1;
            match self.rng().below(4) {
                0 => {
                    emit!(self, "<label for=i{}>label ", self.rng().below(IDS));
                    self.text();
                    self.out.push_str("</label>");
                }
                1 => {
                    self.out.push_str("<label>");
                    self.input();
                    self.text();
                    self.out.push_str("</label>");
                }
                2 => self.control(),
                _ => self.element(depth + 1),
            }
        }
        self.close("form");
    }

    fn floats(&mut self, depth: usize) {
        let n = if self.rng().chance(20) {
            self.rng().range(50, 200)
        } else {
            self.rng().range(1, 8)
        };
        self.open("div");
        for i in 0..n {
            if self.budget == 0 {
                break;
            }
            self.budget = self.budget.saturating_sub(if i % 4 == 0 { 1 } else { 0 });
            let side = self.rng().pick(&["left", "right", "left", "none"]);
            let w = self.rng().range(1, 300);
            let h = self.rng().range(1, 120);
            let clear = if self.rng().chance(10) {
                "clear: both;"
            } else {
                ""
            };
            emit!(
                self,
                "<div style=\"float: {side}; width: {w}px; height: {h}px; {clear}\""
            );
            self.global_attrs();
            self.out.push('>');
            if self.rng().chance(30) {
                self.text();
            }
            self.out.push_str("</div>");
            if self.rng().chance(20) {
                self.text();
            }
        }
        if self.rng().chance(50) {
            self.children(depth);
        }
        self.close("div");
    }

    fn flex_or_grid(&mut self, depth: usize) {
        let display = self.rng().pick(&[
            "flex",
            "inline-flex",
            "grid",
            "inline-grid",
            "table",
            "table-row",
            "flex",
            "grid",
        ]);
        let mut style = format!("display: {display};");
        if display.contains("flex") {
            let _ = write!(
                style,
                " flex-flow: {}; gap: {};",
                self.css.value("flex-flow"),
                self.css.length()
            );
        } else if display.contains("grid") {
            let _ = write!(
                style,
                " grid-template-columns: {}; grid-template-rows: {};",
                self.css.grid_tracks(),
                self.css.grid_tracks()
            );
        }
        emit!(self, "<div style=\"{}\"", attr_escape(&style));
        self.global_attrs();
        self.out.push('>');
        let n = self.rng().range(1, 12);
        for _ in 0..n {
            if self.budget == 0 {
                break;
            }
            let item_style = format!(
                "{}: {}; {}: {}",
                [
                    "flex",
                    "grid-column",
                    "grid-row",
                    "order",
                    "align-self",
                    "width",
                    "display"
                ][self.rng().below(7)],
                {
                    let p = [
                        "flex",
                        "grid-column",
                        "grid-row",
                        "order",
                        "align-self",
                        "width",
                        "display",
                    ][self.rng().below(7)];
                    self.css.value(p)
                },
                ["margin", "min-width", "flex-basis", "height", "position"][self.rng().below(5)],
                {
                    let p = ["margin", "min-width", "flex-basis", "height", "position"]
                        [self.rng().below(5)];
                    self.css.value(p)
                },
            );
            emit!(self, "<div style=\"{}\">", attr_escape(&item_style));
            if self.rng().chance(50) {
                self.text();
            } else {
                self.element(depth + 1);
            }
            self.out.push_str("</div>");
        }
        self.close("div");
    }

    /// `n` nested elements around some text (beyond the tree builder's depth limit).
    fn deep_chain(&mut self, n: usize) {
        let mut stack = Vec::new();
        for _ in 0..n {
            let tag = self.rng().pick(&[
                "div",
                "span",
                "b",
                "a",
                "li",
                "table",
                "td",
                "p",
                "section",
                "em",
                "ul",
                "div style=\"display:flex\"",
                "div style=\"float:left\"",
                "div style=\"display:grid\"",
                "span style=\"position:relative\"",
                "div class=c1",
                "button",
                "label",
                "font",
                "div style=\"display:table-cell\"",
            ]);
            emit!(self, "<{tag}>");
            stack.push(tag.split(' ').next().unwrap_or("div").to_string());
        }
        self.text();
        while let Some(tag) = stack.pop() {
            if !self.rng().chance(5) {
                emit!(self, "</{tag}>");
            }
        }
    }
}

/// Everything about one fuzz case except its HTML.
#[derive(Clone, Debug)]
struct Plan {
    viewport: Size,
    resized: Size,
    ops: Vec<Op>,
    renders: Vec<Render>,
}

/// A point as fractions of the document size (so that the plan does not
/// depend on the document).
#[derive(Copy, Clone, Debug)]
struct Frac(f32, f32);

#[derive(Clone, Debug)]
enum Op {
    HitTest(Frac, Frac),
    Click(Frac, Frac),
    Hover(Option<(Frac, Frac)>),
    Find(&'static str),
    FocusNext(bool),
    Activate,
    Type(&'static str),
    /// Types (or searches) a long run of a repeated piece.
    TypeLong(&'static str, usize),
    FindLong(&'static str, usize),
    Backspace,
    Submit,
    Blur,
    Fragment(&'static str),
    Resize(Size),
}

#[derive(Clone, Debug)]
struct Render {
    width: u32,
    height: u32,
    scroll: Frac,
    scale: f32,
    offset: (i32, i32),
    height_limit: Option<u32>,
}

fn random_viewport(rng: &mut Rng) -> Size {
    let width = match rng.below(10) {
        0 => rng.range(1, 60) as f32,
        1 | 2 => rng.range(1600, 4000) as f32,
        _ => rng.range(200, 1600) as f32,
    };
    let height = match rng.below(8) {
        0 => rng.range(1, 60) as f32,
        _ => rng.range(100, 2400) as f32,
    };
    Size::new(width, height)
}

fn random_frac(rng: &mut Rng) -> Frac {
    Frac(rng.f32(-0.05, 1.05), rng.f32(-0.05, 1.05))
}

fn random_plan(rng: &mut Rng, ops: usize) -> Plan {
    let viewport = random_viewport(rng);
    let resized = random_viewport(rng);
    let ops = (0..ops)
        .map(|_| match rng.below(16) {
            0 | 1 => Op::HitTest(random_frac(rng), random_frac(rng)),
            2 | 3 => Op::Click(random_frac(rng), random_frac(rng)),
            4 => Op::Hover(None),
            5 | 6 => Op::Hover(Some((random_frac(rng), random_frac(rng)))),
            7 => Op::Find(rng.pick(&[
                "lorem", "a", "日本", "x y", "\u{ad}", "", "wwwwwwww", "🙂", "LOREM",
            ])),
            8 | 9 => Op::FocusNext(rng.chance(30)),
            10 => Op::Activate,
            11 if rng.chance(10) => {
                let piece = rng.pick(&["w", "a b ", "日", "\n", "🙂"]);
                let n = rng.range(100, 5000);
                if rng.chance(50) {
                    Op::TypeLong(piece, n)
                } else {
                    Op::FindLong(piece, n)
                }
            }
            11 => Op::Type(rng.pick(&["abc", "\n", "日本語 🙂", "\u{7}\u{0}", "x"])),
            12 => Op::Backspace,
            13 => {
                if rng.chance(50) {
                    Op::Submit
                } else {
                    Op::Blur
                }
            }
            14 => Op::Fragment(rng.pick(&["i1", "n0", "", "%zz", "i7", "top"])),
            _ if rng.chance(10) => {
                // Hostile sizes (the engine clamps them).
                let side = |rng: &mut Rng| {
                    *rng.pick(&[&0.0f32, &-5.0, &f32::NAN, &f32::INFINITY, &1e6, &1e9, &0.4])
                };
                Op::Resize(Size::new(side(rng), side(rng)))
            }
            _ => Op::Resize(random_viewport(rng)),
        })
        .collect();
    let renders = (0..2)
        .map(|_| Render {
            width: rng.range(1, 160) as u32,
            height: rng.range(1, 120) as u32,
            scroll: Frac(rng.f32(-0.1, 1.1), rng.f32(-0.1, 1.1)),
            scale: match rng.below(40) {
                0 => *rng.pick(&[&0.0f32, &-1.0, &f32::NAN, &f32::INFINITY, &64.0, &1e-3]),
                1..=8 => [1.0, 2.0][rng.below(2)],
                _ => rng.f32(0.5, 3.0),
            },
            offset: (rng.range(0, 20) as i32 - 5, rng.range(0, 20) as i32 - 5),
            height_limit: if rng.chance(20) {
                Some(rng.range(0, 200) as u32)
            } else {
                None
            },
        })
        .collect();
    Plan {
        viewport,
        resized,
        ops,
        renders,
    }
}

/// A random document (HTML with inline and linked CSS).
fn random_document(rng: &mut Rng, images: &[String], budget: usize) -> String {
    let mut gen = HtmlGen {
        css: Gen {
            rng,
            images,
            extreme: 0,
        },
        out: String::new(),
        budget,
        max_depth: 14,
    };
    let rng = &mut *gen.css.rng;
    gen.css.extreme = [0, 0, 0, 5, 20, 50][rng.below(6)];
    let doctype = rng.chance(85);
    let base = rng.chance(5);
    let link = rng.chance(30);
    let refresh = rng.chance(5);
    let rules = rng.range(0, 40);
    if doctype {
        gen.out.push_str("<!DOCTYPE html>");
    }
    gen.out
        .push_str("<html><head><meta charset=utf-8><title>Fuzz ");
    gen.text();
    gen.out.push_str("</title>");
    if base {
        gen.out.push_str("<base href=\"sub/\">");
    }
    if refresh {
        let content = gen.rng().pick(&[
            "0",
            "5; url=next.html",
            "-1;url=",
            "1e309; url=x",
            "abc",
            "0;URL='a b'",
            "0; url=javascript:x",
        ]);
        emit!(gen, "<meta http-equiv=refresh content=\"{content}\">");
    }
    if link {
        let href = gen.rng().pick(&[
            "style.css",
            "style2.css",
            "missing.css",
            "data:text/css,p%7Bcolor:red%7D",
            "img/logo.png",
        ]);
        let media = gen
            .rng()
            .pick(&["", " media=print", " media=\"(min-width: 800px)\""]);
        emit!(gen, "<link rel=stylesheet href=\"{href}\"{media}>");
    }
    let css = gen.css.stylesheet(rules);
    emit!(gen, "<style>{css}</style></head>");
    gen.open("body");
    while gen.budget > 0 {
        let before = gen.budget;
        gen.element(1);
        if gen.rng().chance(30) {
            gen.text();
        }
        if gen.budget == before {
            break;
        }
    }
    gen.out.push_str("</body></html>");
    gen.out
}

/// Byte-level damage: deleted, duplicated and inserted bytes (markup
/// characters mostly), as truncated or corrupted downloads would produce.
fn mutate(rng: &mut Rng, html: &str) -> String {
    let mut bytes = html.as_bytes().to_vec();
    for _ in 0..rng.range(1, 20) {
        if bytes.is_empty() {
            break;
        }
        let at = rng.below(bytes.len());
        match rng.below(5) {
            0 => {
                let end = (at + rng.range(1, 200)).min(bytes.len());
                bytes.drain(at..end);
            }
            1 => {
                let end = (at + rng.range(1, 400)).min(bytes.len());
                let chunk = bytes[at..end].to_vec();
                let to = rng.below(bytes.len());
                bytes.splice(to..to, chunk);
            }
            2 => bytes.truncate(at),
            _ => {
                let insert: &[u8] = rng.pick(&[
                    &b"<"[..],
                    b">",
                    b"\"",
                    b"'",
                    b"&",
                    b"</",
                    b"/>",
                    b"<!--",
                    b"<![CDATA[",
                    b"\0",
                    b"<table>",
                    b"<svg>",
                    b"</p>",
                    b"<select>",
                    b"\xff\xfe",
                    b"\xc3",
                    b";}",
                    b"{",
                ]);
                bytes.splice(at..at, insert.iter().copied());
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

// ---- running a case -----------------------------------------------------

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut pixmap = Pixmap::new(width, height).expect("pixmap");
    pixmap.fill(xbro_paint::tiny_skia::Color::from_rgba8(200, 30, 90, 160));
    pixmap.encode_png().expect("png")
}

/// The temporary site the documents are loaded from, and the image URLs
/// they use.
struct Site {
    dir: PathBuf,
    images: Vec<String>,
}

impl Site {
    fn create() -> Site {
        let dir = std::env::temp_dir().join(format!("xbro-fuzz-{}", std::process::id()));
        let img = dir.join("img");
        std::fs::create_dir_all(&img).expect("temporary directory");
        let mut images: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(fixtures_dir().join("img")).expect("fixture images") {
            let path = entry.expect("entry").path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let bytes = std::fs::read(&path).expect("image");
            std::fs::write(img.join(&name), &bytes).expect("copy image");
            let mime = match path.extension().and_then(|e| e.to_str()) {
                Some("png") => "image/png",
                Some("jpg") => "image/jpeg",
                Some("gif") => "image/gif",
                Some("svg") => "image/svg+xml",
                _ => "application/octet-stream",
            };
            images.push(format!("img/{name}"));
            images.push(format!("data:{mime};base64,{}", base64(&bytes)));
        }
        for (w, h) in [(1, 1), (3, 2), (200, 1), (1, 300), (64, 64)] {
            images.push(format!("data:image/png;base64,{}", base64(&png(w, h))));
        }
        std::fs::write(dir.join("tiny.png"), png(2, 2)).expect("write png");
        // Deeply nested SVG (as an image: not limited by the HTML tree builder).
        let deep_svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20'>{}<rect width='5' height='5'/>{}</svg>",
            "<g transform='translate(0.01)'>".repeat(5000),
            "</g>".repeat(5000)
        );
        std::fs::write(dir.join("deep.svg"), deep_svg).expect("write svg");
        images.push("deep.svg".to_string());
        images.extend(
            [
                "tiny.png",
                "missing.png",
                "../outside.png",
                "/abs.png",
                "style.css",
                "data:image/png;base64,iVBORw0KGgo=",
                "data:image/gif;base64,R0lGODlh",
                "data:image/gif;base64,R0lGODlhAQABAAAAACw=",
                "data:,",
                "data:image/png;base64,!!!",
                "data:text/plain,hello",
                "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='20' height='10'%3E%3Crect width='20' height='10' fill='teal'/%3E%3C/svg%3E",
                "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 1 1'><circle r='1'/></svg>",
                "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='1e9' height='1e9'/>",
                "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='0' height='-5'/>",
                "data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=",
                "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='40' height='40'><filter id='f'><feGaussianBlur stdDeviation='1e6'/></filter><rect width='40' height='40' filter='url(%23f)'/></svg>",
                "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' width='100000' height='100000'><rect width='1e9' height='1e9' fill='red'/></svg>",
                "data:image/svg+xml,<svg",
                "http://127.0.0.1:1/remote.png",
                "",
            ]
            .map(str::to_string),
        );
        // Stylesheets that import each other (and themselves).
        let mut rng = Rng::new(0x0057_17e5);
        let mut gen = Gen {
            rng: &mut rng,
            images: &images,
            extreme: 5,
        };
        let style = format!(
            "@import 'style2.css'; @import url(style.css) screen;\n{}",
            gen.stylesheet(40)
        );
        let style2 = format!(
            "@import \"style.css\"; @import 'img/../style2.css';\n{}",
            gen.stylesheet(25)
        );
        std::fs::write(dir.join("style.css"), style).expect("write css");
        std::fs::write(dir.join("style2.css"), style2).expect("write css");
        Site { dir, images }
    }

    fn page_url(&self) -> Url {
        Url::from_file_path(self.dir.join("page.html")).expect("file URL")
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        if !std::thread::panicking() && std::env::var_os("XBRO_FUZZ_KEEP").is_none() {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

fn point(f: Frac, size: Size) -> Point {
    Point::new(
        f.0 * size.width.min(20_000.0),
        f.1 * size.height.min(50_000.0),
    )
}

fn scroll_point(f: Frac, page: &Page) -> Point {
    let size = page.document_size();
    let vp = page.viewport();
    Point::new(
        f.0 * (size.width - vp.width).max(0.0),
        f.1 * (size.height - vp.height).max(0.0),
    )
}

/// Problems found while running one case.
fn check_page(page: &Page, stage: &str, problems: &mut Vec<String>) {
    let size = page.document_size();
    if !(size.width.is_finite()
        && size.height.is_finite()
        && size.width >= 0.0
        && size.height >= 0.0)
    {
        problems.push(format!("{stage}: bad document size {size:?}"));
    }
    for message in page.console() {
        if message.contains("Internal error") {
            problems.push(format!("{stage}: console: {message}"));
        }
    }
    if let Some(root) = &page.layout_tree().root {
        let mut bad = None;
        visit_boxes(root, &mut |b| {
            if bad.is_some() {
                return;
            }
            let lines = b.lines.iter().flat_map(|line| {
                std::iter::once(line.rect).chain(line.fragments.iter().map(|f| match f {
                    Fragment::Text(t) => t.rect,
                    Fragment::InlineBox(i) => i.rect,
                    Fragment::Atomic(a) => a.content,
                }))
            });
            for rect in [b.content, b.margin_box()].into_iter().chain(lines) {
                if !finite_rect(rect) {
                    bad = Some(format!(
                        "{stage}: non-finite rect {rect:?} in the box of {:?}",
                        b.node
                    ));
                    return;
                }
            }
        });
        problems.extend(bad);
    }
}

fn finite_rect(r: Rect) -> bool {
    r.x.is_finite() && r.y.is_finite() && r.width.is_finite() && r.height.is_finite()
}

fn visit_boxes(b: &LayoutBox, f: &mut impl FnMut(&LayoutBox)) {
    f(b);
    for child in &b.children {
        visit_boxes(child, f);
    }
    for line in &b.lines {
        for fragment in &line.fragments {
            if let Fragment::Atomic(atomic) = fragment {
                visit_boxes(atomic, f);
            }
        }
    }
}

/// Records a problem when `rect` has a non-finite coordinate.
fn check_rect(what: &str, rect: Option<Rect>, problems: &mut Vec<String>) {
    if let Some(r) = rect.filter(|r| !finite_rect(*r)) {
        problems.push(format!("{what}: non-finite rect {r:?}"));
    }
}

/// Loads and exercises one document. Returns the problems found.
fn exercise(html: &str, url: Url, plan: &Plan) -> Vec<String> {
    let verbose = std::env::var_os("XBRO_FUZZ_VERBOSE").is_some();
    let mut clock = Instant::now();
    let mut lap = |what: &dyn std::fmt::Debug| {
        if verbose {
            eprintln!(
                "  {:>8.1} ms  {what:?}",
                clock.elapsed().as_secs_f64() * 1000.0
            );
        }
        clock = Instant::now();
    };
    let mut problems = Vec::new();
    let mut page = engine().load_html(html, url, plan.viewport);
    lap(&("load", plan.viewport, page.timings()));
    check_page(&page, "load", &mut problems);
    page.set_viewport(plan.resized);
    lap(&("resize", plan.resized));
    check_page(&page, "resize", &mut problems);
    for op in &plan.ops {
        let size = page.document_size();
        match *op {
            Op::HitTest(p, s) => {
                let (p, s) = (point(p, size), scroll_point(s, &page));
                let _ = (page.node_at(p, s), page.link_at(p, s), page.cursor_at(p, s));
            }
            Op::Click(p, s) => {
                let _ = page.click(point(p, size), scroll_point(s, &page));
            }
            Op::Hover(target) => {
                let target = target.map(|(p, s)| (point(p, size), scroll_point(s, &page)));
                let _ = page.set_hover(target);
            }
            Op::Find(query) => {
                for rect in page.find(query) {
                    check_rect("find", Some(rect), &mut problems);
                }
                let _ = page.find_matches(query);
            }
            Op::FocusNext(backwards) => {
                let rect = page.focus_next(backwards);
                check_rect("focus_next", rect, &mut problems);
            }
            Op::Activate => {
                let _ = page.activate_focused();
            }
            Op::Type(text) => {
                page.insert_text(text);
                check_rect("caret", page.caret_rect(), &mut problems);
            }
            Op::TypeLong(piece, n) => {
                page.insert_text(&piece.repeat(n));
                check_rect("caret", page.caret_rect(), &mut problems);
            }
            Op::FindLong(piece, n) => {
                let _ = page.find(&piece.repeat(n));
            }
            Op::Backspace => page.backspace(),
            Op::Submit => {
                let _ = page.submit_focused();
            }
            Op::Blur => {
                let _ = page.blur();
            }
            Op::Fragment(id) => {
                if let Some(y) = page.fragment_offset(id).filter(|y| !y.is_finite()) {
                    problems.push(format!("fragment_offset({id:?}) = {y}"));
                }
            }
            Op::Resize(viewport) => page.set_viewport(viewport),
        }
        lap(op);
    }
    check_page(&page, "interactions", &mut problems);
    let _ = (page.title(), page.meta_refresh());
    for rect in page.fixed_backdrops() {
        check_rect("fixed backdrop", Some(rect), &mut problems);
    }
    if format!("{:?}", page.display_list()).contains("NaN") {
        problems.push("NaN in the display list".to_string());
    }
    lap(&"checks");
    for r in &plan.renders {
        let Some(mut pixmap) = Pixmap::new(r.width, r.height) else {
            continue;
        };
        let options = RasterOptions {
            scroll: scroll_point(r.scroll, &page),
            scale: r.scale,
            offset: r.offset,
            height: r.height_limit,
        };
        page.render(&mut pixmap, &options);
        lap(&("render", r));
    }
    problems
}

/// A failed case.
struct Failure {
    index: usize,
    seed: u64,
    reasons: Vec<String>,
    html_path: PathBuf,
}

struct Outcome {
    elapsed: Duration,
    peak: usize,
}

/// Runs one case on a worker thread with a time limit.
fn run_case(
    index: usize,
    seed: u64,
    html: String,
    plan: Plan,
    url: Url,
    dir: &Path,
) -> Result<Outcome, Failure> {
    let _ = take_panics();
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let start = Instant::now();
    let (tx, rx) = mpsc::channel();
    let shared = Arc::new(html);
    let worker_html = Arc::clone(&shared);
    let spawned = std::thread::Builder::new()
        .name(format!("fuzz-doc-{index}"))
        .stack_size(WORKER_STACK)
        .spawn(move || {
            let problems = exercise(&worker_html, url, &plan);
            let _ = tx.send(problems);
        });
    let mut reasons = Vec::new();
    match spawned {
        Err(e) => reasons.push(format!("cannot spawn a worker: {e}")),
        Ok(handle) => match rx.recv_timeout(DOC_TIME_LIMIT) {
            Ok(problems) => {
                reasons.extend(problems);
                let _ = handle.join();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                reasons.push(format!("took more than {DOC_TIME_LIMIT:?}"));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                reasons.push("worker panicked".to_string());
                let _ = handle.join();
            }
        },
    }
    let elapsed = start.elapsed();
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
    if peak > MEMORY_LIMIT {
        reasons.push(format!("peak heap {} MiB", peak >> 20));
    }
    reasons.extend(take_panics());
    if reasons.is_empty() {
        return Ok(Outcome { elapsed, peak });
    }
    let html_path = dir.join(format!("failure-{index}.html"));
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&html_path, shared.as_bytes());
    Err(Failure {
        index,
        seed,
        reasons,
        html_path,
    })
}

fn env_number(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.trim().parse().ok()
}

/// Seed of document `index` for a run with `base` seed.
fn case_seed(base: u64, index: usize) -> u64 {
    base ^ (index as u64).wrapping_mul(0xA076_1D64_78BD_642F)
}

/// Runs `docs` documents; panics with a report when some fail.
fn fuzz(docs: usize, budget: (usize, usize), ops: usize) {
    install_panic_recorder();
    let site = Site::create();
    let base = env_number("XBRO_FUZZ_SEED").unwrap_or(0x00C0_FFEE);
    let docs = env_number("XBRO_FUZZ_DOCS").map_or(docs, |n| n as usize);
    let only = env_number("XBRO_FUZZ_ONLY").map(|n| n as usize);
    let dump = std::env::var_os("XBRO_FUZZ_DUMP").map(PathBuf::from);
    let replay = std::env::var_os("XBRO_FUZZ_REPLAY").map(PathBuf::from);
    let failures_dir = std::env::temp_dir().join("xbro-fuzz-failures");
    // Warm up fonts.
    let _ = engine().load_html("<p>warm up</p>", site.page_url(), Size::new(100.0, 100.0));
    let start = Instant::now();
    let mut failures = Vec::new();
    // (time, index) and (peak heap, index) of every passing document.
    let mut times = Vec::new();
    let mut peaks = Vec::new();
    let mut bytes = 0;
    for index in 0..docs {
        if only.is_some_and(|o| o != index) {
            continue;
        }
        let seed = case_seed(base, index);
        let mut rng = Rng::new(seed);
        let elements = rng.range(budget.0, budget.1);
        let mut html = random_document(&mut rng, &site.images, elements);
        if rng.chance(10) {
            html = mutate(&mut rng, &html);
        }
        let plan = random_plan(&mut rng, ops);
        if let Some(path) = &replay {
            html = std::fs::read_to_string(path).expect("replayed file");
        }
        bytes += html.len();
        if let Some(dir) = &dump {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(dir.join(format!("doc-{index}.html")), &html);
            let _ = std::fs::write(
                dir.join("current.txt"),
                format!("{index} seed {seed:#x}\n{plan:#?}\n"),
            );
        }
        match run_case(index, seed, html, plan, site.page_url(), &failures_dir) {
            Ok(outcome) => {
                times.push((outcome.elapsed, index));
                peaks.push((outcome.peak, index));
            }
            Err(failure) => {
                let timed_out = failure.reasons.iter().any(|r| r.starts_with("took more"));
                failures.push(failure);
                // A hung worker keeps running: do not pile up more of them.
                if timed_out || failures.len() >= 10 {
                    break;
                }
            }
        }
    }
    times.sort_unstable_by(|a, b| b.cmp(a));
    peaks.sort_unstable_by(|a, b| b.cmp(a));
    let top = |list: &[(String, usize)]| {
        list.iter()
            .take(3)
            .map(|(v, i)| format!("#{i} ({v})"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let times: Vec<(String, usize)> = times
        .iter()
        .map(|(t, i)| (format!("{:.0} ms", t.as_secs_f64() * 1000.0), *i))
        .collect();
    let peaks: Vec<(String, usize)> = peaks
        .iter()
        .map(|(p, i)| (format!("{} MiB", p >> 20), *i))
        .collect();
    println!(
        "{docs} documents ({} KiB of HTML) in {:.1} s; slowest: {}; largest peak heap: {}",
        bytes >> 10,
        start.elapsed().as_secs_f64(),
        top(&times),
        top(&peaks)
    );
    if !failures.is_empty() {
        let mut report = String::new();
        for f in &failures {
            let _ = writeln!(
                report,
                "document #{} (seed {:#x}, XBRO_FUZZ_ONLY={}), saved to {}:",
                f.index,
                f.seed,
                f.index,
                f.html_path.display()
            );
            for reason in f.reasons.iter().take(8) {
                let _ = writeln!(report, "    {reason}");
            }
        }
        panic!("{} failing document(s):\n{report}", failures.len());
    }
}

#[test]
fn fuzz_pipeline() {
    fuzz(300, (20, 350), 12);
}

#[test]
#[ignore = "long run: use --release"]
fn fuzz_pipeline_long() {
    fuzz(5000, (20, 400), 14);
}
