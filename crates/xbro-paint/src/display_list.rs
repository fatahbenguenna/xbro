//! Display list: a flat, ordered list of drawing commands in CSS px document
//! coordinates, built from a [`LayoutTree`] in CSS paint order.

use xbro_dom::NodeId;
use xbro_layout::{LayoutTree, Point, Rect};
use xbro_style::{BackgroundRepeat, BorderStyle, Color};
use xbro_text::{FontId, ShapedGlyph};

/// Glyphs of one text fragment, positioned absolutely.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRun {
    pub font: FontId,
    pub font_size: f32,
    /// Pen position of the run's first glyph on the baseline.
    pub origin: Point,
    /// Glyphs with `x` relative to `origin.x`.
    pub glyphs: Vec<ShapedGlyph>,
    pub color: Color,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BorderSideItem {
    pub width: f32,
    pub style: BorderStyle,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DisplayItem {
    /// Filled rectangle with optional corner radii (tl, tr, br, bl).
    Rect {
        rect: Rect,
        color: Color,
        radii: [f32; 4],
    },
    /// Border of `rect` (the border box). Sides: top, right, bottom, left.
    Border {
        rect: Rect,
        sides: [BorderSideItem; 4],
        radii: [f32; 4],
    },
    /// Linear gradient filling `rect` (CSS angle convention, stops in 0..=1).
    LinearGradient {
        rect: Rect,
        angle_deg: f32,
        stops: Vec<(Color, f32)>,
        radii: [f32; 4],
    },
    Text(GlyphRun),
    /// Image keyed by node (replaced elements), drawn scaled into `rect`.
    NodeImage {
        rect: Rect,
        node: NodeId,
    },
    /// Image keyed by URL drawn as a background: tiled per `repeat` starting at
    /// `origin` with tile size `tile`, clipped to `clip`.
    BackgroundImage {
        url: String,
        clip: Rect,
        origin: Point,
        tile: xbro_layout::Size,
        repeat: BackgroundRepeat,
    },
    /// Image keyed by URL drawn once, scaled into `rect` (list-style-image, `content: url()`).
    UrlImage {
        rect: Rect,
        url: String,
    },
    /// Clip subsequent items to `rect` (with optional radii) until the matching `PopClip`.
    PushClip {
        rect: Rect,
        radii: [f32; 4],
    },
    PopClip,
    /// Composite subsequent items with `opacity` until the matching `PopOpacity`.
    PushOpacity(f32),
    PopOpacity,
    /// Subsequent items (until `PopFixed`) are in viewport coordinates and ignore scrolling.
    PushFixed,
    PopFixed,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
    /// Canvas background (propagated from the root element, or from `<body>` when
    /// the root's background is transparent; white when both are transparent).
    pub background: Color,
}

/// Builds the display list following CSS 2.1 Appendix E paint order
/// (simplified): for each stacking context, backgrounds/borders of blocks in
/// tree order, then floats, then inline content (inline box backgrounds, text,
/// atomic inlines, decorations), then positioned descendants sorted by
/// `z-index` (negative ones before in-flow content). Boxes with
/// `visibility: hidden` paint nothing (their visible descendants still paint);
/// `opacity < 1` wraps the box's items in Push/PopOpacity; `overflow != visible`
/// clips descendants to the padding box. Items wholly outside any reasonable
/// area (e.g. hidden off-screen at -9999px) are still emitted; culling happens
/// at raster time.
pub fn build_display_list(tree: &LayoutTree) -> DisplayList {
    let _ = tree;
    todo!("xbro-paint: implement build_display_list")
}
