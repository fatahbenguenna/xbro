//! Pipeline benchmark on a synthetic "real-world-like" page of about
//! 1.5 MB: 30 encyclopedia articles (infoboxes, nested navigation tables,
//! floated thumbnails, references, flex card rows and grid galleries), a
//! 3 000-rule stylesheet with descendant and class selectors, and 300
//! images given as `data:` URIs.
//!
//! Run it with
//! `cargo test -p xbro-engine --release --test bench_realworld -- --ignored --nocapture`.
//! It prints the time of each stage (best of a few runs) and checks the
//! targets: the whole load under 1 s and the raster of one 1280×800
//! viewport under 16 ms. `XBRO_BENCH_SAVE=<file>` writes the page to a
//! file (to profile it with `xbro --screenshot`, for example).

mod common;

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use common::engine;
use xbro_engine::{LoadTimings, Page, Point, RasterOptions, Size, Url};
use xbro_paint::tiny_skia::{Color, Pixmap};

const ARTICLES: usize = 30;
const IMAGES: usize = 300;
const RULES: usize = 3000;
const VIEWPORT: Size = Size::new(1280.0, 800.0);

const WORDS: &[&str] = &[
    "the",
    "language",
    "of",
    "system",
    "and",
    "memory",
    "safety",
    "compiler",
    "in",
    "a",
    "performance",
    "is",
    "with",
    "type",
    "ownership",
    "borrow",
    "checker",
    "to",
    "concurrency",
    "programs",
    "development",
    "community",
    "release",
    "version",
    "for",
    "which",
    "was",
    "designed",
    "project",
    "standard",
    "library",
    "features",
    "syntax",
    "similar",
    "has",
    "been",
    "used",
    "by",
    "many",
    "companies",
    "including",
    "web",
    "browser",
    "engine",
    "kernel",
    "operating",
    "systems",
    "early",
    "history",
    "foundation",
];

/// Deterministic word stream.
struct Words(usize);

impl Words {
    fn next(&mut self) -> &'static str {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        WORDS[(self.0 >> 33) % WORDS.len()]
    }

    fn sentence(&mut self, out: &mut String, words: usize) {
        for i in 0..words {
            let w = self.next();
            if i == 0 {
                let mut c = w.chars();
                if let Some(first) = c.next() {
                    out.extend(first.to_uppercase());
                    out.push_str(c.as_str());
                }
            } else {
                out.push(' ');
                out.push_str(w);
            }
        }
        out.push_str(". ");
    }
}

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

/// A distinct small picture (flat colour bands with a diagonal stripe,
/// like a logo or a chart thumbnail), as a `data:` URI, with the size it
/// is displayed at (twice its pixel size for one image in two).
fn image(i: usize) -> (String, u32, u32) {
    let w = 24 + (i * 37 % 48) as u32;
    let h = 18 + (i * 53 % 36) as u32;
    let mut pixmap = Pixmap::new(w, h).expect("pixmap");
    let data = pixmap.data_mut();
    let base = [
        (i * 47 % 256) as u8,
        (i * 91 % 256) as u8,
        (i * 13 % 256) as u8,
    ];
    for y in 0..h {
        for x in 0..w {
            let k = ((y * w + x) * 4) as usize;
            let band = (y * 4 / h) as u8;
            let stripe = (x + y) % 16 < 3;
            data[k] = if stripe {
                255
            } else {
                base[0].wrapping_add(band * 40)
            };
            data[k + 1] = if stripe { 255 } else { base[1] };
            data[k + 2] = if stripe {
                255
            } else {
                base[2].wrapping_sub(band * 30)
            };
            data[k + 3] = 255;
        }
    }
    let png = pixmap.encode_png().expect("png");
    let scale = if i % 2 == 0 { 2 } else { 1 };
    (
        format!("data:image/png;base64,{}", base64(&png)),
        w * scale,
        h * scale,
    )
}

