//! Display list: a flat, ordered list of drawing commands in CSS px document
//! coordinates, built from a [`LayoutTree`] in CSS paint order.
//!
//! # Paint order
//!
//! The builder follows CSS 2.1 Appendix E. Stacking contexts are created by
//! the root, positioned boxes with `z-index` other than `auto`, flex items
//! with a `z-index`, `position: fixed` boxes (as browsers do) and boxes with
//! `opacity < 1`. Within a stacking context:
//!
//! 1. the root's own background and border;
//! 2. child stacking contexts with negative `z-index` (by z, then tree order);
//! 3. backgrounds and borders of in-flow, non-positioned block-level
//!    descendants, in tree order;
//! 4. non-positioned floats, each painted atomically;
//! 5. inline content in tree order: per line, inline box backgrounds and
//!    borders, text (with its decorations), atomic inlines painted atomically;
//!    flex items and block-level replaced content are painted atomically here
//!    too; a box's outline follows its inline content;
//! 6. positioned descendants with `z-index: auto` or `0` and other
//!    `z-index: 0`-like stacking contexts, in tree order;
//! 7. child stacking contexts with positive `z-index`.
//!
//! "Atomically" means as if the box created a stacking context, except that
//! its positioned descendants and real stacking contexts are painted by the
//! enclosing stacking context (so `z-index` works across them).
//!
//! Simplifications: inline elements (`<span>`) are never stacking contexts or
//! positioned layers (their fragments paint with the line); `overflow` clips
//! apply to deferred positioned descendants according to their containing
//! block within the stacking context, while clips of boxes outside the
//! stacking context apply to all of its content; fixed content ignores all
//! ancestor clips (the rasteriser resets clipping inside `PushFixed`).

mod background;
mod builder;
mod controls;

use xbro_dom::NodeId;
use xbro_layout::{LayoutTree, Point, Rect};
use xbro_style::{BackgroundRepeat, BackgroundSize, BorderStyle, Color, LengthPercentage};
use xbro_text::{FontId, FontSystem, ShapedGlyph};

use crate::image::{ImageProvider, NoImageProvider};

pub use background::{background_tile, resolve_radii};

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

impl GlyphRun {
    /// Total advance width of the run (end of the last glyph), in CSS px.
    pub fn width(&self) -> f32 {
        self.glyphs
            .iter()
            .map(|g| g.x + g.advance)
            .fold(0.0f32, f32::max)
    }
}

/// One side of a [`DisplayItem::Border`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BorderSideItem {
    pub width: f32,
    pub style: BorderStyle,
    pub color: Color,
}

impl BorderSideItem {
    /// True when this side paints something.
    pub fn is_visible(&self) -> bool {
        self.width > 0.0 && self.style.is_visible() && !self.color.is_transparent()
    }
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
    ///
    /// A zero `tile` means "the image's natural size".
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
    /// Ellipse inscribed in `rect`: filled when `fill`, else stroked with
    /// `stroke_width` (the stroke is centred on the ellipse). Used for list
    /// markers, radio buttons and range thumbs.
    Ellipse {
        rect: Rect,
        color: Color,
        fill: bool,
        stroke_width: f32,
    },
    /// Open polyline stroked with round caps and joins (check marks).
    Polyline {
        points: Vec<Point>,
        width: f32,
        color: Color,
    },
    /// Filled polygon (`<select>` arrows).
    Polygon {
        points: Vec<Point>,
        color: Color,
    },
    /// Background image whose tile geometry depends on the image's natural
    /// size, which was not known when the list was built (no
    /// [`ImageProvider`] or image not loaded yet). The rasteriser resolves it
    /// with [`background_tile`]: `size` and `position` against the
    /// positioning `area`, painted within `clip`.
    DeferredBackgroundImage {
        url: String,
        area: Rect,
        clip: Rect,
        size: BackgroundSize,
        position: (LengthPercentage, LengthPercentage),
        repeat: BackgroundRepeat,
    },
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
///
/// Without fonts, text decorations use fallback metrics (underline
/// 0.1em below the baseline, 1/14em thick); without images, background images
/// are emitted as [`DisplayItem::DeferredBackgroundImage`] unless their size
/// is fully explicit. See [`build_display_list_full`].
pub fn build_display_list(tree: &LayoutTree) -> DisplayList {
    builder::build(tree, None, &NoImageProvider)
}

/// Like [`build_display_list`], resolving background image geometry with the
/// natural sizes of the images `images` provides (images it does not have
/// yet are emitted as [`DisplayItem::DeferredBackgroundImage`]).
pub fn build_display_list_with_images(
    tree: &LayoutTree,
    images: &dyn ImageProvider,
) -> DisplayList {
    builder::build(tree, None, images)
}

/// Like [`build_display_list_with_images`], also using real font metrics for
/// text decorations (underline position/thickness, overline, line-through).
pub fn build_display_list_full(
    tree: &LayoutTree,
    fonts: &FontSystem,
    images: &dyn ImageProvider,
) -> DisplayList {
    builder::build(tree, Some(fonts), images)
}

impl DisplayList {
    /// Number of items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when the list holds no items (the canvas background still paints).
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
