//! Pixel-level rasterisation tests on hand-built display lists and trees.

mod common;

use std::sync::Arc;

use common::*;
use xbro_dom::NodeId;
use xbro_layout::{BoxKind, ControlKind, Fragment, ListMarker, MarkerContent, Point, Rect, Size};
use xbro_paint::{
    build_display_list, build_display_list_full, draw_text, BorderSideItem, DisplayItem,
    DisplayList, GlyphRun, Image, NoImageProvider, RasterOptions,
};
use xbro_style::{
    BackgroundRepeat, BackgroundSize, BorderStyle, Color, LengthPercentage, Overflow, Position,
    TextDecorationLine,
};

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(x, y, w, h)
}

fn list(background: Color, items: Vec<DisplayItem>) -> DisplayList {
    DisplayList { items, background }
}

fn rect(x: f32, y: f32, w: f32, h: f32, color: Color) -> DisplayItem {
    DisplayItem::Rect {
        rect: r(x, y, w, h),
        color,
        radii: [0.0; 4],
    }
}

fn side(width: f32, style: BorderStyle, color: Color) -> BorderSideItem {
    BorderSideItem {
        width,
        style,
        color,
    }
}

#[test]
fn background_and_rects() {
    let l = list(
        BLUE,
        vec![
            rect(10.0, 10.0, 20.0, 20.0, RED),
            rect(-50.0, -50.0, 5.0, 5.0, GREEN),
        ],
    );
    let p = render(&l, 50, 50);
    assert_px(&p, 0, 0, BLUE);
    assert_px(&p, 49, 49, BLUE);
    assert_px(&p, 10, 10, RED);
    assert_px(&p, 29, 29, RED);
    assert_px(&p, 30, 30, BLUE);
}

#[test]
fn fractional_rects_snap_to_pixels() {
    let l = list(WHITE, vec![rect(10.3, 10.6, 5.2, 0.4, BLACK)]);
    let p = render(&l, 30, 30);
    // y 10.6..11.0 snaps to row 11 (at least one pixel tall).
    assert_px(&p, 12, 11, BLACK);
    assert_px(&p, 12, 10, WHITE);
    assert_px(&p, 12, 12, WHITE);
}

#[test]
fn translucent_rect_blends() {
    let l = list(
        WHITE,
        vec![rect(0.0, 0.0, 10.0, 10.0, Color::rgba(0, 0, 255, 128))],
    );
    let p = render(&l, 10, 10);
    assert!(
        close(px(&p, 5, 5), (127, 127, 255, 255), 2),
        "{:?}",
        px(&p, 5, 5)
    );
}

#[test]
fn border_colors_per_side() {
    let l = list(
        WHITE,
        vec![DisplayItem::Border {
            rect: r(10.0, 10.0, 60.0, 60.0),
            sides: [
                side(10.0, BorderStyle::Solid, RED),
                side(10.0, BorderStyle::Solid, LIME),
                side(10.0, BorderStyle::Solid, BLUE),
                side(10.0, BorderStyle::Solid, BLACK),
            ],
            radii: [0.0; 4],
        }],
    );
    let p = render(&l, 80, 80);
    assert_px(&p, 40, 12, RED);
    assert_px(&p, 67, 40, LIME);
    assert_px(&p, 40, 67, BLUE);
    assert_px(&p, 12, 40, BLACK);
    assert_px(&p, 40, 40, WHITE);
    assert_px(&p, 5, 5, WHITE);
    // Diagonal join in the top-left corner: above the diagonal is top (red), below is left (black).
    assert_px(&p, 17, 11, RED);
    assert_px(&p, 11, 17, BLACK);
}

#[test]
fn uniform_border_ring_and_missing_sides() {
    let l = list(
        WHITE,
        vec![DisplayItem::Border {
            rect: r(0.0, 0.0, 40.0, 40.0),
            sides: [
                side(4.0, BorderStyle::Solid, RED),
                side(0.0, BorderStyle::None, RED),
                side(4.0, BorderStyle::Solid, RED),
                side(4.0, BorderStyle::Hidden, BLUE),
            ],
            radii: [0.0; 4],
        }],
    );
    let p = render(&l, 40, 40);
    assert_px(&p, 20, 1, RED);
    assert_px(&p, 20, 38, RED);
    assert_px(&p, 1, 20, WHITE);
    assert_px(&p, 38, 20, WHITE);
}

#[test]
fn border_styles() {
    let make = |style, color| {
        list(
            WHITE,
            vec![DisplayItem::Border {
                rect: r(0.0, 0.0, 60.0, 60.0),
                sides: [side(6.0, style, color); 4],
                radii: [0.0; 4],
            }],
        )
    };
    // Double: outer and inner thirds colored, middle third empty.
    let p = render(&make(BorderStyle::Double, BLACK), 60, 60);
    assert_px(&p, 30, 0, BLACK);
    assert_px(&p, 30, 3, WHITE);
    assert_px(&p, 30, 5, BLACK);
    // Dashed: both ink and gaps along the top side.
    let p = render(&make(BorderStyle::Dashed, BLACK), 60, 60);
    let row: Vec<_> = (6..54).map(|x| px(&p, x, 3)).collect();
    assert!(row.iter().any(|c| close(*c, rgb(BLACK), 10)));
    assert!(row.iter().any(|c| close(*c, rgb(WHITE), 10)));
    // Dotted.
    let p = render(&make(BorderStyle::Dotted, BLACK), 60, 60);
    let row: Vec<_> = (6..54).map(|x| px(&p, x, 3)).collect();
    assert!(row.iter().any(|c| c.0 < 128));
    assert!(row.iter().any(|c| close(*c, rgb(WHITE), 10)));
    // Inset: top darker than bottom.
    let grey = Color::rgb(200, 200, 200);
    let p = render(&make(BorderStyle::Inset, grey), 60, 60);
    assert_px(&p, 30, 2, Color::rgb(120, 120, 120));
    assert_px(&p, 30, 57, grey);
    // Outset: the opposite.
    let p = render(&make(BorderStyle::Outset, grey), 60, 60);
    assert_px(&p, 30, 2, grey);
    assert_px(&p, 30, 57, Color::rgb(120, 120, 120));
    // Groove: outer half of the top dark, inner half light.
    let p = render(&make(BorderStyle::Groove, grey), 60, 60);
    assert_px(&p, 30, 1, Color::rgb(120, 120, 120));
    assert_px(&p, 30, 4, grey);
    let p = render(&make(BorderStyle::Ridge, grey), 60, 60);
    assert_px(&p, 30, 1, grey);
    assert_px(&p, 30, 4, Color::rgb(120, 120, 120));
}

