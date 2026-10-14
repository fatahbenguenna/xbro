//! Regression tests for the second review of the crate: paint order of
//! boxes moved by layout, rounded replaced content, sliced and translucent
//! inline elements, outlines, controls, EXIF orientation and image
//! minification. Each test failed before its fix.

mod common;

use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use common::*;
use xbro_css::{parse_stylesheet, MediaContext};
use xbro_layout::{
    layout, BoxKind, ControlKind, Edges, Fragment, LayoutBox, LayoutContext, NoImages, Pseudo,
    Rect, Size,
};
use xbro_paint::{
    build_display_list, build_display_list_full, rasterize, DisplayItem, DisplayList, Image,
    NoImageProvider, RasterOptions,
};
use xbro_style::{compute_styles, Color, ComputedStyle, LengthPercentage, Position, StyleContext};
use xbro_text::FontSystem;

const VIEW: Size = Size::new(400.0, 300.0);

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(x, y, w, h)
}

fn bg(color: Color) -> Arc<ComputedStyle> {
    style(|s| s.background_color = color)
}

/// Index of the first `Rect` item filled with `color`.
fn rect_index(list: &DisplayList, color: Color) -> usize {
    list.items
        .iter()
        .position(|i| matches!(i, DisplayItem::Rect { color: c, .. } if *c == color))
        .unwrap_or_else(|| panic!("no rect of {color:?} in {:#?}", list.items))
}

fn positioned(node: usize, position: Position, color: Color) -> LayoutBox {
    block(
        node,
        r(0.0, 0.0, 100.0, 50.0),
        style(|s| {
            s.position = position;
            s.background_color = color;
        }),
    )
}

fn system_fonts() -> &'static FontSystem {
    static FONTS: OnceLock<FontSystem> = OnceLock::new();
    FONTS.get_or_init(FontSystem::new)
}

/// HTML → style → layout → display list → pixels (no images).
fn render_html(html: &str, width: u32, height: u32) -> (DisplayList, tiny_skia::Pixmap) {
    let fonts = system_fonts();
    let doc = xbro_html::parse_document(html);
    let sheets: Vec<_> = doc
        .descendants(doc.root())
        .filter(|&id| doc.is_element_named(id, "style"))
        .map(|id| parse_stylesheet(&doc.text_content(id)))
        .collect();
    let viewport = Size::new(width as f32, height as f32);
    let ctx = StyleContext {
        media: MediaContext {
            width: viewport.width,
            height: viewport.height,
            ..MediaContext::default()
        },
        ..StyleContext::default()
    };
    let styles = compute_styles(&doc, &sheets, &ctx);
    let tree = layout(&LayoutContext {
        document: &doc,
        styles: &styles,
        fonts,
        replaced: &NoImages,
        viewport,
    });
    let list = build_display_list_full(&tree, fonts, &NoImageProvider);
    let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("pixmap");
    rasterize(
        &list,
        fonts,
        &NoImageProvider,
        &mut pixmap,
        &RasterOptions::default(),
    );
    (list, pixmap)
}

// ---------------------------------------------------------------------
// Paint order of positioned boxes moved by layout
// ---------------------------------------------------------------------

/// Layout appends absolutely positioned boxes to their containing block's
/// children, after its in-flow children: an overlay that comes first in
/// the document must still paint below later positioned content.
#[test]
fn moved_absolute_boxes_paint_in_document_order() {
    let overlay = positioned(4, Position::Absolute, RED);
    let content = positioned(5, Position::Relative, BLUE);
    // Box tree order: content (5) before the overlay (4).
    let cb = with_children(
        block(
            3,
            r(0.0, 0.0, 200.0, 100.0),
            style(|s| s.position = Position::Relative),
        ),
        vec![content, overlay],
    );
    let list = build_display_list(&page(VIEW, vec![cb]));
    assert!(
        rect_index(&list, RED) < rect_index(&list, BLUE),
        "the overlay (earlier in the document) paints first"
    );
}

