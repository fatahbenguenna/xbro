//! Regression tests for problems found while reviewing the crate: each
//! test failed before its fix.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::*;
use xbro_dom::NodeId;
use xbro_layout::{BoxKind, Fragment, LayoutTree, Rect, Size};
use xbro_paint::{
    build_display_list, draw_text, BorderSideItem, DisplayItem, DisplayList, Image, ImageProvider,
    RasterOptions,
};
use xbro_style::{BorderStyle, Color, Content, ContentItem};

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(x, y, w, h)
}

fn list(background: Color, items: Vec<DisplayItem>) -> DisplayList {
    DisplayList { items, background }
}

fn side(width: f32, style: BorderStyle, color: Color) -> BorderSideItem {
    BorderSideItem {
        width,
        style,
        color,
    }
}

fn url_images(list: &DisplayList) -> Vec<(Rect, String)> {
    list.items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::UrlImage { rect, url } => Some((*rect, url.clone())),
            _ => None,
        })
        .collect()
}

/// A node-less inline image box (as layout builds for `content: url()`
/// and inside `list-style-image` markers) inside a line of `block`.
fn page_with_atomic(atomic: xbro_layout::LayoutBox) -> LayoutTree {
    let mut host = block(3, r(0.0, 0.0, 100.0, 40.0), style(|_| {}));
    host.lines = vec![line(
        r(0.0, 0.0, 100.0, 40.0),
        30.0,
        vec![Fragment::Atomic(atomic)],
    )];
    page(Size::new(100.0, 40.0), vec![host])
}

fn image_box(content: Rect, s: Arc<xbro_style::ComputedStyle>) -> xbro_layout::LayoutBox {
    let mut b = block(0, content, s);
    b.node = None;
    b.kind = BoxKind::Image;
    b
}

#[test]
fn content_url_images_are_painted() {
    let s = style(|s| {
        s.content = Content::Items(vec![ContentItem::Image("img/star.png".into())]);
    });
    let tree = page_with_atomic(image_box(r(10.0, 10.0, 16.0, 16.0), s));
    let l = build_display_list(&tree);
    assert_eq!(
        url_images(&l),
        vec![(r(10.0, 10.0, 16.0, 16.0), "img/star.png".to_string())]
    );
    let mut images = MapImages::default();
    images
        .urls
        .insert("img/star.png".into(), solid_image(4, 4, RED));
    let p = render_with(&l, 100, 40, &RasterOptions::default(), &images);
    assert_px(&p, 18, 18, RED);
    assert_px(&p, 40, 18, WHITE);
}

#[test]
fn inside_list_style_image_boxes_are_painted() {
    let s = style(|s| s.list_style_image = Some(Arc::from("bullet.png")));
    let tree = page_with_atomic(image_box(r(2.0, 20.0, 8.0, 8.0), s));
    let l = build_display_list(&tree);
    assert_eq!(
        url_images(&l),
        vec![(r(2.0, 20.0, 8.0, 8.0), "bullet.png".to_string())]
    );
    // Hidden or URL-less image boxes paint nothing.
    let hidden = style(|s| {
        s.list_style_image = Some(Arc::from("bullet.png"));
        s.visibility = xbro_style::Visibility::Hidden;
    });
    let tree = page_with_atomic(image_box(r(2.0, 20.0, 8.0, 8.0), hidden));
    assert!(url_images(&build_display_list(&tree)).is_empty());
    let tree = page_with_atomic(image_box(r(2.0, 20.0, 8.0, 8.0), style(|_| {})));
    assert!(url_images(&build_display_list(&tree)).is_empty());
}

#[test]
fn draw_text_far_from_the_target_does_not_overflow() {
    let Some(f) = fonts() else { return };
    let font = sans(f);
    let mut p = tiny_skia::Pixmap::new(40, 40).expect("pixmap");
    let expected = draw_text(&mut p, f, font, 16.0, 0.0, 20.0, "_,gj", BLACK);
    assert!(expected > 0.0);
    for (x, baseline) in [
        (0.0, 1.0e10),
        (0.0, -1.0e10),
        (0.0, 2_147_483_600.0),
        (1.0e10, 20.0),
        (-1.0e10, 20.0),
    ] {
        let mut q = tiny_skia::Pixmap::new(40, 40).expect("pixmap");
        let w = draw_text(&mut q, f, font, 16.0, x, baseline, "_,gj", BLACK);
        assert_eq!(w, expected, "the advance does not depend on the position");
        assert!(q.data().iter().all(|&b| b == 0), "nothing drawn");
    }
}