#[test]
fn rounded_corners_keep_background() {
    let l = list(
        WHITE,
        vec![
            DisplayItem::Rect {
                rect: r(0.0, 0.0, 40.0, 40.0),
                color: RED,
                radii: [10.0; 4],
            },
            DisplayItem::Border {
                rect: r(50.0, 0.0, 40.0, 40.0),
                sides: [side(3.0, BorderStyle::Solid, BLUE); 4],
                radii: [12.0; 4],
            },
        ],
    );
    let p = render(&l, 100, 40);
    assert_px(&p, 0, 0, WHITE);
    assert_px(&p, 39, 39, WHITE);
    assert_px(&p, 20, 20, RED);
    assert_px(&p, 20, 0, RED);
    assert_px(&p, 0, 20, RED);
    // Ring: corner pixel untouched, side colored, inside untouched.
    assert_px(&p, 50, 0, WHITE);
    assert_px(&p, 70, 1, BLUE);
    assert_px(&p, 70, 20, WHITE);
}

#[test]
fn rounded_border_with_differing_colors() {
    let l = list(
        WHITE,
        vec![DisplayItem::Border {
            rect: r(0.0, 0.0, 60.0, 60.0),
            sides: [
                side(5.0, BorderStyle::Solid, RED),
                side(5.0, BorderStyle::Solid, BLUE),
                side(5.0, BorderStyle::Solid, RED),
                side(5.0, BorderStyle::Solid, BLUE),
            ],
            radii: [15.0; 4],
        }],
    );
    let p = render(&l, 60, 60);
    assert_px(&p, 0, 0, WHITE);
    assert_px(&p, 30, 2, RED);
    assert_px(&p, 2, 30, BLUE);
    assert_px(&p, 30, 30, WHITE);
}

#[test]
fn overflow_hidden_clips_children() {
    let child = block(
        4,
        r(20.0, 20.0, 100.0, 100.0),
        style(|s| s.background_color = RED),
    );
    let clipper = with_children(
        block(
            3,
            r(10.0, 10.0, 30.0, 30.0),
            style(|s| {
                s.overflow_x = Overflow::Hidden;
                s.overflow_y = Overflow::Hidden;
            }),
        ),
        vec![child],
    );
    let p = render(
        &build_display_list(&page(Size::new(100.0, 100.0), vec![clipper])),
        100,
        100,
    );
    assert_px(&p, 25, 25, RED);
    assert_px(&p, 39, 39, RED);
    assert_px(&p, 41, 41, WHITE);
    assert_px(&p, 60, 25, WHITE);
}

#[test]
fn rounded_clip_masks_children_and_text() {
    let mut items = vec![DisplayItem::PushClip {
        rect: r(0.0, 0.0, 40.0, 40.0),
        radii: [20.0; 4],
    }];
    items.push(rect(0.0, 0.0, 40.0, 40.0, RED));
    items.push(DisplayItem::PopClip);
    items.push(rect(45.0, 0.0, 5.0, 5.0, BLUE));
    let p = render(&list(WHITE, items), 50, 40);
    assert_px(&p, 1, 1, WHITE);
    assert_px(&p, 20, 20, RED);
    assert_px(&p, 46, 1, BLUE);
}

#[test]
fn nested_clips_intersect() {
    let items = vec![
        DisplayItem::PushClip {
            rect: r(0.0, 0.0, 30.0, 30.0),
            radii: [0.0; 4],
        },
        DisplayItem::PushClip {
            rect: r(10.0, 10.0, 40.0, 40.0),
            radii: [0.0; 4],
        },
        rect(0.0, 0.0, 50.0, 50.0, RED),
        DisplayItem::PopClip,
        DisplayItem::PopClip,
        DisplayItem::PopClip, // unbalanced: ignored
    ];
    let p = render(&list(WHITE, items), 50, 50);
    assert_px(&p, 5, 5, WHITE);
    assert_px(&p, 15, 15, RED);
    assert_px(&p, 29, 29, RED);
    assert_px(&p, 35, 35, WHITE);
}

#[test]
fn opacity_half_blends() {
    let items = vec![
        DisplayItem::PushOpacity(0.5),
        rect(0.0, 0.0, 10.0, 10.0, RED),
        rect(5.0, 0.0, 10.0, 10.0, BLUE),
        DisplayItem::PopOpacity,
    ];
    let p = render(&list(WHITE, items), 20, 10);
    assert!(
        close(px(&p, 2, 5), (255, 127, 127, 255), 2),
        "{:?}",
        px(&p, 2, 5)
    );
    // Group opacity: the overlap shows only blue at 50%, not red through blue.
    assert!(
        close(px(&p, 7, 5), (127, 127, 255, 255), 2),
        "{:?}",
        px(&p, 7, 5)
    );
    assert_px(&p, 17, 5, WHITE);
}

