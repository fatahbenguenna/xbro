//! Text styling at paint time: decoration lines (placement clamped to the
//! font's descent, device-pixel snapping) and synthetic italic/bold glyphs.

mod common;

use common::*;
use xbro_layout::{Fragment, Rect, Size};
use xbro_paint::{
    build_display_list, build_display_list_full, rasterize, DisplayItem, DisplayList, GlyphRun,
    NoImageProvider, RasterOptions,
};
use xbro_style::{Color, TextDecorationLine};
use xbro_text::{FontId, FontQuery, FontSystem, Synthesis};

const SANS: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(x, y, w, h)
}

fn list(items: Vec<DisplayItem>) -> DisplayList {
    DisplayList {
        items,
        background: WHITE,
    }
}

fn decoration(rect: Rect, color: Color) -> DisplayItem {
    DisplayItem::TextDecoration { rect, color }
}

/// Rows (device px) of column `x` whose pixel is close to `color`.
fn rows_of(p: &tiny_skia::Pixmap, x: u32, color: Color) -> Vec<u32> {
    (0..p.height())
        .filter(|&y| close(px(p, x, y), rgb(color), 8))
        .collect()
}

/// Rows of column `x` that are not white at all.
fn touched_rows(p: &tiny_skia::Pixmap, x: u32) -> Vec<u32> {
    (0..p.height())
        .filter(|&y| px(p, x, y) != rgb(WHITE))
        .collect()
}

#[test]
fn decorations_are_whole_device_pixels_thick() {
    // A 1.17 px line is one crisp row wherever it falls (a plain rect of
    // the same size covers one or two rows depending on its position).
    for y in [10.0, 10.25, 10.5, 10.6, 10.9] {
        let p = render(&list(vec![decoration(r(5.0, y, 30.0, 1.17), BLUE)]), 40, 30);
        let rows = touched_rows(&p, 20);
        assert_eq!(rows.len(), 1, "y = {y}: {rows:?}");
        assert_eq!(rows_of(&p, 20, BLUE), rows, "solid, not blended");
        assert_eq!(rows[0], y.round() as u32);
    }
    // 1.5 px rounds to 2 rows, everywhere.
    for y in [10.0, 10.3, 10.7] {
        let p = render(&list(vec![decoration(r(5.0, y, 30.0, 1.5), BLUE)]), 40, 30);
        assert_eq!(rows_of(&p, 20, BLUE).len(), 2, "y = {y}");
        assert_eq!(touched_rows(&p, 20).len(), 2, "y = {y}");
    }
}

#[test]
fn decorations_scale_to_device_pixels() {
    let item = decoration(r(5.0, 10.3, 30.0, 1.17), RED);
    for (scale, rows) in [(1.0, 1), (2.0, 2), (3.0, 4), (0.5, 1), (0.25, 1)] {
        let options = RasterOptions {
            scale,
            ..RasterOptions::default()
        };
        let p = render_with(
            &list(vec![item.clone()]),
            80,
            80,
            &options,
            &NoImageProvider,
        );
        let x = (20.0 * scale) as u32;
        assert_eq!(
            rows_of(&p, x, RED).len(),
            rows,
            "scale {scale}: at least one whole device pixel"
        );
        assert_eq!(touched_rows(&p, x).len(), rows, "scale {scale}: crisp");
    }
}

#[test]
fn decorations_are_clipped_culled_and_faded() {
    // Inside an opacity group, alone: drawn with a faded color.
    let items = vec![
        DisplayItem::PushOpacity(0.5),
        decoration(r(0.0, 5.0, 20.0, 1.0), BLACK),
        DisplayItem::PopOpacity,
    ];
    let p = render(&list(items), 20, 20);
    let c = px(&p, 10, 5);
    assert!(close(c, (128, 128, 128, 255), 2), "{c:?}");
    // Clipped like any other item.
    let items = vec![
        DisplayItem::PushClip {
            rect: r(0.0, 0.0, 10.0, 20.0),
            radii: [0.0; 4],
        },
        decoration(r(0.0, 5.0, 20.0, 1.0), BLACK),
        DisplayItem::PopClip,
    ];
    let p = render(&list(items), 20, 20);
    assert_px(&p, 5, 5, BLACK);
    assert_px(&p, 15, 5, WHITE);
    // Empty, transparent and hostile lines draw nothing and never panic.
    let items = vec![
        decoration(r(0.0, 5.0, 0.0, 1.0), BLACK),
        decoration(r(0.0, 5.0, 20.0, 1.0), Color::TRANSPARENT),
        decoration(r(f32::NAN, f32::INFINITY, f32::NAN, f32::NAN), BLACK),
        decoration(r(-1.0e30, 1.0e30, 1.0e30, 1.0e30), BLACK),
    ];
    let p = render(&list(items), 20, 20);
    assert!(p.data().iter().all(|&b| b == 255));
}