/// The 3 000-rule author stylesheet.
fn stylesheet() -> String {
    let mut css = String::from(
        ":root { --link: #36c; --border: #a2a9b1; --muted: #54595d }\n\
         body { margin: 0; font: 14px/1.6 sans-serif; color: #202122; background: #f8f9fa }\n\
         a { color: var(--link); text-decoration: none }\n\
         .header { display: flex; align-items: center; justify-content: space-between; padding: 8px 16px; background: #fff; border-bottom: 1px solid #eaecf0 }\n\
         .header nav { display: flex; gap: 12px }\n\
         .container { display: flex; gap: 24px; max-width: 1240px; margin: 0 auto; padding: 0 16px }\n\
         .sidebar { flex: 0 0 180px; font-size: 13px }\n\
         .content { flex: 1; min-width: 0; background: #fff; padding: 16px 24px; border: 1px solid #eaecf0 }\n\
         h1 { font: 28px/1.3 serif; border-bottom: 1px solid var(--border); margin: 0 0 4px }\n\
         h2 { font: 21px/1.3 serif; border-bottom: 1px solid var(--border); margin: 18px 0 6px }\n\
         .infobox { float: right; clear: right; width: 22em; margin: 0 0 12px 16px; border: 1px solid var(--border); background: #f8f9fa; font-size: 12px; border-spacing: 3px }\n\
         .infobox th { text-align: left; vertical-align: top; padding: 2px 4px }\n\
         .infobox td { padding: 2px 4px }\n\
         .thumb { float: right; clear: right; margin: 4px 0 8px 16px; padding: 3px; border: 1px solid #c8ccd1; background: #f8f9fa; font-size: 12px }\n\
         .thumb.left { float: left; margin: 4px 16px 8px 0 }\n\
         .thumbcaption { padding: 3px; line-height: 1.4 }\n\
         .toc { display: inline-block; border: 1px solid var(--border); background: #f8f9fa; padding: 7px; font-size: 13px }\n\
         .toc ul { list-style: none; margin: 0 0 0 1em; padding: 0 }\n\
         .wikitable { border-collapse: collapse; margin: 8px 0; background: #f8f9fa; border: 1px solid var(--border) }\n\
         .wikitable th, .wikitable td { border: 1px solid var(--border); padding: 3px 6px }\n\
         .wikitable th { background: #eaecf0; text-align: center }\n\
         pre { background: #f8f9fa; border: 1px solid #eaecf0; padding: 8px; white-space: pre-wrap; font: 13px monospace }\n\
         .cards { display: flex; flex-wrap: wrap; gap: 12px; margin: 12px 0 }\n\
         .card { flex: 1 1 200px; border: 1px solid #c8ccd1; border-radius: 4px; padding: 8px; background: #fff; box-sizing: border-box }\n\
         .card .title { font-weight: bold; margin-bottom: 4px }\n\
         .gallery { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 8px; margin: 12px 0 }\n\
         .gallerybox { border: 1px solid #eaecf0; background: #f8f9fa; text-align: center; padding: 4px }\n\
         .gallerybox img { max-width: 100%; height: auto }\n\
         .navbox { width: 100%; clear: both; border: 1px solid var(--border); margin-top: 12px; font-size: 12px; border-spacing: 0 }\n\
         .navbox-title { background: #ccf; text-align: center; font-weight: bold; padding: 2px }\n\
         .navbox-group { background: #ddf; text-align: right; padding: 2px 6px; white-space: nowrap; width: 1% }\n\
         .navbox-list { padding: 2px 4px }\n\
         .navbox-list ul { margin: 0; padding: 0; list-style: none }\n\
         .navbox-list li { display: inline; padding: 0 4px; border-left: 1px solid #aaa }\n\
         .references { font-size: 12px; columns: 2 }\n\
         sup.reference { font-size: 10px; line-height: 1 }\n\
         .hatnote { font-style: italic; padding-left: 1.6em; margin-bottom: 6px; color: var(--muted) }\n",
    );
    for i in 0..RULES {
        let n = i % 97;
        let rule = match i % 12 {
            0 => format!(
                ".mw-parser-output .cls-{n} a {{ color: #{:06x} }}\n",
                i * 2654435 % 0xffffff
            ),
            1 => format!(
                ".navbox .navbox-list li.item-{n} {{ padding: 0 {}px }}\n",
                3 + i % 4
            ),
            2 => format!(
                "#content .infobox th.label-{n} {{ font-weight: {} }}\n",
                if i % 2 == 0 { "bold" } else { "normal" }
            ),
            3 => format!(
                "div.card-{n} > .title {{ color: rgb({}, 40, 90) }}\n",
                i % 255
            ),
            4 => format!(
                ".article p.para-{} a.link-{n} {{ border-bottom: 1px dotted #{:03x} }}\n",
                i % 9,
                i % 4096
            ),
            5 => format!(
                "@media (max-width: {}px) {{ .sidebar .portal-{n} {{ display: none }} }}\n",
                600 + i % 300
            ),
            6 => format!(".unused-{i} .deep .chain div span {{ display: none }}\n"),
            7 => format!("ul.refs li.ref-{n} cite {{ font-style: normal }}\n"),
            8 => format!(
                ".gallery .gallerybox.gb-{n} {{ background: #f{:x}f{:x}f{:x} }}\n",
                i % 16,
                (i / 16) % 16,
                (i / 256) % 16
            ),
            9 => format!(
                "[data-section=\"s{n}\"] h2 {{ letter-spacing: 0.{}px }}\n",
                i % 10
            ),
            10 => format!(
                ".content .article .section-{n} .thumb img {{ border: 1px solid var(--border) }}\n"
            ),
            _ => format!("a.link-{n}:hover, a.link-{n}:focus {{ text-decoration: underline }}\n"),
        };
        css.push_str(&rule);
    }
    css
}