#[test]
fn nested_and_unbalanced_opacity() {
    let items = vec![
        DisplayItem::PushOpacity(0.5),
        DisplayItem::PushOpacity(0.5),
        rect(0.0, 0.0, 10.0, 10.0, BLACK),
        DisplayItem::PopOpacity,
        DisplayItem::PushOpacity(0.0),
        rect(0.0, 0.0, 10.0, 10.0, RED),
        DisplayItem::PopOpacity,
        DisplayItem::PushOpacity(1.0),
        DisplayItem::PushOpacity(0.5),
        // missing pops: composited at the end
    ];
    let p = render(&list(WHITE, items), 10, 10);
    // 25% black over white.
    assert!(
        close(px(&p, 5, 5), (191, 191, 191, 255), 2),
        "{:?}",
        px(&p, 5, 5)
    );
}

#[test]
fn z_index_order_of_overlapping_boxes() {
    let pos = |node, x, z, color| {
        block(
            node,
            r(x, 0.0, 40.0, 40.0),
            style(|s| {
                s.position = Position::Absolute;
                s.z_index = Some(z);
                s.background_color = color;
            }),
        )
    };
    let tree = page(
        Size::new(100.0, 50.0),
        vec![pos(3, 0.0, 5, RED), pos(4, 20.0, 1, BLUE)],
    );
    let p = render(&build_display_list(&tree), 100, 50);
    assert_px(&p, 10, 10, RED);
    assert_px(&p, 30, 10, RED);
    assert_px(&p, 50, 10, BLUE);
}

#[test]
fn fixed_boxes_ignore_scroll() {
    let fixed = block(
        3,
        r(0.0, 0.0, 100.0, 10.0),
        style(|s| {
            s.position = Position::Fixed;
            s.background_color = RED;
        }),
    );
    let flow = block(
        4,
        r(0.0, 100.0, 100.0, 10.0),
        style(|s| s.background_color = BLUE),
    );
    let mut tree = page(Size::new(100.0, 50.0), vec![fixed, flow]);
    tree.document_size = Size::new(100.0, 500.0);
    let options = RasterOptions {
        scroll: Point::new(0.0, 95.0),
        ..RasterOptions::default()
    };
    let p = render_with(
        &build_display_list(&tree),
        100,
        50,
        &options,
        &NoImageProvider,
    );
    assert_px(&p, 50, 5, RED);
    assert_px(&p, 50, 12, BLUE);
    assert_px(&p, 50, 30, WHITE);
}

#[test]
fn fixed_content_escapes_ancestor_clips() {
    let items = vec![
        DisplayItem::PushClip {
            rect: r(0.0, 0.0, 10.0, 10.0),
            radii: [0.0; 4],
        },
        DisplayItem::PushFixed,
        rect(20.0, 20.0, 10.0, 10.0, RED),
        DisplayItem::PopFixed,
        rect(20.0, 0.0, 10.0, 10.0, BLUE),
        DisplayItem::PopClip,
    ];
    let p = render(&list(WHITE, items), 40, 40);
    assert_px(&p, 25, 25, RED);
    assert_px(&p, 25, 5, WHITE);
}

#[test]
fn scroll_scale_and_offset_mapping() {
    let l = list(WHITE, vec![rect(10.0, 20.0, 5.0, 5.0, RED)]);
    let options = RasterOptions {
        scroll: Point::new(5.0, 10.0),
        scale: 2.0,
        offset: (3, 4),
        height: None,
    };
    let p = render_with(&l, 60, 60, &options, &NoImageProvider);
    // Device = (doc − scroll) × 2 + offset → x 13..23, y 24..34.
    assert_px(&p, 13, 24, RED);
    assert_px(&p, 22, 33, RED);
    assert_px(&p, 12, 24, WHITE);
    assert_px(&p, 23, 33, WHITE);
    assert_px(&p, 13, 23, WHITE);
}

#[test]
fn drawing_area_band() {
    let l = list(BLUE, vec![rect(0.0, 0.0, 100.0, 100.0, RED)]);
    let mut p = tiny_skia::Pixmap::new(20, 40).unwrap();
    p.fill(tiny_skia::Color::from_rgba8(0, 255, 0, 255));
    let options = RasterOptions {
        offset: (0, 10),
        height: Some(20),
        ..RasterOptions::default()
    };
    xbro_paint::rasterize(&l, no_fonts(), &NoImageProvider, &mut p, &options);
    assert_px(&p, 5, 5, LIME);
    assert_px(&p, 5, 10, RED);
    assert_px(&p, 5, 29, RED);
    assert_px(&p, 5, 30, LIME);
    // Out-of-range offsets and heights are clamped.
    let options = RasterOptions {
        offset: (0, 1000),
        height: Some(u32::MAX),
        ..RasterOptions::default()
    };
    xbro_paint::rasterize(&l, no_fonts(), &NoImageProvider, &mut p, &options);
    let options = RasterOptions {
        offset: (-5, -10),
        scale: f32::NAN,
        ..RasterOptions::default()
    };
    xbro_paint::rasterize(&l, no_fonts(), &NoImageProvider, &mut p, &options);
    assert_px(&p, 5, 5, BLUE);
}

#[test]
fn text_draws_dark_pixels_inside_its_box_only() {
    let Some(fonts) = fonts() else { return };
    let frag = text(fonts, 4, "Hello", 10.0, 30.0, 20.0, style(|_| {}));
    let bounds = frag.rect;
    let mut para = block(3, r(0.0, 0.0, 200.0, 50.0), style(|_| {}));
    para.lines = vec![line(
        r(0.0, 0.0, 200.0, 50.0),
        30.0,
        vec![Fragment::Text(frag)],
    )];
    let p = render(
        &build_display_list(&page(Size::new(200.0, 50.0), vec![para])),
        200,
        50,
    );
    let x0 = bounds.x as u32;
    let x1 = bounds.right().ceil() as u32 + 1;
    let y0 = bounds.y as u32;
    let y1 = bounds.bottom().ceil() as u32 + 1;
    let inside = dark_pixels(&p, x0, y0, x1, y1, 128);
    let total = dark_pixels(&p, 0, 0, 200, 50, 250);
    let inside_any = dark_pixels(
        &p,
        x0.saturating_sub(1),
        y0.saturating_sub(1),
        x1 + 1,
        y1 + 1,
        250,
    );
    assert!(inside > 40, "glyph ink expected, got {inside}");
    assert_eq!(total, inside_any, "no ink outside the text box");
}