/// Positive and negative layers with equal `z-index` follow the document
/// order too.
#[test]
fn equal_z_index_ties_follow_document_order() {
    let z = |node, color| {
        block(
            node,
            r(0.0, 0.0, 50.0, 50.0),
            style(|s| {
                s.position = Position::Absolute;
                s.z_index = Some(2);
                s.background_color = color;
            }),
        )
    };
    let list = build_display_list(&page(VIEW, vec![z(9, RED), z(7, BLUE)]));
    assert!(rect_index(&list, BLUE) < rect_index(&list, RED));
    let neg = |node, color| {
        block(
            node,
            r(0.0, 0.0, 50.0, 50.0),
            style(|s| {
                s.position = Position::Absolute;
                s.z_index = Some(-1);
                s.background_color = color;
            }),
        )
    };
    let list = build_display_list(&page(VIEW, vec![neg(9, LIME), neg(7, GREEN)]));
    assert!(rect_index(&list, GREEN) < rect_index(&list, LIME));
}

/// A positioned `::before` paints right after its element (below the
/// element's positioned children); a positioned `::after` paints after
/// the element's whole subtree, even when a later sibling's node id is
/// smaller than some descendant's.
#[test]
fn positioned_pseudo_elements_follow_their_element() {
    let pseudo = |which, color| {
        let mut b = positioned(3, Position::Absolute, color);
        b.pseudo = Some(which);
        b
    };
    let child = with_children(
        positioned(4, Position::Relative, BLUE),
        vec![block(8, r(0.0, 0.0, 1.0, 1.0), style(|_| {}))],
    );
    // Box tree order as layout builds it: children, then the moved
    // `::before` (absolute) and the `::after`.
    let host = with_children(
        block(
            3,
            r(0.0, 0.0, 200.0, 100.0),
            style(|s| s.position = Position::Relative),
        ),
        vec![
            child,
            pseudo(Pseudo::Before, RED),
            pseudo(Pseudo::After, LIME),
        ],
    );
    let later = positioned(9, Position::Relative, GREEN);
    let list = build_display_list(&page(VIEW, vec![host, later]));
    let (before, child, after, later) = (
        rect_index(&list, RED),
        rect_index(&list, BLUE),
        rect_index(&list, LIME),
        rect_index(&list, GREEN),
    );
    assert!(before < child, "::before below the element's children");
    assert!(child < after, "::after above the element's subtree");
    assert!(after < later, "::after below later content");
}

/// The hero overlay pattern end to end: an absolutely positioned overlay
/// followed by relatively positioned content.
#[test]
fn hero_overlay_stays_below_its_content() {
    let (_, p) = render_html(
        r#"<body style="margin:0">
           <div style="position:relative;height:100px">
             <div style="position:absolute;left:0;top:0;width:200px;height:100px;background:red"></div>
             <div style="position:relative;width:100px;height:50px;background:blue"></div>
           </div>
           <style>.h::before{content:"";position:absolute;left:0;top:0;width:200px;height:100px;background:red}</style>
           <div class="h" style="position:relative;height:100px">
             <div style="position:relative;width:100px;height:50px;background:blue"></div>
           </div></body>"#,
        300,
        220,
    );
    assert_px(&p, 50, 25, BLUE);
    assert_px(&p, 150, 75, RED);
    assert_px(&p, 50, 125, BLUE);
    assert_px(&p, 150, 175, RED);
}

/// `position: sticky` boxes create stacking contexts, as in browsers.
#[test]
fn sticky_boxes_are_stacking_contexts() {
    let inner = block(
        5,
        r(0.0, 0.0, 50.0, 50.0),
        style(|s| {
            s.position = Position::Absolute;
            s.z_index = Some(100);
            s.background_color = RED;
        }),
    );
    let sticky = with_children(
        block(
            4,
            r(0.0, 0.0, 100.0, 100.0),
            style(|s| s.position = Position::Sticky),
        ),
        vec![inner],
    );
    let above = block(
        6,
        r(0.0, 0.0, 100.0, 100.0),
        style(|s| {
            s.position = Position::Relative;
            s.z_index = Some(1);
            s.background_color = BLUE;
        }),
    );
    let list = build_display_list(&page(VIEW, vec![sticky, above]));
    // The z-index 100 child is confined to the sticky box's context (z 0).
    assert!(rect_index(&list, RED) < rect_index(&list, BLUE));
}