/// DejaVu Sans with its `post` underline position replaced by
/// `position` font units (negative = below the baseline).
fn sans_with_underline_at(position: i16) -> Option<FontSystem> {
    let mut font = std::fs::read(SANS).ok()?;
    let count = usize::from(u16::from_be_bytes([font[4], font[5]]));
    let post = (0..count).find_map(|i| {
        let record = 12 + i * 16;
        (font.get(record..record + 4)? == b"post").then(|| {
            let at = record + 8;
            u32::from_be_bytes(font[at..at + 4].try_into().unwrap()) as usize
        })
    })?;
    // post: version (4), italicAngle (4), underlinePosition (2).
    font[post + 8..post + 10].copy_from_slice(&position.to_be_bytes());
    let fonts = FontSystem::from_fonts(vec![font]);
    fonts.has_fonts().then_some(fonts)
}

/// Builds the display list of one underlined fragment and returns the
/// underline rect and the fragment's baseline and font.
fn underline_of(fonts: &FontSystem, size: f32) -> (Rect, f32, FontId) {
    let deco = style(|s| {
        s.decorations_in_effect = TextDecorationLine {
            underline: true,
            ..TextDecorationLine::NONE
        };
        s.decoration_color = BLUE;
    });
    let frag = text(fonts, 4, "under", 10.0, 40.0, size, deco);
    let font = frag.font;
    let mut para = block(3, r(0.0, 0.0, 200.0, 80.0), style(|_| {}));
    para.lines = vec![line(
        r(0.0, 0.0, 200.0, 80.0),
        40.0,
        vec![Fragment::Text(frag)],
    )];
    let tree = page(Size::new(200.0, 80.0), vec![para]);
    let list = build_display_list_full(&tree, fonts, &NoImageProvider);
    let rect = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::TextDecoration { rect, .. } => Some(*rect),
            _ => None,
        })
        .expect("underline");
    (rect, 40.0, font)
}

#[test]
fn underlines_never_go_below_the_descent() {
    // A font whose underline sits 1.5em below the baseline (far below its
    // descent, into the next line).
    let Some(fonts) = sans_with_underline_at(-3072) else {
        return;
    };
    let size = 20.0;
    let (rect, baseline, font) = underline_of(&fonts, size);
    let m = fonts.metrics(font, size);
    assert!(m.underline_offset > m.descent + 1.0, "{m:?}");
    let bottom = rect.y + rect.height;
    assert!(
        bottom <= baseline + m.descent + 1.0 + 1e-3,
        "underline bottom {bottom} below the descent {}",
        baseline + m.descent
    );
    assert!(rect.y > baseline, "still below the baseline: {}", rect.y);

    // A sane font keeps its metric placement.
    let Some(fonts) = common::fonts() else { return };
    let (rect, baseline, font) = underline_of(fonts, size);
    let m = fonts.metrics(font, size);
    assert!(m.underline_offset + m.underline_thickness < m.descent);
    assert!((rect.y - (baseline + m.underline_offset)).abs() < 1e-4);
    assert!((rect.height - m.underline_thickness.max(1.0)).abs() < 1e-4);
}

#[test]
fn underlines_never_go_above_the_baseline() {
    // A font whose underline is above the baseline is moved down to it.
    let Some(fonts) = sans_with_underline_at(400) else {
        return;
    };
    let (rect, baseline, _) = underline_of(&fonts, 20.0);
    assert!(rect.y >= baseline, "{} vs baseline {baseline}", rect.y);
    // Without fonts, the fragment's box gives the descent.
    let deco = style(|s| {
        s.decorations_in_effect = TextDecorationLine {
            underline: true,
            ..TextDecorationLine::NONE
        };
        s.decoration_color = BLUE;
    });
    let Some(f) = common::fonts() else { return };
    let mut frag = text(f, 4, "x", 0.0, 30.0, 40.0, deco);
    // Descent of 0.5px only: the 40/14 px underline sits on the baseline.
    frag.rect = r(0.0, 0.0, frag.rect.width, 30.5);
    let mut para = block(3, r(0.0, 0.0, 200.0, 40.0), style(|_| {}));
    para.lines = vec![line(
        r(0.0, 0.0, 200.0, 40.0),
        30.0,
        vec![Fragment::Text(frag)],
    )];
    let list = build_display_list(&page(Size::new(200.0, 40.0), vec![para]));
    let Some(DisplayItem::TextDecoration { rect, .. }) = list
        .items
        .iter()
        .find(|i| matches!(i, DisplayItem::TextDecoration { .. }))
    else {
        panic!("underline");
    };
    assert_eq!(rect.y, 30.0);
}