#[test]
fn text_respects_color_and_scale() {
    let Some(fonts) = fonts() else { return };
    let run = |color| {
        let frag = text(fonts, 1, "MMMM", 0.0, 20.0, 20.0, style(|_| {}));
        DisplayItem::Text(GlyphRun {
            font: frag.font,
            font_size: frag.font_size,
            origin: Point::new(0.0, 20.0),
            glyphs: frag.glyphs,
            color,
        })
    };
    let l = list(WHITE, vec![run(RED)]);
    let p = render(&l, 100, 30);
    let reds = (0..100)
        .flat_map(|x| (0..30).map(move |y| (x, y)))
        .filter(|&(x, y)| close(px(&p, x, y), rgb(RED), 8))
        .count();
    assert!(reds > 50, "{reds}");
    // At 2x the ink area roughly quadruples.
    let options = RasterOptions {
        scale: 2.0,
        ..RasterOptions::default()
    };
    let p2 = render_with(
        &list(WHITE, vec![run(BLACK)]),
        200,
        60,
        &options,
        &NoImageProvider,
    );
    let p1 = render(&list(WHITE, vec![run(BLACK)]), 100, 30);
    let (a1, a2) = (
        dark_pixels(&p1, 0, 0, 100, 30, 128),
        dark_pixels(&p2, 0, 0, 200, 60, 128),
    );
    assert!(
        a2 as f32 > a1 as f32 * 3.0 && (a2 as f32) < a1 as f32 * 5.0,
        "{a1} vs {a2}"
    );
}

#[test]
fn text_is_clipped() {
    let Some(fonts) = fonts() else { return };
    let frag = text(fonts, 1, "WWWWWWWW", 0.0, 20.0, 20.0, style(|_| {}));
    let items = vec![
        DisplayItem::PushClip {
            rect: r(0.0, 0.0, 30.0, 30.0),
            radii: [0.0; 4],
        },
        DisplayItem::Text(GlyphRun {
            font: frag.font,
            font_size: 20.0,
            origin: Point::new(0.0, 20.0),
            glyphs: frag.glyphs.clone(),
            color: BLACK,
        }),
        DisplayItem::PopClip,
    ];
    let p = render(&list(WHITE, items), 150, 30);
    assert!(dark_pixels(&p, 0, 0, 30, 30, 128) > 20);
    assert_eq!(dark_pixels(&p, 30, 0, 150, 30, 250), 0);
}

#[test]
fn underline_is_painted_below_the_baseline() {
    let Some(fonts) = fonts() else { return };
    let deco = style(|s| {
        s.decorations_in_effect = TextDecorationLine {
            underline: true,
            ..TextDecorationLine::NONE
        };
        s.decoration_color = BLUE;
        s.color = BLACK;
    });
    let frag = text(fonts, 4, "x x", 10.0, 30.0, 20.0, deco);
    let width = frag.rect.width;
    let mut para = block(3, r(0.0, 0.0, 200.0, 50.0), style(|_| {}));
    para.lines = vec![line(
        r(0.0, 0.0, 200.0, 50.0),
        30.0,
        vec![Fragment::Text(frag)],
    )];
    let tree = page(Size::new(200.0, 50.0), vec![para]);
    let p = render(
        &build_display_list_full(&tree, fonts, &NoImageProvider),
        200,
        50,
    );
    let row = (30..36)
        .find(|&y| close(px(&p, 12, y), rgb(BLUE), 30))
        .expect("underline row");
    // Spans the whole fragment, including the space.
    let blue = (10..(10 + width as u32))
        .filter(|&x| close(px(&p, x, row), rgb(BLUE), 30))
        .count();
    assert!(blue as f32 >= width - 2.0, "{blue} of {width}");
}

#[test]
fn gradient_endpoints() {
    let l = list(
        WHITE,
        vec![
            DisplayItem::LinearGradient {
                rect: r(0.0, 0.0, 100.0, 20.0),
                angle_deg: 90.0,
                stops: vec![(RED, 0.0), (BLUE, 1.0)],
                radii: [0.0; 4],
            },
            DisplayItem::LinearGradient {
                rect: r(0.0, 20.0, 20.0, 100.0),
                angle_deg: 180.0,
                stops: vec![(BLACK, 0.0), (WHITE, 1.0)],
                radii: [0.0; 4],
            },
        ],
    );
    let p = render(&l, 100, 120);
    assert!(close(px(&p, 0, 10), rgb(RED), 8), "{:?}", px(&p, 0, 10));
    assert!(close(px(&p, 99, 10), rgb(BLUE), 8), "{:?}", px(&p, 99, 10));
    let mid = px(&p, 50, 10);
    assert!(mid.0 > 100 && mid.2 > 100, "{mid:?}");
    assert!(close(px(&p, 10, 20), rgb(BLACK), 8));
    assert!(close(px(&p, 10, 119), rgb(WHITE), 8));
}

#[test]
fn gradient_to_transparent_has_no_grey_fringe() {
    let l = list(
        WHITE,
        vec![DisplayItem::LinearGradient {
            rect: r(0.0, 0.0, 100.0, 10.0),
            angle_deg: 90.0,
            stops: vec![(RED, 0.0), (Color::TRANSPARENT, 1.0)],
            radii: [0.0; 4],
        }],
    );
    let p = render(&l, 100, 10);
    let (r_, g, b, _) = px(&p, 50, 5);
    assert!(r_ > 250 && g.abs_diff(b) < 3 && g > 110, "{:?}", (r_, g, b));
}