/// When an author style displays `<head>`, the body is still found (it
/// is the last in-flow element child of the root) for canvas propagation.
#[test]
fn canvas_propagation_finds_the_body_after_a_displayed_head() {
    let head = block(2, r(0.0, 0.0, 400.0, 10.0), style(|_| {}));
    let body = block(3, r(0.0, 10.0, 400.0, 290.0), bg(BLUE));
    let root = with_children(
        block(1, r(0.0, 0.0, 400.0, 300.0), style(|_| {})),
        vec![head, body],
    );
    let tree = xbro_layout::LayoutTree {
        root: Some(root),
        viewport: VIEW,
        document_size: VIEW,
    };
    let list = build_display_list(&tree);
    assert_eq!(list.background, BLUE);
    assert!(
        !list
            .items
            .iter()
            .any(|i| matches!(i, DisplayItem::Rect { color, .. } if *color == BLUE)),
        "the body's background is the canvas"
    );
}

// ---------------------------------------------------------------------
// Rounded replaced content
// ---------------------------------------------------------------------

fn image_box(node: usize, border_box: Rect, f: impl FnOnce(&mut ComputedStyle)) -> LayoutBox {
    let s = style(f);
    let w = s.border_widths();
    let border = Edges::new(w.top, w.right, w.bottom, w.left);
    let mut b = block(node, border_box.inset(border), s);
    b.border = border;
    b.kind = BoxKind::Image;
    b
}

/// `<img style="border-radius:50%">` (avatars): the image is clipped to
/// the rounded content box.
#[test]
fn rounded_images_are_clipped_to_their_corners() {
    let img = image_box(5, r(10.0, 10.0, 80.0, 80.0), |s| {
        s.border_radius = [LengthPercentage::percent(50.0); 4];
    });
    let list = build_display_list(&page(VIEW, vec![img]));
    let clip = list
        .items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushClip { radii, .. } if radii[0] > 39.0))
        .expect("rounded clip around the image");
    assert!(matches!(
        list.items.get(clip + 1),
        Some(DisplayItem::NodeImage { .. })
    ));
    assert_eq!(list.items.get(clip + 2), Some(&DisplayItem::PopClip));
    let mut images = MapImages::default();
    images.nodes.insert(5, solid_image(8, 8, RED));
    let p = render_with(&list, 100, 100, &RasterOptions::default(), &images);
    assert_px(&p, 50, 50, RED);
    assert_px(&p, 12, 12, WHITE);
    assert_px(&p, 87, 87, WHITE);
}

/// The content edge's radii shrink by the border and padding widths.
#[test]
fn image_clip_radii_follow_the_content_edge() {
    let mut img = image_box(5, r(0.0, 0.0, 100.0, 100.0), |s| {
        s.border = solid(4.0, BLACK);
        s.border_radius = [LengthPercentage::px(20.0); 4];
    });
    img.padding = Edges::all(6.0);
    img.content = img.content.inset(Edges::all(6.0));
    let list = build_display_list(&page(VIEW, vec![img]));
    let radii = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::PushClip { rect, radii } if *rect == r(10.0, 10.0, 80.0, 80.0) => {
                Some(*radii)
            }
            _ => None,
        })
        .expect("clip on the content box");
    assert_eq!(radii, [10.0; 4]);
    // Square images get no clip at all.
    let square = image_box(6, r(0.0, 0.0, 50.0, 50.0), |_| {});
    let list = build_display_list(&page(VIEW, vec![square]));
    assert!(!list
        .items
        .iter()
        .any(|i| matches!(i, DisplayItem::PushClip { .. })));
}

// ---------------------------------------------------------------------
// Inline elements: sliced corners and opacity
// ---------------------------------------------------------------------

fn host_with_lines(lines: Vec<xbro_layout::LineBox>) -> LayoutBox {
    let mut host = block(3, r(0.0, 0.0, 300.0, 100.0), style(|_| {}));
    host.lines = lines;
    host
}

fn ib(node: usize, rect: Rect, s: Arc<ComputedStyle>) -> Fragment {
    Fragment::InlineBox(inline_box(node, rect, s))
}

fn border_radii(list: &DisplayList) -> Vec<[f32; 4]> {
    list.items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::Rect { color, radii, .. } if *color == LIME => Some(*radii),
            _ => None,
        })
        .collect()
}

