//! Rasterisation performance on a text-heavy synthetic page.
//!
//! `cargo test -p xbro-paint --release --test perf -- --ignored --nocapture`
//! prints the timings and checks the 15 ms budget for a 1280×800 viewport
//! of a text-heavy page (~12,000 glyphs and ~500 rects, more than the
//! ~3,000 glyphs the budget is specified for), plus a "modern" page of
//! rounded, clipped, translucent cards; the non-ignored test runs lighter
//! checks suitable for debug builds.

mod common;

use std::time::{Duration, Instant};

use common::*;
use xbro_layout::{Fragment, LayoutBox, LayoutTree, Rect, Size};
use xbro_paint::{build_display_list_full, rasterize, DisplayList, NoImageProvider, RasterOptions};
use xbro_style::{Color, LengthPercentage, Overflow, TextDecorationLine};
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

/// A "modern" page: 48 rounded `overflow: hidden` cards (header band
/// crossing the rounded corners, text, a translucent badge), half of them
/// translucent as a whole.
fn cards(fonts: &FontSystem) -> LayoutTree {
    let mut children = Vec::new();
    for i in 0..48 {
        let x = 16.0 + (i % 4) as f32 * 314.0;
        let y = 16.0 + (i / 4) as f32 * 190.0;
        let card_rect = Rect::new(x, y, 300.0, 176.0);
        let mut card = bordered(
            100 + i,
            card_rect,
            style(|s| {
                s.background_color = Color::rgb(250, 250, 250);
                s.border = solid(1.0, Color::rgb(220, 220, 220));
                s.border_radius = [LengthPercentage::px(12.0); 4];
                s.overflow_x = Overflow::Hidden;
                s.overflow_y = Overflow::Hidden;
                if i % 2 == 0 {
                    s.opacity = 0.9;
                }
            }),
        );
        let header = block(
            1000 + i,
            Rect::new(x, y, 300.0, 60.0),
            style(|s| {
                s.background_image = Some(xbro_style::BackgroundImage::LinearGradient {
                    angle_deg: 135.0,
                    stops: vec![
                        (Color::rgb(80, 120, 220), 0.0),
                        (Color::rgb(160, 80, 200), 1.0),
                    ],
                });
            }),
        );
        let badge = block(
            2000 + i,
            Rect::new(x + 230.0, y + 70.0, 56.0, 20.0),
            style(|s| {
                s.background_color = Color::rgb(30, 160, 90);
                s.border_radius = [LengthPercentage::px(10.0); 4];
                s.opacity = 0.7;
            }),
        );
        let mut body = block(
            3000 + i,
            Rect::new(x + 12.0, y + 70.0, 210.0, 90.0),
            style(|_| {}),
        );
        body.lines = (0..6)
            .map(|l| {
                let base = y + 84.0 + l as f32 * 15.0;
                let words =
                    WORDS[(i + l) % WORDS.len()].to_string() + " sit amet consectetur adipiscing";
                let f = text(fonts, 3000 + i, &words, x + 12.0, base, 12.0, style(|_| {}));
                line(
                    Rect::new(x + 12.0, base - 12.0, 210.0, 15.0),
                    base,
                    vec![Fragment::Text(f)],
                )
            })
            .collect();
        card.children = vec![header, badge, body];
        children.push(card);
    }
    page(Size::new(1280.0, 800.0), children)
}

fn time_frames(list: &DisplayList, fonts: &FontSystem, frames: usize) -> (Duration, Duration) {
    time_frames_at(list, fonts, frames, 1.0)
}

fn time_frames_at(
    list: &DisplayList,
    fonts: &FontSystem,
    frames: usize,
    scale: f32,
) -> (Duration, Duration) {
    let mut target =
        tiny_skia::Pixmap::new((1280.0 * scale) as u32, (800.0 * scale) as u32).expect("pixmap");
    let options = RasterOptions {
        scale,
        ..RasterOptions::default()
    };
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
    let list = build_display_list_full(&cards(fonts), fonts, &NoImageProvider);
    let (cold, warm) = time_frames(&list, fonts, 2);
    assert!(
        warm < Duration::from_secs(2),
        "cards: cold {cold:?}, warm {warm:?}"
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

    // Rounded overflow clips, gradients and opacity groups.
    let list = build_display_list_full(&cards(fonts), fonts, &NoImageProvider);
    for scale in [1.0, 2.0] {
        let (cold, warm) = time_frames_at(&list, fonts, 15, scale);
        println!(
            "cards ({} items) @{scale}x: cold {cold:?}, warm median {warm:?}",
            list.items.len()
        );
        // A stress case (most of the viewport in translucent groups):
        // loose bound, the machine may be busy.
        if !cfg!(debug_assertions) && scale == 1.0 {
            assert!(
                warm < Duration::from_millis(40),
                "cards frame took {warm:?}"
            );
        }
    }
}