#[test]
fn images_are_scaled_into_their_rect() {
    let mut images = MapImages::default();
    images.nodes.insert(7, checker(2, 2, RED, BLUE));
    let l = list(
        WHITE,
        vec![DisplayItem::NodeImage {
            rect: r(0.0, 0.0, 100.0, 100.0),
            node: NodeId(7),
        }],
    );
    let p = render_with(&l, 120, 100, &RasterOptions::default(), &images);
    assert!(close(px(&p, 10, 10), rgb(RED), 10), "{:?}", px(&p, 10, 10));
    assert!(close(px(&p, 90, 10), rgb(BLUE), 10), "{:?}", px(&p, 90, 10));
    assert!(close(px(&p, 10, 90), rgb(BLUE), 10));
    assert!(close(px(&p, 90, 90), rgb(RED), 10));
    assert_px(&p, 110, 50, WHITE);
    // Missing images draw nothing.
    let l = list(
        WHITE,
        vec![DisplayItem::NodeImage {
            rect: r(0.0, 0.0, 100.0, 100.0),
            node: NodeId(8),
        }],
    );
    assert_px(
        &render_with(&l, 10, 10, &RasterOptions::default(), &images),
        5,
        5,
        WHITE,
    );
}

#[test]
fn svg_images_render() {
    let svg = Image::from_svg_str(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><circle cx="10" cy="10" r="8" fill="lime"/></svg>"#,
    )
    .expect("svg");
    assert_eq!(svg.natural_size(), Size::new(20.0, 20.0));
    let mut images = MapImages::default();
    images.nodes.insert(1, svg.clone());
    images.urls.insert("icon.svg".into(), svg);
    let l = list(
        WHITE,
        vec![
            DisplayItem::NodeImage {
                rect: r(0.0, 0.0, 40.0, 40.0),
                node: NodeId(1),
            },
            DisplayItem::UrlImage {
                rect: r(50.0, 0.0, 20.0, 20.0),
                url: "icon.svg".into(),
            },
        ],
    );
    for _ in 0..2 {
        // Second pass hits the SVG cache.
        let p = render_with(&l, 80, 40, &RasterOptions::default(), &images);
        assert_px(&p, 20, 20, LIME);
        assert_px(&p, 1, 1, WHITE);
        assert_px(&p, 60, 10, LIME);
    }
}

#[test]
fn background_tiling_modes() {
    let mut images = MapImages::default();
    images.urls.insert("t.png".into(), solid_image(10, 10, RED));
    let bg = |y: f32, repeat| DisplayItem::BackgroundImage {
        url: "t.png".into(),
        clip: r(0.0, y, 100.0, 30.0),
        origin: Point::new(20.0, y + 5.0),
        tile: Size::new(10.0, 10.0),
        repeat,
    };
    let l = list(
        WHITE,
        vec![
            bg(0.0, BackgroundRepeat::Repeat),
            bg(40.0, BackgroundRepeat::RepeatX),
            bg(80.0, BackgroundRepeat::RepeatY),
            bg(120.0, BackgroundRepeat::NoRepeat),
        ],
    );
    let p = render_with(&l, 100, 160, &RasterOptions::default(), &images);
    // Repeat fills the clip.
    assert_px(&p, 2, 2, RED);
    assert_px(&p, 95, 28, RED);
    // Repeat-x: one row of tiles.
    assert_px(&p, 2, 47, RED);
    assert_px(&p, 2, 42, WHITE);
    assert_px(&p, 50, 57, WHITE);
    // Repeat-y: one column.
    assert_px(&p, 25, 82, RED);
    assert_px(&p, 35, 90, WHITE);
    // No-repeat: a single tile.
    assert_px(&p, 25, 130, RED);
    assert_px(&p, 35, 130, WHITE);
    assert_px(&p, 25, 140, WHITE);
}

#[test]
fn background_tiles_follow_scale_and_pattern() {
    let mut images = MapImages::default();
    images.urls.insert("c.png".into(), checker(2, 2, RED, BLUE));
    let l = list(
        WHITE,
        vec![DisplayItem::BackgroundImage {
            url: "c.png".into(),
            clip: r(0.0, 0.0, 8.0, 8.0),
            origin: Point::new(0.0, 0.0),
            tile: Size::new(0.0, 0.0), // natural size
            repeat: BackgroundRepeat::Repeat,
        }],
    );
    let p = render_with(&l, 8, 8, &RasterOptions::default(), &images);
    assert_px(&p, 0, 0, RED);
    assert_px(&p, 1, 0, BLUE);
    assert_px(&p, 2, 0, RED);
    assert_px(&p, 3, 3, RED);
    assert_px(&p, 7, 6, BLUE);
}

#[test]
fn deferred_backgrounds_resolve_at_raster_time() {
    let mut images = MapImages::default();
    images
        .urls
        .insert("d.png".into(), solid_image(10, 10, BLUE));
    let tree_box = block(
        3,
        r(0.0, 0.0, 100.0, 50.0),
        style(|s| {
            s.background_image = Some(xbro_style::BackgroundImage::Url("d.png".into()));
            s.background_repeat = BackgroundRepeat::NoRepeat;
            s.background_size = BackgroundSize::Contain;
            s.background_position = (
                LengthPercentage::percent(50.0),
                LengthPercentage::percent(50.0),
            );
        }),
    );
    let l = build_display_list(&page(Size::new(100.0, 50.0), vec![tree_box]));
    assert!(matches!(
        l.items[0],
        DisplayItem::DeferredBackgroundImage { .. }
    ));
    let p = render_with(&l, 100, 50, &RasterOptions::default(), &images);
    // Contain → 50×50 tile centred: x 25..75.
    assert_px(&p, 26, 25, BLUE);
    assert_px(&p, 74, 25, BLUE);
    assert_px(&p, 20, 25, WHITE);
    assert_px(&p, 80, 25, WHITE);
}