/// An inline element wrapping over three lines rounds only the outer
/// corners of its first and last fragments (`box-decoration-break:
/// slice`), not the edges where it is broken.
#[test]
fn split_inline_boxes_round_only_their_ends() {
    let s = style(|s| {
        s.background_color = LIME;
        s.border_radius = [LengthPercentage::px(6.0); 4];
    });
    let lines = (0..3)
        .map(|i| {
            let y = i as f32 * 20.0;
            line(
                r(0.0, y, 300.0, 20.0),
                y + 15.0,
                vec![ib(7, r(10.0, y, 100.0, 20.0), s.clone())],
            )
        })
        .collect();
    let list = build_display_list(&page(VIEW, vec![host_with_lines(lines)]));
    assert_eq!(
        border_radii(&list),
        vec![[6.0, 0.0, 0.0, 6.0], [0.0; 4], [0.0, 6.0, 6.0, 0.0]]
    );
    // An element on a single line keeps all four corners.
    let one = vec![line(
        r(0.0, 0.0, 300.0, 20.0),
        15.0,
        vec![ib(7, r(10.0, 0.0, 100.0, 20.0), s)],
    )];
    let list = build_display_list(&page(VIEW, vec![host_with_lines(one)]));
    assert_eq!(border_radii(&list), vec![[6.0; 4]]);
}

/// An inline element with `opacity < 1` wraps its background and the
/// fragments it contains (not the following ones) in an opacity group.
#[test]
fn inline_opacity_groups_the_contained_fragments() {
    let translucent = style(|s| {
        s.opacity = 0.5;
        s.background_color = RED;
    });
    let inner = bg(BLUE);
    let after = bg(LIME);
    let lines = vec![line(
        r(0.0, 0.0, 300.0, 20.0),
        15.0,
        vec![
            ib(7, r(10.0, 0.0, 100.0, 20.0), translucent),
            ib(8, r(20.0, 0.0, 30.0, 20.0), inner),
            ib(9, r(110.0, 0.0, 50.0, 20.0), after),
        ],
    )];
    let list = build_display_list(&page(VIEW, vec![host_with_lines(lines)]));
    let push = list
        .items
        .iter()
        .position(|i| matches!(i, DisplayItem::PushOpacity(o) if *o == 0.5))
        .expect("opacity group");
    let pop = list
        .items
        .iter()
        .position(|i| matches!(i, DisplayItem::PopOpacity))
        .expect("closed group");
    let (red, blue, lime) = (
        rect_index(&list, RED),
        rect_index(&list, BLUE),
        rect_index(&list, LIME),
    );
    assert!(push < red && red < blue && blue < pop, "{:#?}", list.items);
    assert!(pop < lime, "following fragments are outside the group");
}

/// `opacity: 0` hides the contained fragments (nested groups included)
/// without emitting anything; groups are closed at the end of the line.
#[test]
fn inline_opacity_zero_and_nested_groups() {
    let hidden = style(|s| {
        s.opacity = 0.0;
        s.background_color = RED;
    });
    let nested = style(|s| {
        s.opacity = 0.5;
        s.background_color = BLUE;
    });
    let lines = vec![
        line(
            r(0.0, 0.0, 300.0, 20.0),
            15.0,
            vec![
                ib(7, r(10.0, 0.0, 100.0, 20.0), hidden),
                ib(8, r(20.0, 0.0, 30.0, 20.0), nested.clone()),
                ib(9, r(120.0, 0.0, 50.0, 20.0), bg(LIME)),
            ],
        ),
        line(
            r(0.0, 20.0, 300.0, 20.0),
            35.0,
            vec![
                ib(10, r(10.0, 20.0, 100.0, 20.0), nested.clone()),
                ib(11, r(20.0, 20.0, 50.0, 20.0), nested),
            ],
        ),
    ];
    let list = build_display_list(&page(VIEW, vec![host_with_lines(lines)]));
    assert!(!list
        .items
        .iter()
        .any(|i| matches!(i, DisplayItem::Rect { color, .. } if *color == RED)));
    rect_index(&list, LIME);
    let pushes = list
        .items
        .iter()
        .filter(|i| matches!(i, DisplayItem::PushOpacity(_)))
        .count();
    let pops = list
        .items
        .iter()
        .filter(|i| matches!(i, DisplayItem::PopOpacity))
        .count();
    assert_eq!((pushes, pops), (2, 2), "{:#?}", list.items);
}

