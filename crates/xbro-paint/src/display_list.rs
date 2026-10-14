//! Display list: a flat, ordered list of drawing commands in CSS px document
//! coordinates, built from a [`LayoutTree`] in CSS paint order.
//!
//! # Paint order
//!
//! The builder follows CSS 2.1 Appendix E. Stacking contexts are created by
//! the root, positioned boxes with `z-index` other than `auto`, flex items
//! with a `z-index`, `position: fixed` and `sticky` boxes (as browsers do)
//! and boxes with `opacity < 1`. Within a stacking context:
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
//! The boxes of steps 2, 6 and 7 are ordered by document position (node
//! index, with `::before` after its element and `::after` after the
//! element's subtree): layout moves absolutely positioned boxes under
//! their containing block, so the box tree order alone would paint an
//! overlay positioned before some content above it.
//!
//! "Atomically" means as if the box created a stacking context, except that
//! its positioned descendants and real stacking contexts are painted by the
//! enclosing stacking context (so `z-index` works across them).
//!
//! Simplifications: inline elements (`<span>`) are never stacking contexts or
//! positioned layers (their fragments paint with the line), but an inline
//! element with `opacity < 1` groups the fragments it contains on each
//! line in an opacity group, and its fragments only round the corners of
//! its first and last fragment (`box-decoration-break: slice`); `overflow` clips
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
    /// Font the run was shaped with (each glyph names its own face, which
    /// may differ after fallback).
    pub font: FontId,
    /// Font size in CSS px.
    pub font_size: f32,
    /// Pen position of the run's first glyph on the baseline.
    pub origin: Point,
    /// Glyphs with `x` relative to `origin.x`.
    pub glyphs: Vec<ShapedGlyph>,
    /// Text color (never fully transparent in built lists).
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
    /// Used width in CSS px (0 for a missing side).
    pub width: f32,
    /// Line style.
    pub style: BorderStyle,
    /// Line color.
    pub color: Color,
}

impl BorderSideItem {
    /// True when this side paints something.
    pub fn is_visible(&self) -> bool {
        self.width > 0.0 && self.style.is_visible() && !self.color.is_transparent()
    }
}

/// One drawing command. Coordinates are CSS px in the document (in the
/// viewport between [`DisplayItem::PushFixed`] and [`DisplayItem::PopFixed`]);
/// corner radii are ordered top-left, top-right, bottom-right, bottom-left.
#[derive(Clone, Debug, PartialEq)]
pub enum DisplayItem {
    /// Filled rectangle with optional corner radii (tl, tr, br, bl).
    Rect {
        /// Area to fill.
        rect: Rect,
        /// Fill color.
        color: Color,
        /// Corner radii.
        radii: [f32; 4],
    },
    /// Border of `rect` (the border box). Sides: top, right, bottom, left.
    Border {
        /// Border box (outer edge of the border).
        rect: Rect,
        /// Top, right, bottom and left sides.
        sides: [BorderSideItem; 4],
        /// Outer corner radii.
        radii: [f32; 4],
    },
    /// Linear gradient filling `rect` (CSS angle convention, stops in 0..=1).
    LinearGradient {
        /// Area to fill (the gradient box).
        rect: Rect,
        /// Direction of the gradient line: 0 = to top, 90 = to right.
        angle_deg: f32,
        /// Colors and their positions along the gradient line.
        stops: Vec<(Color, f32)>,
        /// Corner radii of the filled area.
        radii: [f32; 4],
    },
    /// A run of glyphs.
    Text(GlyphRun),
    /// Image keyed by node (replaced elements), drawn scaled into `rect`.
    NodeImage {
        /// Destination (the content box).
        rect: Rect,
        /// Replaced element, passed to [`ImageProvider::node_image`].
        node: NodeId,
    },
    /// Image keyed by URL drawn as a background: tiled per `repeat` starting at
    /// `origin` with tile size `tile`, clipped to `clip`.
    ///
    /// A zero `tile` means "the image's natural size".
    BackgroundImage {
        /// Absolute image URL, passed to [`ImageProvider::url_image`].
        url: String,
        /// Painting area (the border box).
        clip: Rect,
        /// Top-left corner of one tile; the others follow from `repeat`.
        origin: Point,
        /// Size of one tile.
        tile: xbro_layout::Size,
        /// Tiling axes.
        repeat: BackgroundRepeat,
    },
    /// Image keyed by URL drawn once, scaled into `rect` (list-style-image, `content: url()`).
    UrlImage {
        /// Destination.
        rect: Rect,
        /// Absolute image URL, passed to [`ImageProvider::url_image`].
        url: String,
    },
    /// Clip subsequent items to `rect` (with optional radii) until the matching `PopClip`.
    PushClip {
        /// Clip rectangle (a padding box).
        rect: Rect,
        /// Corner radii of the clip shape.
        radii: [f32; 4],
    },
    /// Ends the innermost [`DisplayItem::PushClip`].
    PopClip,
    /// Composite subsequent items with `opacity` until the matching `PopOpacity`.
    PushOpacity(f32),
    /// Ends the innermost [`DisplayItem::PushOpacity`] group.
    PopOpacity,
    /// Subsequent items (until `PopFixed`) are in viewport coordinates and ignore scrolling.
    PushFixed,
    /// Ends the innermost [`DisplayItem::PushFixed`] section.
    PopFixed,
    /// Ellipse inscribed in `rect`: filled when `fill`, else stroked with
    /// `stroke_width` (the stroke is centred on the ellipse). Used for list
    /// markers, radio buttons and range thumbs.
    Ellipse {
        /// Bounding box of the ellipse.
        rect: Rect,
        /// Fill or stroke color.
        color: Color,
        /// Filled (`true`) or stroked.
        fill: bool,
        /// Stroke width in CSS px (ignored when filled).
        stroke_width: f32,
    },
    /// Open polyline stroked with round caps and joins (check marks).
    Polyline {
        /// Vertices, at least two to draw anything.
        points: Vec<Point>,
        /// Stroke width in CSS px.
        width: f32,
        /// Stroke color.
        color: Color,
    },
    /// Filled polygon (`<select>` arrows).
    Polygon {
        /// Vertices (closed implicitly), at least three to draw anything.
        points: Vec<Point>,
        /// Fill color.
        color: Color,
    },
    /// Background image whose tile geometry depends on the image's natural
    /// size, which was not known when the list was built (no
    /// [`ImageProvider`] or image not loaded yet). The rasteriser resolves it
    /// with [`background_tile`]: `size` and `position` against the
    /// positioning `area`, painted within `clip`.
    DeferredBackgroundImage {
        /// Absolute image URL, passed to [`ImageProvider::url_image`].
        url: String,
        /// Background positioning area (the padding box).
        area: Rect,
        /// Painting area (the border box).
        clip: Rect,
        /// `background-size`.
        size: BackgroundSize,
        /// `background-position` (x, y).
        position: (LengthPercentage, LengthPercentage),
        /// `background-repeat`.
        repeat: BackgroundRepeat,
    },
}

/// Drawing commands of a page plus its canvas color.
///
/// The default list is empty with a white canvas, like the list of an
/// empty document.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayList {
    /// Commands in paint order (later items paint over earlier ones).
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

impl Default for DisplayList {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            background: Color::WHITE,
        }
    }
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