#[test]
fn checkbox_and_radio_controls() {
    let control = |node, x: f32, kind| {
        let mut b = block(node, r(x, 0.0, 13.0, 13.0), style(|_| {}));
        b.kind = BoxKind::Control(kind);
        b
    };
    let tree = page(
        Size::new(100.0, 20.0),
        vec![
            control(3, 0.0, ControlKind::Checkbox { checked: true }),
            control(4, 20.0, ControlKind::Checkbox { checked: false }),
            control(5, 40.0, ControlKind::Radio { checked: true }),
            control(6, 60.0, ControlKind::Range { value: 1.0 }),
        ],
    );
    let p = render(&build_display_list(&tree), 100, 20);
    let accent = Color::rgb(0, 0x75, 0xff);
    assert_px(&p, 1, 1, accent);
    assert_px(&p, 11, 11, accent);
    // Check mark: white pixels inside the blue box.
    let whites = (2..11)
        .flat_map(|x| (2..11).map(move |y| (x, y)))
        .filter(|&(x, y)| close(px(&p, x, y), rgb(WHITE), 40))
        .count();
    assert!(whites > 5, "{whites}");
    // Unchecked: grey border, white inside.
    assert_px(&p, 26, 0, Color::rgb(0x76, 0x76, 0x76));
    assert_px(&p, 26, 6, WHITE);
    // Radio: dot in the centre, white ring gap.
    assert_px(&p, 46, 6, accent);
    // Range at 100%: the thumb sits at the right end.
    assert!(close(px(&p, 67, 6), rgb(accent), 30));
}

#[test]
fn list_markers_are_painted() {
    let mut li = block(3, r(20.0, 0.0, 80.0, 20.0), style(|_| {}));
    li.marker = Some(ListMarker {
        rect: r(4.0, 4.0, 8.0, 8.0),
        color: RED,
        content: MarkerContent::Square,
    });
    let mut li2 = block(4, r(20.0, 20.0, 80.0, 20.0), style(|_| {}));
    li2.marker = Some(ListMarker {
        rect: r(2.0, 22.0, 12.0, 12.0),
        color: BLUE,
        content: MarkerContent::Disc,
    });
    let p = render(
        &build_display_list(&page(Size::new(100.0, 40.0), vec![li, li2])),
        100,
        40,
    );
    assert_px(&p, 8, 8, RED);
    assert_px(&p, 8, 28, BLUE);
    assert_px(&p, 2, 22, WHITE);
}

#[test]
fn ellipses_polylines_polygons() {
    let l = list(
        WHITE,
        vec![
            DisplayItem::Ellipse {
                rect: r(0.0, 0.0, 20.0, 20.0),
                color: RED,
                fill: true,
                stroke_width: 0.0,
            },
            DisplayItem::Ellipse {
                rect: r(30.0, 0.0, 20.0, 20.0),
                color: BLUE,
                fill: false,
                stroke_width: 2.0,
            },
            DisplayItem::Polyline {
                points: vec![Point::new(60.0, 10.0), Point::new(80.0, 10.0)],
                width: 4.0,
                color: BLACK,
            },
            DisplayItem::Polygon {
                points: vec![
                    Point::new(85.0, 0.0),
                    Point::new(100.0, 0.0),
                    Point::new(92.5, 15.0),
                ],
                color: LIME,
            },
            DisplayItem::Polygon {
                points: vec![],
                color: LIME,
            },
            DisplayItem::Polyline {
                points: vec![Point::new(f32::NAN, 0.0)],
                width: f32::INFINITY,
                color: BLACK,
            },
        ],
    );
    let p = render(&l, 100, 20);
    assert_px(&p, 10, 10, RED);
    assert_px(&p, 0, 0, WHITE);
    assert_px(&p, 40, 0, BLUE);
    assert_px(&p, 40, 10, WHITE);
    assert_px(&p, 70, 10, BLACK);
    assert_px(&p, 92, 3, LIME);
}

#[test]
fn draw_text_helper() {
    let Some(fonts) = fonts() else { return };
    let mut p = tiny_skia::Pixmap::new(100, 30).unwrap();
    p.fill(tiny_skia::Color::WHITE);
    let font = sans(fonts);
    let advance = draw_text(&mut p, fonts, font, 16.0, 2.0, 20.0, "Tab", BLACK);
    assert!(advance > 15.0 && advance < 40.0, "{advance}");
    assert!(dark_pixels(&p, 0, 0, 100, 30, 128) > 20);
    assert_eq!(dark_pixels(&p, 50, 0, 100, 30, 250), 0);
    // Degenerate input.
    assert_eq!(
        draw_text(&mut p, fonts, font, f32::NAN, 0.0, 0.0, "x", BLACK),
        0.0
    );
    let _ = draw_text(&mut p, fonts, font, 16.0, f32::INFINITY, -1e9, "x", BLACK);
}

#[test]
fn culled_items_and_huge_geometry() {
    let l = list(
        WHITE,
        vec![
            rect(-9999.0, -9999.0, 10.0, 10.0, RED),
            DisplayItem::Rect {
                rect: r(-1e9, -1e9, 2e9, 2e9),
                color: BLUE,
                radii: [1e6; 4],
            },
            DisplayItem::Border {
                rect: r(-1e7, 5.0, 2e7, 1e7),
                sides: [side(2.0, BorderStyle::Dashed, BLACK); 4],
                radii: [0.0; 4],
            },
            DisplayItem::LinearGradient {
                rect: r(0.0, -1e8, 10.0, 2e8),
                angle_deg: 0.0,
                stops: vec![(RED, 0.0), (LIME, 1.0)],
                radii: [3.0; 4],
            },
        ],
    );
    let p = render(&l, 50, 50);
    // The gradient covers x 0..10; the huge rect fills the rest.
    assert_px(&p, 30, 30, BLUE);
    // The huge dashed border's top side runs along y = 5..7: dashes and
    // gaps (their phase follows the whole 2e7 px side, not the viewport).
    let black = (12..50)
        .filter(|&x| close(px(&p, x, 6), rgb(BLACK), 40))
        .count();
    assert!((10..=28).contains(&black), "dashed top side: {black}");
}