/// Counts provider lookups.
#[derive(Default)]
struct CountingImages {
    lookups: AtomicUsize,
}

impl ImageProvider for CountingImages {
    fn node_image(&self, _: NodeId) -> Option<Image> {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        Some(solid_image(2, 2, RED))
    }
    fn url_image(&self, _: &str) -> Option<Image> {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        Some(solid_image(2, 2, RED))
    }
}

#[test]
fn off_screen_images_are_culled_before_lookup() {
    let mut items = Vec::new();
    for i in 0..50 {
        let y = 1000.0 + 30.0 * i as f32;
        items.push(DisplayItem::NodeImage {
            rect: r(0.0, y, 20.0, 20.0),
            node: NodeId(i),
        });
        items.push(DisplayItem::UrlImage {
            rect: r(30.0, y, 20.0, 20.0),
            url: format!("u{i}"),
        });
    }
    items.push(DisplayItem::NodeImage {
        rect: r(0.0, 0.0, 20.0, 20.0),
        node: NodeId(999),
    });
    let images = CountingImages::default();
    let p = render_with(
        &list(WHITE, items),
        60,
        60,
        &RasterOptions::default(),
        &images,
    );
    assert_px(&p, 10, 10, RED);
    assert_eq!(images.lookups.load(Ordering::Relaxed), 1);
}

/// Column `x` of rows `y0..y1` (rgb only).
fn column(p: &tiny_skia::Pixmap, x: u32, y0: u32, y1: u32) -> Vec<(u8, u8, u8, u8)> {
    (y0..y1).map(|y| px(p, x, y)).collect()
}

/// Renders a 6000px tall box with dashed/dotted borders at two scroll
/// positions 3px apart: the pattern must move with the content.
fn assert_pattern_follows_scroll(style: BorderStyle, radius: f32) {
    let sides = [side(4.0, style, BLUE); 4];
    let l = list(
        WHITE,
        vec![DisplayItem::Border {
            rect: r(10.0, -2000.0, 100.0, 6000.0),
            sides,
            radii: [radius; 4],
        }],
    );
    let at = |scroll_y: f32| {
        render_with(
            &l,
            120,
            300,
            &RasterOptions {
                scroll: xbro_layout::Point::new(0.0, scroll_y),
                ..RasterOptions::default()
            },
            &no_images(),
        )
    };
    let a = at(0.0);
    let b = at(3.0);
    let col_a = column(&a, 12, 50, 250);
    let col_b = column(&b, 12, 47, 247);
    let ink = col_a.iter().filter(|c| c.2 > 200 && c.0 < 100).count();
    assert!(ink > 40 && ink < 190, "{style:?}: a dashed pattern ({ink})");
    let differing = col_a
        .iter()
        .zip(&col_b)
        .filter(|(p, q)| !close(**p, **q, 24))
        .count();
    assert!(
        differing <= 4,
        "{style:?} r={radius}: pattern moved with the content ({differing} rows differ)"
    );
}

fn no_images() -> xbro_paint::NoImageProvider {
    xbro_paint::NoImageProvider
}

#[test]
fn dashed_borders_of_tall_boxes_scroll_with_the_content() {
    assert_pattern_follows_scroll(BorderStyle::Dashed, 0.0);
    assert_pattern_follows_scroll(BorderStyle::Dotted, 0.0);
}

#[test]
fn rounded_dashed_borders_of_tall_boxes_scroll_with_the_content() {
    assert_pattern_follows_scroll(BorderStyle::Dashed, 12.0);
    assert_pattern_follows_scroll(BorderStyle::Dotted, 12.0);
}

#[test]
fn dashed_side_of_a_rounded_border_stays_in_the_ring() {
    let l = list(
        WHITE,
        vec![DisplayItem::Border {
            rect: r(10.0, 10.0, 120.0, 70.0),
            sides: [
                side(6.0, BorderStyle::Dashed, RED),
                side(6.0, BorderStyle::Solid, BLUE),
                side(6.0, BorderStyle::Solid, BLUE),
                side(6.0, BorderStyle::Solid, BLUE),
            ],
            radii: [30.0; 4],
        }],
    );
    let p = render(&l, 140, 90);
    // Outside the rounded corners: untouched.
    for (x, y) in [(12, 13), (14, 12), (127, 13), (125, 12)] {
        assert_px(&p, x, y, WHITE);
    }
    // The dashes still paint along the straight part of the top side.
    let red = (40..100)
        .filter(|&x| {
            let c = px(&p, x, 13);
            c.0 > 200 && c.1 < 80
        })
        .count();
    assert!(red > 15, "{red}");
}