/// End to end: a translucent `<span>` blends with the canvas.
#[test]
fn translucent_spans_are_translucent() {
    let (_, p) = render_html(
        r#"<body style="margin:0;font:20px sans-serif">
           <span style="opacity:0.5;background:rgb(0,0,255);color:rgb(0,0,255)">xxxxxxxx</span></body>"#,
        200,
        40,
    );
    let (r, g, b, _) = px(&p, 6, 12);
    assert!(
        (110..=145).contains(&r) && (110..=145).contains(&g) && b == 255,
        "half blue: {:?}",
        (r, g, b)
    );
}

// ---------------------------------------------------------------------
// Outlines and controls
// ---------------------------------------------------------------------

/// Outlines follow the box's rounded corners (concentric radii).
#[test]
fn outlines_follow_border_radius() {
    let b = block(
        4,
        r(20.0, 20.0, 100.0, 50.0),
        style(|s| {
            s.outline_width = 3.0;
            s.outline_color = RED;
            s.border_radius = [
                LengthPercentage::px(10.0),
                LengthPercentage::ZERO,
                LengthPercentage::px(10.0),
                LengthPercentage::ZERO,
            ];
        }),
    );
    let list = build_display_list(&page(VIEW, vec![b]));
    let outline = list
        .items
        .iter()
        .find_map(|i| match i {
            DisplayItem::Border { rect, radii, sides } if sides[0].color == RED => {
                Some((*rect, *radii))
            }
            _ => None,
        })
        .expect("outline");
    assert_eq!(outline.0, r(17.0, 17.0, 106.0, 56.0));
    assert_eq!(outline.1, [13.0, 0.0, 13.0, 0.0]);
}

/// The `<select>` arrow paints above the selected option's text.
#[test]
fn select_arrow_paints_above_its_text() {
    let Some(fonts) = fonts() else { return };
    let mut select = block(5, r(0.0, 0.0, 120.0, 20.0), style(|s| s.color = BLACK));
    select.kind = BoxKind::Control(ControlKind::Select);
    let t = text(
        fonts,
        6,
        "A very long option",
        0.0,
        15.0,
        14.0,
        style(|_| {}),
    );
    select.lines = vec![line(
        r(0.0, 0.0, 120.0, 20.0),
        15.0,
        vec![Fragment::Text(t)],
    )];
    let list = build_display_list(&page(VIEW, vec![select]));
    let text = list
        .items
        .iter()
        .position(|i| matches!(i, DisplayItem::Text(_)))
        .expect("text");
    let arrow = list
        .items
        .iter()
        .position(|i| matches!(i, DisplayItem::Polygon { .. }))
        .expect("arrow");
    assert!(text < arrow);
}

// ---------------------------------------------------------------------
// Images: EXIF orientation and minification
// ---------------------------------------------------------------------

/// Raw EXIF (TIFF) data holding only an orientation tag.
fn exif_orientation(value: u16) -> Vec<u8> {
    let mut v = b"II*\0".to_vec();
    v.extend_from_slice(&8u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&0x0112u16.to_le_bytes());
    v.extend_from_slice(&3u16.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&value.to_le_bytes());
    v.extend_from_slice(&[0, 0]);
    v.extend_from_slice(&0u32.to_le_bytes());
    v
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A 4 × 2 image whose top-left pixel is red, the rest blue.
fn wide_image() -> image::DynamicImage {
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(4, 2, |x, y| {
        if x == 0 && y == 0 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 255, 255])
        }
    }))
}

fn raster_pixel(img: &Image, x: u32, y: u32) -> (u8, u8, u8, u8) {
    let Image::Raster(p) = img else {
        panic!("raster expected")
    };
    let c = p.pixel(x, y).expect("in bounds");
    (c.red(), c.green(), c.blue(), c.alpha())
}

