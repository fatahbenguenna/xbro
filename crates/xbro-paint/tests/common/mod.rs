//! Helpers shared by the integration tests: hand-built layout trees,
//! rendering and pixel access.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use xbro_dom::NodeId;
use xbro_layout::{
    BoxKind, Edges, Fragment, InlineBoxFragment, LayoutBox, LayoutTree, LineBox, Rect, Size,
    TextFragment,
};
use xbro_paint::{rasterize, DisplayList, Image, ImageProvider, NoImageProvider, RasterOptions};
use xbro_style::{BorderSide, BorderStyle, Color, ComputedStyle, Display};
use xbro_text::{FontId, FontQuery, FontSystem};

pub const RED: Color = Color::rgb(255, 0, 0);
pub const GREEN: Color = Color::rgb(0, 128, 0);
pub const LIME: Color = Color::rgb(0, 255, 0);
pub const BLUE: Color = Color::rgb(0, 0, 255);
pub const WHITE: Color = Color::WHITE;
pub const BLACK: Color = Color::BLACK;

const SANS: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";

/// A deterministic font system with DejaVu Sans only (`None` when the font
/// is not installed: text tests are then skipped).
pub fn fonts() -> Option<&'static FontSystem> {
    static FONTS: OnceLock<Option<FontSystem>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let data = std::fs::read(SANS).ok()?;
            let fonts = FontSystem::from_fonts(vec![data]);
            fonts.has_fonts().then_some(fonts)
        })
        .as_ref()
}

/// An empty font system (for tests without text).
pub fn no_fonts() -> &'static FontSystem {
    static FONTS: OnceLock<FontSystem> = OnceLock::new();
    FONTS.get_or_init(|| FontSystem::from_fonts(Vec::new()))
}

/// A block-level style customised by `f`.
pub fn style(f: impl FnOnce(&mut ComputedStyle)) -> Arc<ComputedStyle> {
    let mut s = ComputedStyle {
        display: Display::Block,
        ..ComputedStyle::default()
    };
    f(&mut s);
    Arc::new(s)
}

/// A block box whose border box is `rect` (no padding/border/margin).
pub fn block(node: usize, rect: Rect, style: Arc<ComputedStyle>) -> LayoutBox {
    LayoutBox {
        node: Some(NodeId(node)),
        pseudo: None,
        style,
        kind: BoxKind::Block,
        content: rect,
        padding: Edges::ZERO,
        border: Edges::ZERO,
        margin: Edges::ZERO,
        children: Vec::new(),
        lines: Vec::new(),
        marker: None,
        border_top_gap: None,
    }
}

/// A block whose border box is `rect`, with used border widths taken
/// from its style (content box shrunk accordingly).
pub fn bordered(node: usize, rect: Rect, style: Arc<ComputedStyle>) -> LayoutBox {
    let w = style.border_widths();
    let border = Edges::new(w.top, w.right, w.bottom, w.left);
    let mut b = block(node, rect.inset(border), style);
    b.border = border;
    b
}

pub fn with_children(mut b: LayoutBox, children: Vec<LayoutBox>) -> LayoutBox {
    b.children = children;
    b
}

/// Solid border sides of `width` and `color` on every side.
pub fn solid(width: f32, color: Color) -> xbro_style::Sides<BorderSide> {
    xbro_style::Sides::all(BorderSide {
        width,
        style: BorderStyle::Solid,
        color,
    })
}

/// A tree with an `<html>` root covering `size` (transparent) and a body child.
pub fn page(size: Size, body_children: Vec<LayoutBox>) -> LayoutTree {
    let full = Rect::new(0.0, 0.0, size.width, size.height);
    let body = with_children(block(2, full, style(|_| {})), body_children);
    let root = with_children(block(1, full, style(|_| {})), vec![body]);
    LayoutTree {
        root: Some(root),
        viewport: size,
        document_size: size,
    }
}

/// The DejaVu Sans font id in [`fonts`].
pub fn sans(fonts: &FontSystem) -> FontId {
    let families = vec!["sans-serif".to_string()];
    fonts.select(&FontQuery {
        families: &families,
        weight: 400,
        italic: false,
    })
}