/// The whole page: about 1.5 MB of HTML.
fn page() -> String {
    // 10 images per article: the infobox, 4 thumbnails and a gallery of 5.
    let images: Vec<(String, u32, u32)> = (0..IMAGES).map(image).collect();
    let mut img = images.iter().cycle();
    let mut words = Words(7);
    let mut html = String::with_capacity(1_700_000);
    let _ = write!(
        html,
        "<!DOCTYPE html><html lang=en><head><meta charset=utf-8><title>Synthetic encyclopedia</title><style>{}</style></head><body>",
        stylesheet()
    );
    html.push_str("<header class=header><a class=logo href='/'>Encyclopedia</a><nav>");
    for i in 0..8 {
        let _ = write!(html, "<a href='/portal/{i}'>Portal {i}</a>");
    }
    html.push_str("</nav><form><input type=search name=q placeholder=Search><button>Go</button></form></header>");
    html.push_str("<div class=container><aside class=sidebar>");
    for p in 0..12 {
        let _ = write!(
            html,
            "<div class='portal portal-{p}'><h3>Section {p}</h3><ul>"
        );
        for k in 0..6 {
            let _ = write!(
                html,
                "<li><a href='/s/{p}/{k}'>{} {}</a></li>",
                words.next(),
                words.next()
            );
        }
        html.push_str("</ul></div>");
    }
    html.push_str("</aside><main id=content class=content>");
    for a in 0..ARTICLES {
        let _ = write!(html, "<article class=article data-article={a}><h1 id=a{a}>");
        words.sentence(&mut html, 4);
        html.push_str(
            "</h1><div class=hatnote>For other uses, see <a href='#'>disambiguation</a>.</div>",
        );
        // Infobox with a nested table.
        let (src, w, h) = img.next().expect("image");
        let _ = write!(
            html,
            "<table class=infobox><caption>Article {a}</caption><tr><td colspan=2><img src='{src}' width={w} height={h} alt=''></td></tr>"
        );
        for r in 0..10 {
            let _ = write!(
                html,
                "<tr><th class='label-{}'>{}</th><td>",
                (a + r) % 97,
                words.next()
            );
            if r == 4 {
                html.push_str("<table class=wikitable><tr><th>x</th><th>y</th></tr><tr><td>1</td><td>2</td></tr></table>");
            } else {
                words.sentence(&mut html, 5);
            }
            html.push_str("</td></tr>");
        }
        html.push_str("</table>");
        // Table of contents.
        html.push_str("<div class=toc><b>Contents</b><ul>");
        for s in 0..8 {
            let _ = write!(
                html,
                "<li><a href='#a{a}s{s}'>{} {}</a><ul><li><a href='#'>{}</a></li></ul></li>",
                s + 1,
                words.next(),
                words.next()
            );
        }
        html.push_str("</ul></div>");
        for s in 0..8 {
            let _ = write!(
                html,
                "<section class='section-{}' data-section=s{}><h2 id=a{a}s{s}>{} {}</h2>",
                (a * 8 + s) % 97,
                (a + s) % 97,
                words.next(),
                words.next()
            );
            if s % 2 == 0 {
                let (src, w, h) = img.next().expect("image");
                let side = if s % 4 == 0 { "" } else { " left" };
                let _ = write!(html, "<div class='thumb{side}'><img src='{src}' width={w} height={h} alt='thumbnail'><div class=thumbcaption>");
                words.sentence(&mut html, 8);
                html.push_str("</div></div>");
            }
            for p in 0..3 {
                let _ = write!(html, "<p class='para-{}'>", p % 9);
                for k in 0..5 {
                    words.sentence(&mut html, 10 + (k * 3) % 7);
                    match k {
                        1 => {
                            let _ = write!(
                                html,
                                "<a class='link-{}' href='/wiki/{}'>{}</a> ",
                                (a + s + p) % 97,
                                words.next(),
                                words.next()
                            );
                        }
                        2 => {
                            let _ =
                                write!(html, "<b>{}</b> <i>{}</i> ", words.next(), words.next());
                        }
                        3 => {
                            let _ = write!(
                                html,
                                "<sup class=reference><a href='#r{a}-{k}'>[{}]</a></sup> ",
                                k + p
                            );
                        }
                        _ => {}
                    }
                }
                html.push_str("</p>");
            }
            match s {
                2 => {
                    html.push_str("<table class='wikitable sortable'><tr><th>Version</th><th>Date</th><th>Notes</th><th>Status</th><th>Link</th></tr>");
                    for r in 0..6 {
                        let _ = write!(html, "<tr><td>1.{r}</td><td>2015-0{}-01</td><td>", r + 1);
                        words.sentence(&mut html, 6);
                        let _ = write!(
                            html,
                            "</td><td>{}</td><td><a href='/v/{r}'>notes</a></td></tr>",
                            words.next()
                        );
                    }
                    html.push_str("</table>");
                }
                3 => {
                    html.push_str("<pre>fn main() {\n    let v: Vec&lt;u32&gt; = (0..10).collect();\n    println!(\"{:?}\", v);\n}</pre>");
                }
                4 => {
                    html.push_str("<div class=cards>");
                    for c in 0..4 {
                        let _ = write!(
                            html,
                            "<div class='card card-{}'><div class=title>{} {}</div>",
                            (a + c) % 97,
                            words.next(),
                            words.next()
                        );
                        words.sentence(&mut html, 12);
                        html.push_str("</div>");
                    }
                    html.push_str("</div>");
                }
                5 => {
                    html.push_str("<div class=gallery>");
                    for g in 0..5 {
                        let (src, w, h) = img.next().expect("image");
                        let _ = write!(html, "<div class='gallerybox gb-{}'><img src='{src}' width={w} height={h} alt=''><div>{}</div></div>", (a + g) % 97, words.next());
                    }
                    html.push_str("</div>");
                }
                _ => {}
            }
            html.push_str("</section>");
        }
        // References.
        html.push_str("<h2>References</h2><div class=references><ol class=refs>");
        for r in 0..20 {
            let _ = write!(html, "<li class='ref-{}' id=r{a}-{r}><cite>", (a + r) % 97);
            words.sentence(&mut html, 9);
            let _ = write!(
                html,
                "</cite> <a href='https://example.org/{a}/{r}'>Retrieved</a></li>"
            );
        }
        html.push_str("</ol></div>");
        // Navigation box: nested tables.
        html.push_str(
            "<table class=navbox><tr><th colspan=2 class=navbox-title>Related topics</th></tr>",
        );
        for g in 0..5 {
            let _ = write!(
                html,
                "<tr><th class=navbox-group>{}</th><td class=navbox-list>",
                words.next()
            );
            if g == 2 {
                html.push_str("<table class=navbox><tr><th class=navbox-group>Sub</th><td class=navbox-list><ul>");
                for k in 0..8 {
                    let _ = write!(
                        html,
                        "<li class='item-{}'><a href='#'>{}</a></li>",
                        (a + k) % 97,
                        words.next()
                    );
                }
                html.push_str("</ul></td></tr></table>");
            } else {
                html.push_str("<ul>");
                for k in 0..12 {
                    let _ = write!(
                        html,
                        "<li class='item-{}'><a href='#'>{} {}</a></li>",
                        (g + k) % 97,
                        words.next(),
                        words.next()
                    );
                }
                html.push_str("</ul>");
            }
            html.push_str("</td></tr>");
        }
        html.push_str("</table></article>");
    }
    html.push_str(
        "<footer class=footer><p>Content is available under a free license.</p></footer>",
    );
    html.push_str("</main></div></body></html>");
    html
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn raster_once(page: &Page, scroll: Point) -> Duration {
    let mut pixmap = Pixmap::new(VIEWPORT.width as u32, VIEWPORT.height as u32).expect("pixmap");
    pixmap.fill(Color::WHITE);
    let start = Instant::now();
    page.render(
        &mut pixmap,
        &RasterOptions {
            scroll,
            ..RasterOptions::default()
        },
    );
    start.elapsed()
}

#[test]
#[ignore = "benchmark: run with --release -- --ignored --nocapture"]
fn realworld_page_pipeline_timings() {
    let html = page();
    // `XBRO_BENCH_SAVE=<file>` keeps the page, to profile it with other tools.
    if let Some(path) = std::env::var_os("XBRO_BENCH_SAVE") {
        std::fs::write(path, &html).expect("saving the page");
    }
    let url = Url::parse("https://bench.test/wiki/Synthetic").expect("url");
    // Warm up fonts and glyph caches.
    let _ = engine().load_html("<p>warm <b>up</b> <i>it</i></p>", url.clone(), VIEWPORT);
    let mut best: Option<(LoadTimings, Duration)> = None;
    let mut page = None;
    for _ in 0..3 {
        let start = Instant::now();
        let p = engine().load_html(&html, url.clone(), VIEWPORT);
        let total = start.elapsed();
        if best.as_ref().is_none_or(|(_, t)| total < *t) {
            best = Some((*p.timings(), total));
        }
        page = Some(p);
    }
    let (t, total) = best.expect("timings");
    let page = page.expect("page");
    // Raster: the first one fills the glyph cache.
    let first = raster_once(&page, Point::ZERO);
    let raster = (0..5)
        .map(|_| raster_once(&page, Point::ZERO))
        .min()
        .unwrap_or_default();
    let mid = Point::new(0.0, page.document_size().height / 2.0);
    let raster_mid = (0..5)
        .map(|_| raster_once(&page, mid))
        .min()
        .unwrap_or_default();
    let image_bytes: usize = html
        .split("data:image/png;base64,")
        .skip(1)
        .map(|rest| rest.find('\'').unwrap_or(0))
        .sum();
    println!(
        "page: {:.0} KB of HTML ({:.0} KB of images), {} DOM nodes, {} images, document {:.0}x{:.0} px, {} display items",
        html.len() as f64 / 1024.0,
        image_bytes as f64 / 1024.0,
        page.document().len(),
        page.images().node_count(),
        page.document_size().width,
        page.document_size().height,
        page.display_list().items.len(),
    );
    println!(
        "parse {:.1} ms | stylesheets {:.1} ms | style {:.1} ms | images {:.1} ms | layout {:.1} ms | display list {:.1} ms | total load {:.1} ms",
        ms(t.parse),
        ms(t.stylesheets),
        ms(t.style),
        ms(t.images),
        ms(t.layout),
        ms(t.display_list),
        ms(total)
    );
    println!(
        "raster 1280x800: first {:.1} ms, top {:.1} ms, middle {:.1} ms",
        ms(first),
        ms(raster),
        ms(raster_mid)
    );
    let console = page.console();
    assert!(console.is_empty(), "console: {console:?}");
    assert!(html.len() > 1_400_000, "{} bytes", html.len());
    assert_eq!(page.images().node_count(), IMAGES);
    if cfg!(debug_assertions) {
        println!("debug build: targets not checked");
        return;
    }
    println!(
        "targets: load < 1000 ms: {}; raster < 16 ms: {}",
        total < Duration::from_secs(1),
        raster.max(raster_mid) < Duration::from_millis(16)
    );
}