#[test]
fn large_scale_and_odd_targets() {
    let l = list(WHITE, vec![rect(0.0, 0.0, 1.0, 1.0, RED)]);
    let options = RasterOptions {
        scale: 1000.0,
        ..RasterOptions::default()
    };
    let p = render_with(&l, 10, 10, &options, &NoImageProvider);
    assert_px(&p, 9, 9, RED);
    let p = render_with(&l, 1, 1, &RasterOptions::default(), &NoImageProvider);
    assert_px(&p, 0, 0, RED);
}

#[test]
fn image_tiles_and_svg_are_safe_with_degenerate_sizes() {
    let mut images = MapImages::default();
    images.urls.insert("t.png".into(), solid_image(1, 1, RED));
    images.urls.insert(
        "s.svg".into(),
        Image::from_svg_str(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="1"/>"#,
        )
        .unwrap(),
    );
    let bg = |tile: Size, url: &str| DisplayItem::BackgroundImage {
        url: url.into(),
        clip: r(0.0, 0.0, 50.0, 50.0),
        origin: Point::new(f32::NAN, 1e30),
        tile,
        repeat: BackgroundRepeat::Repeat,
    };
    let l = list(
        WHITE,
        vec![
            bg(Size::new(0.001, 0.001), "t.png"),
            bg(Size::new(f32::INFINITY, 5.0), "t.png"),
            bg(Size::new(100000.0, 1.0), "s.svg"),
            DisplayItem::UrlImage {
                rect: r(0.0, 0.0, 1e7, 1e7),
                url: "s.svg".into(),
            },
        ],
    );
    let _ = render_with(&l, 50, 50, &RasterOptions::default(), &images);
}

#[test]
fn transparent_and_arc_shared_images() {
    // An image with alpha composites over the background.
    let half = Arc::new({
        let mut p = tiny_skia::Pixmap::new(1, 1).unwrap();
        p.data_mut().copy_from_slice(&[0, 0, 128, 128]);
        p
    });
    let mut images = MapImages::default();
    images.nodes.insert(1, Image::Raster(half));
    let l = list(
        WHITE,
        vec![DisplayItem::NodeImage {
            rect: r(0.0, 0.0, 10.0, 10.0),
            node: NodeId(1),
        }],
    );
    let p = render_with(&l, 10, 10, &RasterOptions::default(), &images);
    assert!(
        close(px(&p, 5, 5), (127, 127, 255, 255), 3),
        "{:?}",
        px(&p, 5, 5)
    );
}

#[test]
fn reused_opacity_layers_leave_no_stale_pixels() {
    // Consecutive groups share one pooled layer: a later group must not
    // composite an earlier group's pixels again. (Groups hold two items:
    // single-item groups are drawn without a layer.)
    let tiny = |x: f32| rect(x, 19.0, 1.0, 1.0, Color::rgba(0, 0, 0, 1));
    let items = vec![
        DisplayItem::PushOpacity(0.5),
        rect(0.0, 0.0, 10.0, 10.0, RED),
        tiny(0.0),
        DisplayItem::PopOpacity,
        DisplayItem::PushOpacity(0.5),
        rect(20.0, 0.0, 10.0, 10.0, BLUE),
        tiny(20.0),
        DisplayItem::PopOpacity,
        DisplayItem::PushOpacity(0.5),
        DisplayItem::Ellipse {
            rect: r(0.0, 11.0, 8.0, 8.0),
            color: BLUE,
            fill: true,
            stroke_width: 0.0,
        },
        tiny(30.0),
        DisplayItem::PopOpacity,
        // Dirty over the whole area: stale pixels would be composited again.
        DisplayItem::PushOpacity(0.5),
        rect(0.0, 0.0, 40.0, 20.0, Color::rgba(0, 0, 0, 1)),
        tiny(39.0),
        DisplayItem::PopOpacity,
    ];
    let p = render(&list(WHITE, items), 40, 20);
    assert!(
        close(px(&p, 5, 5), (255, 127, 127, 255), 2),
        "{:?}",
        px(&p, 5, 5)
    );
    assert!(
        close(px(&p, 25, 5), (127, 127, 255, 255), 2),
        "{:?}",
        px(&p, 25, 5)
    );
    assert!(
        close(px(&p, 4, 15), (127, 127, 255, 255), 3),
        "{:?}",
        px(&p, 4, 15)
    );
    assert_px(&p, 15, 5, WHITE);
}