#[test]
fn dashed_border_far_off_screen_paints_nothing() {
    // A border whose sides all lie outside the area, around it.
    let l = list(
        WHITE,
        vec![DisplayItem::Border {
            rect: r(-500.0, -500.0, 2000.0, 2000.0),
            sides: [side(3.0, BorderStyle::Dashed, RED); 4],
            radii: [0.0; 4],
        }],
    );
    let p = render(&l, 100, 100);
    assert!((0..100).all(|x| px(&p, x, 0) == (255, 255, 255, 255)));
}

/// Percent-encodes markup for a `data:` URL.
fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/=:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// An SVG drawing a lime square through an `<image>` whose `data:` URL
/// holds a nested SVG with `depth` levels of elements.
fn svg_with_nested_image(depth: usize) -> String {
    let mut inner =
        String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">"#);
    inner.push_str(&"<g>".repeat(depth - 1));
    inner.push_str(r#"<rect width="10" height="10" fill="lime"/>"#);
    inner.push_str(&"</g>".repeat(depth - 1));
    inner.push_str("</svg>");
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image width="10" height="10" href="data:image/svg+xml,{}"/></svg>"#,
        percent_encode(&inner)
    )
}

fn center_of(image: Image) -> (u8, u8, u8, u8) {
    let mut images = MapImages::default();
    images.nodes.insert(1, image);
    let l = list(
        WHITE,
        vec![DisplayItem::NodeImage {
            rect: r(0.0, 0.0, 20.0, 20.0),
            node: NodeId(1),
        }],
    );
    px(
        &render_with(&l, 20, 20, &RasterOptions::default(), &images),
        10,
        10,
    )
}

#[test]
fn nested_data_url_svgs_get_the_structural_scan() {
    let shallow = Image::decode(svg_with_nested_image(8).as_bytes(), None).expect("svg");
    let (r_, g, b, _) = center_of(shallow);
    assert!(
        g > 200 && r_ < 80 && b < 80,
        "nested svg drawn: {:?}",
        (r_, g, b)
    );
    // Beyond the nesting limit the nested document is dropped (the outer
    // one still decodes).
    let deep = Image::decode(svg_with_nested_image(300).as_bytes(), None).expect("outer svg");
    assert_eq!(center_of(deep), (255, 255, 255, 255));
}

#[test]
fn chained_clip_paths_and_masks_are_bounded() {
    let chain = |n: usize, element: &str, attribute: &str| {
        let mut s = String::from(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><defs>"#,
        );
        for i in 0..n {
            s.push_str(&format!(
                r#"<{element} id="d{i}" {attribute}="url(#d{})"><rect width="10" height="10" fill="white"/></{element}>"#,
                i + 1
            ));
        }
        s.push_str(&format!(
            r#"</defs><rect width="10" height="10" fill="lime" {attribute}="url(#d0)"/></svg>"#
        ));
        s
    };
    for (element, attribute) in [("clipPath", "clip-path"), ("mask", "mask")] {
        assert!(
            Image::from_svg_str(&chain(4, element, attribute)).is_some(),
            "short {element} chains are fine"
        );
        assert!(
            Image::from_svg_str(&chain(40, element, attribute)).is_none(),
            "long {element} chains are rejected"
        );
        assert!(Image::decode(chain(40, element, attribute).as_bytes(), None).is_none());
    }
}

#[test]
fn entity_expansion_is_bounded() {
    let svg = |refs: usize| {
        format!(
            r#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "{}"><!ENTITY b "{}">]><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><desc>{}</desc><rect width="10" height="10" fill="lime"/></svg>"#,
            "x".repeat(2000),
            "&a;".repeat(100),
            "&b;".repeat(refs)
        )
    };
    assert!(Image::decode(svg(1).as_bytes(), Some("image/svg+xml")).is_some());
    assert!(Image::decode(svg(10).as_bytes(), Some("image/svg+xml")).is_none());
    assert!(Image::from_svg_str(&svg(10)).is_none());
}