/// Photos carry their rotation in EXIF metadata; browsers apply it
/// (`image-orientation: from-image`). Orientation 6 = rotate 90° clockwise.
#[test]
fn png_exif_orientation_is_applied() {
    let mut png = Cursor::new(Vec::new());
    wide_image()
        .write_to(&mut png, image::ImageFormat::Png)
        .expect("png");
    let png = png.into_inner();
    // Insert an eXIf chunk right after IHDR (8 + 25 bytes).
    let exif = exif_orientation(6);
    let mut chunk = (exif.len() as u32).to_be_bytes().to_vec();
    let mut body = b"eXIf".to_vec();
    body.extend_from_slice(&exif);
    chunk.extend_from_slice(&body);
    chunk.extend_from_slice(&crc32(&body).to_be_bytes());
    let mut bytes = png[..33].to_vec();
    bytes.extend_from_slice(&chunk);
    bytes.extend_from_slice(&png[33..]);
    let img = Image::decode(&bytes, Some("image/png")).expect("decodes");
    assert_eq!(img.natural_size(), Size::new(2.0, 4.0), "rotated");
    // The top-left corner moved to the top-right.
    assert_eq!(raster_pixel(&img, 1, 0), (255, 0, 0, 255));
    assert_eq!(raster_pixel(&img, 0, 0), (0, 0, 255, 255));
}

#[test]
fn jpeg_exif_orientation_is_applied() {
    let mut jpeg = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(wide_image().to_rgb8())
        .resize_exact(40, 20, image::imageops::FilterType::Nearest)
        .write_to(&mut jpeg, image::ImageFormat::Jpeg)
        .expect("jpeg");
    let jpeg = jpeg.into_inner();
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(&exif_orientation(6));
    let mut bytes = jpeg[..2].to_vec(); // SOI
    bytes.extend_from_slice(&[0xFF, 0xE1]);
    bytes.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(&jpeg[2..]);
    let img = Image::decode(&bytes, None).expect("decodes");
    assert_eq!(img.natural_size(), Size::new(20.0, 40.0), "rotated");
    // The red block (top-left 10×10) is now at the top right.
    let (r, _, b, _) = raster_pixel(&img, 15, 4);
    assert!(r > 200 && b < 60, "{:?}", raster_pixel(&img, 15, 4));
    let (r, _, b, _) = raster_pixel(&img, 4, 4);
    assert!(r < 60 && b > 200, "{:?}", raster_pixel(&img, 4, 4));
}