#[test]
fn single_item_groups_match_layered_groups() {
    let Some(fonts) = fonts() else { return };
    let t = text(fonts, 1, "Hg", 4.0, 30.0, 24.0, style(|_| {}));
    let run = DisplayItem::Text(GlyphRun {
        font: t.font,
        font_size: t.font_size,
        origin: Point::new(4.0, 30.0),
        glyphs: t.glyphs,
        color: BLUE,
    });
    let shapes = [
        rect(2.0, 2.0, 30.0, 10.0, RED),
        DisplayItem::Rect {
            rect: r(2.0, 2.0, 30.0, 30.0),
            color: Color::rgba(0, 128, 0, 200),
            radii: [8.0; 4],
        },
        DisplayItem::Ellipse {
            rect: r(2.0, 2.0, 30.0, 30.0),
            color: BLUE,
            fill: false,
            stroke_width: 3.0,
        },
        DisplayItem::Polyline {
            points: vec![
                Point::new(2.0, 2.0),
                Point::new(30.0, 30.0),
                Point::new(30.0, 2.0),
            ],
            width: 4.0,
            color: RED,
        },
        DisplayItem::Polygon {
            points: vec![
                Point::new(2.0, 2.0),
                Point::new(30.0, 30.0),
                Point::new(2.0, 30.0),
            ],
            color: BLUE,
        },
        run,
    ];
    for shape in shapes {
        let fast = list(
            WHITE,
            vec![
                DisplayItem::PushOpacity(0.4),
                shape.clone(),
                DisplayItem::PopOpacity,
            ],
        );
        // An empty clip pair forces the layered path.
        let layered = list(
            WHITE,
            vec![
                DisplayItem::PushOpacity(0.4),
                shape.clone(),
                DisplayItem::PushClip {
                    rect: r(0.0, 0.0, 1.0, 1.0),
                    radii: [0.0; 4],
                },
                DisplayItem::PopClip,
                DisplayItem::PopOpacity,
            ],
        );
        let (a, b) = (render(&fast, 40, 40), render(&layered, 40, 40));
        let worst = a
            .data()
            .iter()
            .zip(b.data())
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        assert!(worst <= 3, "{shape:?}: max channel difference {worst}");
        assert!(a.data().iter().any(|&v| v < 250), "{shape:?} drew nothing");
    }
}

#[test]
fn text_in_reused_layers_is_not_duplicated() {
    let Some(fonts) = fonts() else { return };
    let run = |x: f32| {
        let t = text(fonts, 1, "Hg", x, 30.0, 24.0, style(|_| {}));
        DisplayItem::Text(GlyphRun {
            font: t.font,
            font_size: t.font_size,
            origin: Point::new(x, 30.0),
            glyphs: t.glyphs,
            color: BLACK,
        })
    };
    // Two items per group: single-item groups are drawn without a layer.
    let tiny = rect(119.0, 39.0, 1.0, 1.0, Color::rgba(0, 0, 0, 1));
    let items = vec![
        DisplayItem::PushOpacity(0.5),
        run(0.0),
        tiny.clone(),
        DisplayItem::PopOpacity,
        DisplayItem::PushOpacity(0.5),
        run(60.0),
        tiny,
        DisplayItem::PopOpacity,
    ];
    let p = render(&list(WHITE, items), 120, 40);
    // Half-opaque black text never gets darker than ~50% grey.
    let darkest = (0..120)
        .flat_map(|x| (0..40).map(move |y| (x, y)))
        .map(|(x, y)| px(&p, x, y).0)
        .min()
        .unwrap_or(255);
    assert!((120..=135).contains(&darkest), "darkest {darkest}");
}

#[test]
fn deeply_nested_opacity_is_bounded() {
    // 300 nested groups on a 2000×2000 area would need 4.8 GB of layers;
    // the budget caps live layers and the drawing still happens.
    let mut items = Vec::new();
    for _ in 0..300 {
        items.push(DisplayItem::PushOpacity(0.999));
    }
    items.push(rect(0.0, 0.0, 10.0, 10.0, RED));
    for _ in 0..300 {
        items.push(DisplayItem::PopOpacity);
    }
    let start = std::time::Instant::now();
    let p = render(&list(WHITE, items), 2000, 2000);
    let (r_, g, b, _) = px(&p, 5, 5);
    assert!(r_ > 200 && g < 100 && b < 100, "{:?}", (r_, g, b));
    assert_px(&p, 50, 50, WHITE);
    assert!(start.elapsed() < std::time::Duration::from_secs(10));
}

#[test]
fn rounded_clip_inside_and_across_corners() {
    let Some(fonts) = fonts() else { return };
    let t = text(fonts, 1, "Wide text", 20.0, 50.0, 16.0, style(|_| {}));
    let items = vec![
        DisplayItem::PushClip {
            rect: r(0.0, 0.0, 100.0, 100.0),
            radii: [30.0; 4],
        },
        // Fully inside the cross between the corners: drawn without a mask.
        DisplayItem::Text(GlyphRun {
            font: t.font,
            font_size: t.font_size,
            origin: Point::new(20.0, 50.0),
            glyphs: t.glyphs,
            color: BLACK,
        }),
        rect(40.0, 0.0, 20.0, 100.0, BLUE),
        // Across every corner: masked.
        rect(0.0, 0.0, 100.0, 10.0, RED),
        rect(0.0, 90.0, 100.0, 10.0, RED),
        DisplayItem::PopClip,
    ];
    let p = render(&list(WHITE, items), 110, 110);
    assert!(dark_pixels(&p, 20, 35, 100, 55, 100) > 30);
    assert_px(&p, 50, 50, BLUE);
    assert_px(&p, 50, 5, RED);
    assert_px(&p, 50, 95, RED);
    for (x, y) in [(1, 1), (98, 1), (98, 98), (1, 98), (5, 5)] {
        assert_px(&p, x, y, WHITE);
    }
    assert_px(&p, 105, 50, WHITE);
}

#[test]
fn deepest_accepted_svg_renders_on_small_stacks() {
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(|| {
            let mut s =
                String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">"#);
            s.push_str(&"<g opacity='0.999'>".repeat(255));
            s.push_str(r#"<rect width="10" height="10" fill="lime"/>"#);
            s.push_str(&"</g>".repeat(255));
            s.push_str("</svg>");
            let image = Image::from_svg_str(&s).expect("256 levels are accepted");
            let mut images = MapImages::default();
            images.nodes.insert(1, image);
            let l = list(
                WHITE,
                vec![DisplayItem::NodeImage {
                    rect: r(0.0, 0.0, 20.0, 20.0),
                    node: NodeId(1),
                }],
            );
            let p = render_with(&l, 20, 20, &RasterOptions::default(), &images);
            let (r_, g, b, _) = px(&p, 10, 10);
            assert!(g > 200 && r_ < 100 && b < 100, "{:?}", (r_, g, b));
        })
        .expect("thread")
        .join()
        .expect("no stack overflow");
}