#[test]
fn svg_decoding_and_rendering_work_on_tiny_caller_stacks() {
    // The 256-level limit holds whatever stack the caller has: parsing
    // and rendering run on their own large stack.
    std::thread::Builder::new()
        .stack_size(256 << 10)
        .spawn(|| {
            let mut s =
                String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">"#);
            s.push_str(&"<g opacity='0.999'>".repeat(255));
            s.push_str(r#"<rect width="10" height="10" fill="lime"/>"#);
            s.push_str(&"</g>".repeat(255));
            s.push_str("</svg>");
            let image = Image::decode(s.as_bytes(), None).expect("256 levels are accepted");
            let (r_, g, b, _) = center_of(image);
            assert!(g > 200 && r_ < 100 && b < 100, "{:?}", (r_, g, b));
        })
        .expect("thread")
        .join()
        .expect("no stack overflow");
}

#[test]
fn svgs_with_long_prologs_are_sniffed_without_a_content_type() {
    let svg = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- {} -->
<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="lime"/></svg>"#,
        "Licensed under the terms of a long license header. ".repeat(200)
    );
    let image = Image::decode(svg.as_bytes(), None).expect("sniffed as SVG");
    assert_eq!(image.natural_size(), Size::new(10.0, 10.0));
    let (r_, g, b, _) = center_of(image);
    assert!(g > 200 && r_ < 80 && b < 80, "{:?}", (r_, g, b));
}

/// Every coordinate of `item` is finite (glyph offsets aside).
fn assert_finite(item: &DisplayItem) {
    let rect_ok = |r: &Rect| {
        r.x.is_finite() && r.y.is_finite() && r.width.is_finite() && r.height.is_finite()
    };
    let ok = match item {
        DisplayItem::Rect { rect, radii, .. } | DisplayItem::LinearGradient { rect, radii, .. } => {
            rect_ok(rect) && radii.iter().all(|r| r.is_finite())
        }
        DisplayItem::Border { rect, sides, radii } => {
            rect_ok(rect)
                && radii.iter().all(|r| r.is_finite())
                && sides.iter().all(|s| s.width.is_finite())
        }
        DisplayItem::Text(run) => {
            run.origin.x.is_finite() && run.origin.y.is_finite() && run.font_size.is_finite()
        }
        DisplayItem::NodeImage { rect, .. }
        | DisplayItem::UrlImage { rect, .. }
        | DisplayItem::TextDecoration { rect, .. }
        | DisplayItem::PushClip { rect, .. } => rect_ok(rect),
        DisplayItem::BackgroundImage {
            clip, origin, tile, ..
        } => {
            rect_ok(clip)
                && origin.x.is_finite()
                && origin.y.is_finite()
                && tile.width.is_finite()
                && tile.height.is_finite()
        }
        DisplayItem::DeferredBackgroundImage { area, clip, .. } => rect_ok(area) && rect_ok(clip),
        DisplayItem::Ellipse {
            rect, stroke_width, ..
        } => rect_ok(rect) && stroke_width.is_finite(),
        DisplayItem::Polyline { points, width, .. } => {
            width.is_finite() && points.iter().all(|p| p.x.is_finite() && p.y.is_finite())
        }
        DisplayItem::Polygon { points, .. } => {
            points.iter().all(|p| p.x.is_finite() && p.y.is_finite())
        }
        DisplayItem::PushOpacity(o) => o.is_finite(),
        DisplayItem::PopClip
        | DisplayItem::PopOpacity
        | DisplayItem::PushFixed
        | DisplayItem::PopFixed => true,
    };
    assert!(ok, "non-finite geometry in {item:?}");
}