/// A glyph run of `s` drawn with `font` at (10, 60), 40px.
fn run(fonts: &FontSystem, font: FontId, s: &str) -> DisplayItem {
    let shaped = fonts.shape(font, 40.0, s, 0.0, 0.0);
    DisplayItem::Text(GlyphRun {
        font,
        font_size: 40.0,
        origin: xbro_layout::Point::new(10.0, 60.0),
        glyphs: shaped.glyphs,
        color: BLACK,
    })
}

fn draw(fonts: &FontSystem, items: Vec<DisplayItem>) -> tiny_skia::Pixmap {
    let mut p = tiny_skia::Pixmap::new(200, 80).expect("pixmap");
    rasterize(
        &list(items),
        fonts,
        &NoImageProvider,
        &mut p,
        &RasterOptions::default(),
    );
    p
}

/// Leftmost dark column of row `y`.
fn left_ink(p: &tiny_skia::Pixmap, y: u32) -> Option<u32> {
    (0..p.width()).find(|&x| {
        let (r, g, b, _) = px(p, x, y);
        u32::from(r) + u32::from(g) + u32::from(b) < 3 * 128
    })
}

#[test]
fn synthetic_italic_text_is_drawn_slanted() {
    let Some(fonts) = fonts() else { return };
    let families = vec!["dejavu sans".to_string()];
    let query = |weight, italic| {
        fonts.select(&FontQuery {
            families: &families,
            weight,
            italic,
        })
    };
    let italic = query(400, true);
    assert_eq!(
        italic.synthesis(),
        Synthesis {
            oblique: true,
            bold: false
        }
    );
    let upright = draw(fonts, vec![run(fonts, query(400, false), "l")]);
    let slanted = draw(fonts, vec![run(fonts, italic, "l")]);
    // Rows near the top (baseline 60, 'l' ≈ 30px tall) and the bottom.
    let (top, bottom) = (34, 58);
    let (ut, ub) = (
        left_ink(&upright, top).unwrap(),
        left_ink(&upright, bottom).unwrap(),
    );
    let (st, sb) = (
        left_ink(&slanted, top).unwrap(),
        left_ink(&slanted, bottom).unwrap(),
    );
    assert!(ut.abs_diff(ub) <= 1, "upright: {ut} / {ub}");
    let shift = st as f32 - sb as f32;
    let expected = 0.25 * (bottom - top) as f32;
    assert!(
        (shift - expected).abs() <= 1.5,
        "slanted: {st} at the top, {sb} at the bottom (expected ≈{expected})"
    );
}

#[test]
fn synthetic_bold_text_is_wider_and_darker() {
    let Some(fonts) = fonts() else { return };
    let regular = sans(fonts);
    let bold = regular.with_synthesis(Synthesis {
        oblique: false,
        bold: true,
    });
    let thin = draw(fonts, vec![run(fonts, regular, "ll")]);
    let thick = draw(fonts, vec![run(fonts, bold, "ll")]);
    let dark = |p: &tiny_skia::Pixmap| dark_pixels(p, 0, 0, 200, 80, 128);
    assert!(
        dark(&thick) as f32 > 1.3 * dark(&thin) as f32,
        "{} vs {}",
        dark(&thick),
        dark(&thin)
    );
    // The second 'l' moves right by 40/24 px (the first glyph's extra advance).
    let row = 50;
    let starts = |p: &tiny_skia::Pixmap| {
        let mut starts = Vec::new();
        let mut inside = false;
        for x in 0..p.width() {
            let (r, _, _, _) = px(p, x, row);
            let ink = r < 128;
            if ink && !inside {
                starts.push(x);
            }
            inside = ink;
        }
        starts
    };
    let (a, b) = (starts(&thin), starts(&thick));
    assert_eq!((a.len(), b.len()), (2, 2), "{a:?} / {b:?}");
    assert_eq!(a[0], b[0]);
    let moved = (b[1] - a[1]) as f32;
    assert!((moved - 40.0 / 24.0).abs() <= 1.0, "moved {moved}");
}