/// A raster image drawn at a tenth of its size averages its pixels
/// instead of sampling a few of them (stripes of one black and two white
/// pixels become a light grey, not a moiré of black and white bands).
#[test]
fn minified_images_are_filtered() {
    let (w, h) = (300u32, 300u32);
    let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("pixmap");
    for y in 0..h {
        for x in 0..w {
            let v = if x % 3 == 0 { 0 } else { 255 };
            let i = ((y * w + x) * 4) as usize;
            pixmap.data_mut()[i..i + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    let mut images = MapImages::default();
    images.nodes.insert(1, Image::Raster(Arc::new(pixmap)));
    let list = DisplayList {
        items: vec![DisplayItem::NodeImage {
            rect: r(3.0, 3.0, 30.0, 30.0),
            node: xbro_dom::NodeId(1),
        }],
        background: WHITE,
    };
    let p = render_with(&list, 40, 40, &RasterOptions::default(), &images);
    for y in 5..31 {
        for x in 5..31 {
            let (v, ..) = px(&p, x, y);
            assert!((120..=220).contains(&v), "pixel ({x},{y}) = {v}");
        }
    }
}

/// A positioned `::after` moved by layout under an ancestor containing
/// block still paints after its element's subtree and before the
/// element's later siblings.
#[test]
fn moved_after_pseudo_keeps_its_document_position() {
    let host = with_children(
        block(3, r(0.0, 0.0, 200.0, 50.0), style(|_| {})),
        vec![positioned(5, Position::Relative, BLUE)],
    );
    let later = positioned(9, Position::Relative, GREEN);
    let mut after = positioned(3, Position::Absolute, LIME);
    after.pseudo = Some(Pseudo::After);
    let cb = with_children(
        block(
            2,
            r(0.0, 0.0, 300.0, 200.0),
            style(|s| s.position = Position::Relative),
        ),
        vec![host, later, after],
    );
    let list = build_display_list(&page(VIEW, vec![cb]));
    let (child, after, later) = (
        rect_index(&list, BLUE),
        rect_index(&list, LIME),
        rect_index(&list, GREEN),
    );
    assert!(child < after && after < later, "{child} {after} {later}");
}

/// Hostile geometry and opacities in inline content keep the opacity
/// groups balanced and never panic.
#[test]
fn hostile_inline_groups_stay_balanced() {
    let nan = f32::NAN;
    let weird = [
        r(nan, 0.0, 10.0, 10.0),
        r(0.0, nan, nan, nan),
        r(f32::INFINITY, 0.0, f32::NEG_INFINITY, 1.0),
        r(-1e30, 0.0, 1e30, 1.0),
        r(5.0, 0.0, 0.0, 0.0),
    ];
    let mut fragments = Vec::new();
    for (i, rect) in weird.iter().enumerate() {
        for opacity in [0.5, 0.0, nan, -1.0, 2.0] {
            fragments.push(ib(
                20 + i,
                *rect,
                style(|s| {
                    s.opacity = opacity;
                    s.background_color = RED;
                    s.border_radius = [LengthPercentage::percent(nan); 4];
                    s.outline_width = nan;
                }),
            ));
        }
        let mut atomic = block(40 + i, *rect, bg(BLUE));
        atomic.style = style(|s| {
            s.opacity = 0.5;
            s.background_color = BLUE;
        });
        fragments.push(Fragment::Atomic(atomic));
    }
    let lines = vec![
        line(r(0.0, 0.0, 300.0, 20.0), 15.0, fragments.clone()),
        line(r(nan, nan, nan, nan), nan, fragments),
    ];
    let list = build_display_list(&page(VIEW, vec![host_with_lines(lines)]));
    let mut depth = 0i32;
    for item in &list.items {
        match item {
            DisplayItem::PushOpacity(o) => {
                assert!(*o > 0.0 && *o < 1.0, "{o}");
                depth += 1;
            }
            DisplayItem::PopOpacity => {
                depth -= 1;
                assert!(depth >= 0);
            }
            _ => {}
        }
    }
    assert_eq!(depth, 0);
    let _ = render(&list, 100, 50);
}

/// Positioned boxes inside nested clipping containers share the clips
/// they have in common instead of repeating the whole chain each (the
/// list stayed proportional to boxes × nesting depth).
#[test]
fn positioned_boxes_share_their_clip_chains() {
    let kids: Vec<LayoutBox> = (0..100)
        .map(|i| {
            block(
                100 + i,
                r(0.0, i as f32, 10.0, 1.0),
                style(|s| {
                    s.position = Position::Relative;
                    s.background_color = RED;
                }),
            )
        })
        .collect();
    let mut b = with_children(
        block(
            10,
            r(0.0, 0.0, 100.0, 100.0),
            style(|s| s.overflow_x = xbro_style::Overflow::Hidden),
        ),
        kids,
    );
    for i in 0..20 {
        b = with_children(
            block(
                9 - i.min(9),
                r(0.0, 0.0, 100.0, 100.0),
                style(|s| s.overflow_x = xbro_style::Overflow::Hidden),
            ),
            vec![b],
        );
    }
    let list = build_display_list(&page(VIEW, vec![b]));
    let pushes = list
        .items
        .iter()
        .filter(|i| matches!(i, DisplayItem::PushClip { .. }))
        .count();
    let pops = list
        .items
        .iter()
        .filter(|i| matches!(i, DisplayItem::PopClip))
        .count();
    assert_eq!(pushes, pops);
    assert!(pushes <= 21, "{pushes} clips pushed");
    let p = render(&list, 100, 100);
    assert_px(&p, 5, 50, RED);
    assert_px(&p, 50, 50, WHITE);
}

/// The default list (used by the engine when painting fails) has the
/// white canvas of an empty document, not a transparent one (which a
/// window shows as black).
#[test]
fn default_display_list_has_a_white_canvas() {
    let list = DisplayList::default();
    assert!(list.is_empty());
    assert_eq!(list.background, WHITE);
    assert_eq!(
        list,
        build_display_list(&xbro_layout::LayoutTree {
            root: None,
            viewport: VIEW,
            document_size: VIEW,
        })
    );
}
