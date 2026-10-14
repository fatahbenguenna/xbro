//! Performance of the whole pipeline on a synthetic ~1 MB page.
//!
//! Run with `cargo test -p xbro-engine --release --test perf -- --nocapture`
//! to see the timings; the 1.5 s budget is only enforced in release builds.

mod common;

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use common::*;
use xbro_engine::{Point, RasterOptions, Url};

/// About 1 MB of HTML with roughly 15 000 elements (an encyclopedia article).
fn synthetic_page() -> String {
    let words = [
        "lorem",
        "ipsum",
        "dolor",
        "sit",
        "amet",
        "consectetur",
        "adipiscing",
        "elit",
        "sed",
        "do",
        "eiusmod",
        "tempor",
        "incididunt",
        "labore",
        "dolore",
        "magna",
        "aliqua",
        "enim",
        "minim",
        "veniam",
        "quis",
        "nostrud",
        "exercitation",
        "ullamco",
        "laboris",
    ];
    let mut html = String::with_capacity(1_100_000);
    html.push_str("<!DOCTYPE html><html><head><title>Synthetic</title></head><body>");
    html.push_str("<div class=page><nav class=toc><ul>");
    for s in 0..52 {
        let _ = write!(html, "<li><a href='#s{s}'>Section {s}</a></li>");
    }
    html.push_str("</ul></nav><main class=content>");
    let mut w = 0usize;
    let mut word = || {
        w += 1;
        words[(w * 7 + w / 3) % words.len()]
    };
    for s in 0..52 {
        let _ = write!(
            html,
            "<section class='sec s{s}'><h2 id=s{s}>Section {s}</h2>"
        );
        for p in 0..20 {
            let _ = write!(html, "<p class='para p{}'>", p % 10);
            for k in 0..60 {
                match k % 12 {
                    3 => {
                        let _ = write!(
                            html,
                            "<a class='l{}' href='/wiki/{}'>{}</a> ",
                            k % 50,
                            word(),
                            word()
                        );
                    }
                    7 => {
                        let _ = write!(html, "<b>{}</b> ", word());
                    }
                    10 => {
                        let _ = write!(
                            html,
                            "<span class='c{}'>{} {}</span> ",
                            (s + k) % 100,
                            word(),
                            word()
                        );
                    }
                    _ => {
                        let _ = write!(html, "{} ", word());
                    }
                }
            }
            html.push_str("</p>");
        }
        html.push_str("<ul class=list>");
        for i in 0..6 {
            let _ = write!(
                html,
                "<li class='item i{i}'>{} <i>{}</i> {}</li>",
                word(),
                word(),
                word()
            );
        }
        html.push_str("</ul><table class=data><tr><th>Key</th><th>Value</th></tr>");
        for r in 0..4 {
            let _ = write!(html, "<tr><td>{r}</td><td>{} {}</td></tr>", word(), word());
        }
        html.push_str("</table></section>");
    }
    html.push_str("</main></div></body></html>");
    html
}

/// A 2 000-rule stylesheet mixing the usual selector shapes.
fn synthetic_css() -> String {
    let mut css = String::from(
        "body { font: 14px/1.6 sans-serif; margin: 0 } .page { display: flex; gap: 16px } \
         .toc { flex: 0 0 180px } .content { flex: 1; min-width: 0 } \
         table.data { border-collapse: collapse } td, th { border: 1px solid #ccc; padding: 2px 4px }\n",
    );
    for i in 0..2000 {
        let rule = match i % 8 {
            0 => format!(".c{} {{ color: rgb({}, 40, 90) }}\n", i % 100, i % 255),
            1 => format!(
                ".sec.s{} p.p{} {{ margin: {}px 0 }}\n",
                i % 40,
                i % 10,
                8 + i % 5
            ),
            2 => format!(
                "main .para a.l{}:hover {{ text-decoration: underline }}\n",
                i % 50
            ),
            3 => format!(
                "#s{} {{ border-bottom: 1px solid #{:03x} }}\n",
                i % 40,
                i % 4096
            ),
            4 => format!("ul.list > li.i{} {{ padding-left: {}px }}\n", i % 6, i % 20),
            5 => format!("[data-x{i}] {{ color: red }}\n"),
            6 => format!(".unused-{i} .deep div span {{ display: none }}\n"),
            _ => format!(
                "@media (max-width: {}px) {{ .c{} {{ font-size: 12px }} }}\n",
                300 + i,
                i % 100
            ),
        };
        css.push_str(&rule);
    }
    css
}

#[test]
fn one_megabyte_page_under_budget() {
    let html = synthetic_page();
    let css = synthetic_css();
    let full = html.replacen("</head>", &format!("<style>{css}</style></head>"), 1);
    let elements = full.matches('<').count() - full.matches("</").count();
    // Warm up fonts and caches with a small page.
    let _ = load_html("<p>warm <b>up</b></p>");
    let start = Instant::now();
    let mut page = engine().load_html(
        &full,
        Url::parse("https://perf.test/article").unwrap(),
        VIEWPORT,
    );
    let load = start.elapsed();
    let t = *page.timings();
    let raster_start = Instant::now();
    let mut pixmap = xbro_paint::tiny_skia::Pixmap::new(1024, 768).unwrap();
    page.render(&mut pixmap, &RasterOptions::default());
    let raster = raster_start.elapsed();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!(
        "synthetic page: {:.0} KB, ~{elements} elements, {} nodes, document {:.0}x{:.0}",
        full.len() as f64 / 1024.0,
        page.document().len(),
        page.document_size().width,
        page.document_size().height
    );
    println!(
        "parse {:.1} ms | stylesheets {:.1} ms | style {:.1} ms | images {:.1} ms | layout {:.1} ms | display list {:.1} ms | raster {:.1} ms | load_html total {:.1} ms",
        ms(t.parse),
        ms(t.stylesheets),
        ms(t.style),
        ms(t.images),
        ms(t.layout),
        ms(t.display_list),
        ms(raster),
        ms(load)
    );
    assert!(full.len() > 900_000, "{} bytes", full.len());
    assert!(elements > 14_000, "{elements} elements");
    assert!(page.document_size().height > 10_000.0);
    // Interactions on a big page.
    let find_start = Instant::now();
    let hits = page.find("lorem");
    let find = find_start.elapsed();
    let hover_start = Instant::now();
    let link = page
        .find("Section 3")
        .first()
        .map(|r| Point::new(r.x + 2.0, r.y + 2.0))
        .unwrap_or(Point::ZERO);
    page.set_hover(Some((link, Point::ZERO)));
    let hover = hover_start.elapsed();
    println!(
        "find {:.1} ms ({} hits) | hover {:.1} ms",
        ms(find),
        hits.len(),
        ms(hover)
    );
    assert!(!hits.is_empty());
    let total = load + raster;
    if cfg!(debug_assertions) {
        println!("debug build: budget not enforced ({:.0} ms)", ms(total));
    } else {
        assert!(
            total < Duration::from_millis(1500),
            "pipeline took {:.0} ms",
            ms(total)
        );
    }
}
