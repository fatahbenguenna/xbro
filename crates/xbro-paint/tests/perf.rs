//! Rasterisation performance on a text-heavy synthetic page.
//!
//! `cargo test -p xbro-paint --release --test perf -- --ignored --nocapture`
//! prints the timings and checks the 15 ms budget for a 1280×800 viewport
//! with ~3,000 glyphs and ~500 rects; the non-ignored test runs a lighter
//! check suitable for debug builds.

mod common;

use std::time::{Duration, Instant};

use common::*;
use xbro_layout::{Fragment, LayoutBox, LayoutTree, Rect, Size};
use xbro_paint::{build_display_list_full, rasterize, DisplayList, NoImageProvider, RasterOptions};
use xbro_style::{Color, LengthPercentage, TextDecorationLine};
use xbro_text::FontSystem;

const WORDS: &[&str] = &[
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
    "ut",
    "labore",
    "et",
    "dolore",
    "magna",
    "aliqua",
];

/// A page of `lines` text lines (~55 glyphs each) and ~`rects` decorated boxes.
fn scene(fonts: &FontSystem, lines: usize, rects: usize) -> LayoutTree {
    let mut children: Vec<LayoutBox> = Vec::new();
    let link = style(|s| {
        s.color = Color::rgb(0, 0, 238);
        s.decorations_in_effect = TextDecorationLine {
            underline: true,
            ..TextDecorationLine::NONE
        };
        s.decoration_color = Color::rgb(0, 0, 238);
    });
    let plain = style(|_| {});
    for i in 0..lines {
        let y = 4.0 + i as f32 * 13.0;
        let mut para = block(10 + i, Rect::new(8.0, y, 1264.0, 13.0), style(|_| {}));
        let mut x = 8.0;
        let mut frags = Vec::new();
        let mut w = i;
        while x < 1150.0 {
            let word = format!("{} ", WORDS[w % WORDS.len()]);
            w += 7;
            let st = if w % 11 == 0 {
                link.clone()
            } else {
                plain.clone()
            };
            let f = text(fonts, 10 + i, &word, x, y + 10.0, 11.0, st);
            x += f.rect.width;
            frags.push(Fragment::Text(f));
        }
        para.lines = vec![line(Rect::new(8.0, y, 1264.0, 13.0), y + 10.0, frags)];
        children.push(para);
    }
    for i in 0..rects {
        let x = (i % 25) as f32 * 50.0;
        let y = (i / 25) as f32 * 40.0;
        let color = Color::rgb((i * 37 % 256) as u8, (i * 91 % 256) as u8, 200);
        let b = bordered(
            10_000 + i,
            Rect::new(x, y, 46.0, 30.0),
            style(|s| {
                s.background_color = Color::rgba(color.r, color.g, color.b, 60);
                if i % 3 == 0 {
                    s.border = solid(1.0, color);
                }
                if i % 7 == 0 {
                    s.border_radius = [LengthPercentage::px(4.0); 4];
                }
            }),
        );
        children.push(b);
    }
    page(Size::new(1280.0, 800.0), children)
}

fn time_frames(list: &DisplayList, fonts: &FontSystem, frames: usize) -> (Duration, Duration) {
    let mut target = tiny_skia::Pixmap::new(1280, 800).expect("pixmap");
    let options = RasterOptions::default();
    let start = Instant::now();
    rasterize(list, fonts, &NoImageProvider, &mut target, &options);
    let cold = start.elapsed();
    let mut times = Vec::with_capacity(frames);
    for _ in 0..frames {
        let start = Instant::now();
        rasterize(list, fonts, &NoImageProvider, &mut target, &options);
        times.push(start.elapsed());
    }
    times.sort();
    (cold, times[times.len() / 2])
}

fn glyph_count(list: &DisplayList) -> usize {
    list.items
        .iter()
        .map(|i| match i {
            xbro_paint::DisplayItem::Text(run) => run.glyphs.len(),
            _ => 0,
        })
        .sum()
}

#[test]
fn raster_perf_light() {
    let Some(fonts) = fonts() else { return };
    let tree = scene(fonts, 60, 500);
    let list = build_display_list_full(&tree, fonts, &NoImageProvider);
    assert!(glyph_count(&list) > 2500, "{}", glyph_count(&list));
    let (cold, warm) = time_frames(&list, fonts, 3);
    // Generous bound for unoptimised builds; the release bench checks 15 ms.
    assert!(
        warm < Duration::from_secs(2),
        "cold {cold:?}, warm {warm:?}"
    );
}

#[test]
#[ignore = "benchmark: run with --release -- --ignored --nocapture"]
fn raster_bench() {
    let Some(fonts) = fonts() else { return };
    let tree = scene(fonts, 60, 500);
    let start = Instant::now();
    let list = build_display_list_full(&tree, fonts, &NoImageProvider);
    let build = start.elapsed();
    let glyphs = glyph_count(&list);
    let (cold, warm) = time_frames(&list, fonts, 30);
    println!(
        "display list: {} items, {glyphs} glyphs, built in {build:?}; raster 1280x800: cold {cold:?}, warm median {warm:?}",
        list.items.len()
    );
    // 2x (HiDPI) for information.
    let mut big = tiny_skia::Pixmap::new(2560, 1600).expect("pixmap");
    let options = RasterOptions {
        scale: 2.0,
        ..RasterOptions::default()
    };
    rasterize(&list, fonts, &NoImageProvider, &mut big, &options);
    let start = Instant::now();
    rasterize(&list, fonts, &NoImageProvider, &mut big, &options);
    println!("raster 2560x1600 @2x (warm): {:?}", start.elapsed());
    if !cfg!(debug_assertions) {
        assert!(warm < Duration::from_millis(15), "warm frame took {warm:?}");
    }
}