#[test]
fn hostile_trees_of_every_kind_stay_finite() {
    use xbro_layout::{ControlKind, ListMarker, MarkerContent};
    use xbro_style::{BackgroundImage, BackgroundSize, LengthPercentage, Overflow, Position};
    let nan = f32::NAN;
    let hostile = |s: &mut xbro_style::ComputedStyle| {
        s.background_color = RED;
        s.background_image = Some(BackgroundImage::Url("bg.png".into()));
        s.background_size = BackgroundSize::Explicit(
            Some(LengthPercentage::px(nan)),
            Some(LengthPercentage::percent(f32::INFINITY)),
        );
        s.background_position = (LengthPercentage::px(nan), LengthPercentage::px(-1e30));
        s.border = solid(f32::INFINITY, BLUE);
        s.border_radius = [LengthPercentage::px(nan); 4];
        s.outline_width = nan;
        s.outline_color = BLUE;
        s.font_size = nan;
        s.overflow_x = Overflow::Hidden;
        s.list_style_image = Some(Arc::from("bullet.png"));
    };
    let weird = r(nan, f32::INFINITY, f32::NEG_INFINITY, nan);
    let mut kids = Vec::new();
    for (i, kind) in [
        BoxKind::Block,
        BoxKind::Flex,
        BoxKind::Image,
        BoxKind::Placeholder,
        BoxKind::Control(ControlKind::TextField),
        BoxKind::Control(ControlKind::Select),
        BoxKind::Control(ControlKind::Checkbox { checked: true }),
        BoxKind::Control(ControlKind::Radio { checked: true }),
        BoxKind::Control(ControlKind::Range { value: nan }),
        BoxKind::Control(ControlKind::Progress {
            value: f32::INFINITY,
        }),
        BoxKind::Control(ControlKind::Color(RED)),
    ]
    .into_iter()
    .enumerate()
    {
        let mut b = block(10 + i, weird, style(hostile));
        b.kind = kind;
        if i % 2 == 0 {
            b.node = None;
        }
        b.padding = xbro_layout::Edges::all(nan);
        b.border = xbro_layout::Edges::all(f32::INFINITY);
        b.marker = Some(ListMarker {
            rect: weird,
            color: BLACK,
            content: [
                MarkerContent::Disc,
                MarkerContent::Circle,
                MarkerContent::Square,
            ][i % 3]
                .clone(),
        });
        kids.push(b);
    }
    // Stacking contexts with extreme z-indices and translucent fixed boxes.
    for (i, z) in [i32::MIN, -1, 0, 1, i32::MAX].into_iter().enumerate() {
        kids.push(block(
            100 + i,
            weird,
            style(|s| {
                hostile(s);
                s.position = if i == 4 {
                    Position::Fixed
                } else {
                    Position::Absolute
                };
                s.z_index = Some(z);
                s.opacity = 0.5;
            }),
        ));
    }
    let mut host = block(3, r(0.0, 0.0, 100.0, 100.0), style(|_| {}));
    let mut fragments = vec![Fragment::InlineBox(xbro_layout::InlineBoxFragment {
        node: NodeId(4),
        style: style(hostile),
        rect: weird,
        border: xbro_layout::Edges::all(nan),
        padding: xbro_layout::Edges::all(nan),
    })];
    if let Some(f) = fonts() {
        let mut t = text(
            f,
            5,
            "hostile",
            0.0,
            10.0,
            12.0,
            style(|s| {
                s.decorations_in_effect = xbro_style::TextDecorationLine {
                    underline: true,
                    overline: true,
                    line_through: true,
                };
                s.decoration_color = RED;
            }),
        );
        t.rect = weird;
        t.baseline = f32::INFINITY;
        fragments.push(Fragment::Text(t));
    }
    host.lines = vec![line(weird, nan, fragments)];
    kids.push(host);
    let mut tree = page(Size::new(100.0, 100.0), kids);
    tree.document_size = Size::new(nan, f32::INFINITY);
    let images = MapImages::default();
    for l in [
        build_display_list(&tree),
        xbro_paint::build_display_list_with_images(&tree, &images),
    ] {
        l.items.iter().for_each(assert_finite);
        for (scroll, scale) in [
            (xbro_layout::Point::new(nan, f32::INFINITY), 1.0),
            (xbro_layout::Point::new(0.0, 0.0), 3.0),
            (xbro_layout::Point::new(-1e30, 1e30), 0.25),
        ] {
            let options = RasterOptions {
                scroll,
                scale,
                ..RasterOptions::default()
            };
            let _ = render_with(&l, 64, 64, &options, &images);
        }
    }
}

#[test]
fn svg_images_keep_their_aspect_ratio() {
    // A 20×10 SVG drawn into a 40×40 box is letterboxed (preserveAspectRatio
    // defaults to `xMidYMid meet`), not stretched.
    let svg = Image::from_svg_str(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10" viewBox="0 0 20 10"><rect width="20" height="10" fill="lime"/></svg>"#,
    )
    .expect("svg");
    let mut images = MapImages::default();
    images.nodes.insert(1, svg);
    let l = list(
        WHITE,
        vec![DisplayItem::NodeImage {
            rect: r(0.0, 0.0, 40.0, 40.0),
            node: NodeId(1),
        }],
    );
    let p = render_with(&l, 40, 40, &RasterOptions::default(), &images);
    assert_px(&p, 20, 20, LIME);
    assert_px(&p, 2, 11, LIME);
    assert_px(&p, 20, 3, WHITE);
    assert_px(&p, 20, 36, WHITE);
}