/// A shaped text fragment with its pen at (`x`, `baseline`).
pub fn text(
    fonts: &FontSystem,
    node: usize,
    s: &str,
    x: f32,
    baseline: f32,
    size: f32,
    style: Arc<ComputedStyle>,
) -> TextFragment {
    let font = sans(fonts);
    let run = fonts.shape(font, size, s, 0.0, 0.0);
    let m = fonts.metrics(font, size);
    TextFragment {
        node: NodeId(node),
        rect: Rect::new(x, baseline - m.ascent, run.width, m.ascent + m.descent),
        baseline,
        text: s.to_string(),
        font,
        font_size: size,
        glyphs: run.glyphs,
        style,
    }
}

/// A line box holding `fragments`.
pub fn line(rect: Rect, baseline: f32, fragments: Vec<Fragment>) -> LineBox {
    LineBox {
        rect,
        baseline,
        fragments,
    }
}

/// An inline box fragment covering `rect`.
pub fn inline_box(node: usize, rect: Rect, style: Arc<ComputedStyle>) -> InlineBoxFragment {
    let w = style.border_widths();
    InlineBoxFragment {
        node: NodeId(node),
        style,
        rect,
        border: Edges::new(w.top, w.right, w.bottom, w.left),
        padding: Edges::ZERO,
    }
}

/// Renders `list` into a new `w` × `h` pixmap.
pub fn render_with(
    list: &DisplayList,
    w: u32,
    h: u32,
    options: &RasterOptions,
    images: &dyn ImageProvider,
) -> tiny_skia::Pixmap {
    let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("pixmap");
    let fonts = fonts().unwrap_or_else(no_fonts);
    rasterize(list, fonts, images, &mut pixmap, options);
    pixmap
}

pub fn render(list: &DisplayList, w: u32, h: u32) -> tiny_skia::Pixmap {
    render_with(list, w, h, &RasterOptions::default(), &NoImageProvider)
}

/// Pixel at (x, y) as (r, g, b, a) premultiplied.
pub fn px(p: &tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8, u8) {
    let c = p.pixel(x, y).expect("pixel in bounds");
    (c.red(), c.green(), c.blue(), c.alpha())
}

pub fn rgb(c: Color) -> (u8, u8, u8, u8) {
    (c.r, c.g, c.b, 255)
}

/// True when every channel differs by at most `tol`.
pub fn close(a: (u8, u8, u8, u8), b: (u8, u8, u8, u8), tol: u8) -> bool {
    a.0.abs_diff(b.0) <= tol
        && a.1.abs_diff(b.1) <= tol
        && a.2.abs_diff(b.2) <= tol
        && a.3.abs_diff(b.3) <= tol
}

#[track_caller]
pub fn assert_px(p: &tiny_skia::Pixmap, x: u32, y: u32, expected: Color) {
    let got = px(p, x, y);
    assert!(
        close(got, rgb(expected), 2),
        "pixel ({x},{y}) = {got:?}, expected {:?}",
        rgb(expected)
    );
}

/// Number of pixels in `rect` (device px) darker than `threshold` (luma).
pub fn dark_pixels(
    p: &tiny_skia::Pixmap,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    threshold: u8,
) -> usize {
    let mut n = 0;
    for y in y0..y1.min(p.height()) {
        for x in x0..x1.min(p.width()) {
            let (r, g, b, _) = px(p, x, y);
            let luma = (u32::from(r) * 3 + u32::from(g) * 6 + u32::from(b)) / 10;
            if luma < u32::from(threshold) {
                n += 1;
            }
        }
    }
    n
}

/// Image provider backed by maps (for image tests).
#[derive(Default)]
pub struct MapImages {
    pub nodes: HashMap<usize, Image>,
    pub urls: HashMap<String, Image>,
}

impl ImageProvider for MapImages {
    fn node_image(&self, node: NodeId) -> Option<Image> {
        self.nodes.get(&node.0).cloned()
    }
    fn url_image(&self, url: &str) -> Option<Image> {
        self.urls.get(url).cloned()
    }
}

/// A `w` × `h` checkerboard of `a` and `b` 1-px cells as a raster image.
pub fn checker(w: u32, h: u32, a: Color, b: Color) -> Image {
    let mut pixmap = tiny_skia::Pixmap::new(w, h).expect("pixmap");
    for y in 0..h {
        for x in 0..w {
            let c = if (x + y) % 2 == 0 { a } else { b };
            let i = ((y * w + x) * 4) as usize;
            pixmap.data_mut()[i..i + 4].copy_from_slice(&[c.r, c.g, c.b, c.a]);
        }
    }
    Image::Raster(Arc::new(pixmap))
}

/// Solid-color raster image.
pub fn solid_image(w: u32, h: u32, c: Color) -> Image {
    checker(w, h, c, c)
}
