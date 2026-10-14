//! CPU rasterisation of display lists with tiny-skia.
//!
//! The drawing area is the band of full-width rows `offset.1 .. offset.1 +
//! height` of the target; everything outside it is left untouched. Items
//! are mapped to device pixels with `device = (doc − scroll) × scale +
//! offset` (items between `PushFixed`/`PopFixed` ignore `scroll`), culled
//! against the current clip, and drawn:
//!
//! * rectangles snapped to device pixels, rounded shapes anti-aliased;
//! * borders per side (diagonal joins between differing colors), with
//!   dashed/dotted/double/groove/ridge/inset/outset styles;
//! * text from cached glyph coverage masks blended in a dedicated loop;
//! * images with bilinear filtering (raster images drawn at less than
//!   half their size are first reduced with a box filter, from a bounded
//!   cache, instead of shimmering with moiré), SVG rendered by resvg at the
//!   destination size (bounded cache), uniformly fitted and centred as
//!   with the default `preserveAspectRatio` (`none`/`slice` on the root
//!   element are not honoured: usvg flattens the viewBox);
//! * clips as device rects (geometric clipping) plus lazily built masks
//!   for rounded clips, used only by shapes that reach a rounded corner;
//!   `PushFixed` starts a fresh clip stack because fixed boxes are not
//!   clipped by their ancestors' overflow;
//! * opacity groups in transparent layers composited on `PopOpacity`
//!   (only their dirty region); a group holding a single solid-color item
//!   draws it with a faded color instead. Layers are reused within a call
//!   (cleared over their dirty region only); at most 256 MiB of layers are
//!   live at once, and deeper nested groups are drawn without their own
//!   opacity (the enclosing groups' opacity still applies).

mod border;
mod clip;
mod images;
mod shapes;
mod text;

use std::rc::Rc;

use tiny_skia::{Mask, Pixmap, PixmapMut};
use xbro_layout::{Point, Rect};
use xbro_style::Color;
use xbro_text::{FontId, FontSystem};

use crate::display_list::{DisplayItem, DisplayList};
use crate::image::ImageProvider;
use clip::{ClipState, DevRect, MaskPool};

/// Where and how to rasterise.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RasterOptions {
    /// Document point (CSS px) drawn at the target's top-left (the scroll position).
    pub scroll: Point,
    /// Device pixels per CSS px (HiDPI factor × zoom).
    pub scale: f32,
    /// Offset in device pixels at which to draw inside the target (e.g. below the toolbar).
    pub offset: (i32, i32),
    /// Height in device pixels of the area to draw, starting at `offset.1`
    /// (the target may be taller, e.g. when it also holds the browser UI).
    /// `None` = to the bottom of the target.
    pub height: Option<u32>,
}

impl Default for RasterOptions {
    fn default() -> Self {
        Self {
            scroll: Point::ZERO,
            scale: 1.0,
            offset: (0, 0),
            height: None,
        }
    }
}

/// Largest accepted scale factor.
const MAX_SCALE: f32 = 64.0;

/// Memory budget for the opacity layers live at the same time (each is a
/// pixmap of the whole drawing area).
const MAX_LAYER_BYTES: usize = 256 << 20;

/// Upper bound on live opacity layers, whatever the area size.
const MAX_LAYERS: usize = 64;

/// `x / 255` rounded, exact for `x <= 255 * 255`.
#[inline]
pub(crate) fn div255(x: u32) -> u32 {
    let x = x + 128;
    (x + (x >> 8)) >> 8
}

/// Paints `list` into `target`: first fills the drawing area with
/// `list.background`, then draws every item intersecting the visible region
/// (items outside it are culled cheaply). Text is drawn from cached glyph
/// coverage masks blended with the run color; images are scaled with bilinear
/// filtering; SVG images are rendered with resvg at the destination size.
///
/// The drawing area spans the full width of the target (`offset.0` shifts
/// the content horizontally) and is clamped to the target. A non-finite or
/// non-positive `scale` draws only the background.
pub fn rasterize(
    list: &DisplayList,
    fonts: &FontSystem,
    images: &dyn ImageProvider,
    target: &mut tiny_skia::Pixmap,
    options: &RasterOptions,
) {
    let width = target.width();
    let target_height = target.height();
    let y0 = options.offset.1.clamp(0, target_height as i32) as u32;
    let available = target_height - y0;
    let height = options.height.map_or(available, |h| h.min(available));
    if width == 0 || height == 0 {
        return;
    }
    let row_bytes = width as usize * 4;
    let start = y0 as usize * row_bytes;
    let end = start + height as usize * row_bytes;
    let Some(band) = target.data_mut().get_mut(start..end) else {
        return;
    };
    {
        let Some(mut area) = PixmapMut::from_bytes(&mut *band, width, height) else {
            return;
        };
        area.fill(shapes::skia_color(list.background));
    }

    let scale = options.scale;
    if !(scale.is_finite() && scale > 0.0) {
        return;
    }
    let scale = scale.min(MAX_SCALE);
    let off_x = options.offset.0 as f32;
    // Rows above the area (offset.1 < 0) shift the content up.
    let off_y = (options.offset.1 - y0 as i32) as f32;
    let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
    let doc = (
        off_x - finite(options.scroll.x) * scale,
        off_y - finite(options.scroll.y) * scale,
    );
    let layer_bytes = (width as usize * height as usize * 4).max(1);
    let mut painter = Painter {
        fonts,
        images,
        base: band,
        width,
        height,
        scale,
        doc,
        fixed: (off_x, off_y),
        fixed_depth: 0,
        clips: Vec::new(),
        saved_clips: Vec::new(),
        layers: Vec::new(),
        pool: Vec::new(),
        masks: MaskPool::default(),
        max_layers: (MAX_LAYER_BYTES / layer_bytes).clamp(1, MAX_LAYERS),
        skip: 0,
        full: ClipState::full(width, height),
    };
    let items = &list.items;
    let mut i = 0;
    while let Some(item) = items.get(i) {
        // A group holding a single solid-color item needs no layer: its
        // opacity is applied to the item's color instead.
        if let (DisplayItem::PushOpacity(opacity), Some(inner), Some(DisplayItem::PopOpacity)) =
            (item, items.get(i + 1), items.get(i + 2))
        {
            if let Some(faded) = fade(inner, *opacity) {
                painter.item(&faded);
                i += 3;
                continue;
            }
        }
        painter.item(item);
        i += 1;
    }
    painter.finish();
}

/// `item` with its color's alpha multiplied by `opacity`, for the single
/// solid-color items whose group opacity equals per-item alpha (`None`
/// for other items and for opacities handled otherwise: 0, 1, NaN).
fn fade(item: &DisplayItem, opacity: f32) -> Option<DisplayItem> {
    if !(opacity > 0.0 && opacity < 1.0) {
        return None;
    }
    let f = |c: Color| Color::rgba(c.r, c.g, c.b, (f32::from(c.a) * opacity).round() as u8);
    Some(match item {
        DisplayItem::Rect { rect, color, radii } => DisplayItem::Rect {
            rect: *rect,
            color: f(*color),
            radii: *radii,
        },
        DisplayItem::Ellipse {
            rect,
            color,
            fill,
            stroke_width,
        } => DisplayItem::Ellipse {
            rect: *rect,
            color: f(*color),
            fill: *fill,
            stroke_width: *stroke_width,
        },
        DisplayItem::Polygon { points, color } => DisplayItem::Polygon {
            points: points.clone(),
            color: f(*color),
        },
        DisplayItem::Polyline {
            points,
            width,
            color,
        } => DisplayItem::Polyline {
            points: points.clone(),
            width: *width,
            color: f(*color),
        },
        DisplayItem::Text(run) => DisplayItem::Text(crate::display_list::GlyphRun {
            color: f(run.color),
            ..run.clone()
        }),
        _ => return None,
    })
}

/// Draws `text` with the given font on `target` at device-pixel position
/// (`x`, `baseline`), returning the advance width in device px. Used by the
/// browser UI (address bar, tabs, status bar).
#[allow(clippy::too_many_arguments)]
pub fn draw_text(
    target: &mut tiny_skia::Pixmap,
    fonts: &FontSystem,
    font: FontId,
    size_px: f32,
    x: f32,
    baseline: f32,
    text: &str,
    color: Color,
) -> f32 {
    if !(size_px.is_finite() && size_px > 0.0) {
        return 0.0;
    }
    let run = fonts.shape(font, size_px, text, 0.0, 0.0);
    if color.is_transparent() || !x.is_finite() || !baseline.is_finite() {
        return run.width;
    }
    let (w, h) = (target.width(), target.height());
    // Glyph ink stays within a few ems of the baseline: runs far above or
    // below the target draw nothing (and keep the integer maths in range).
    if baseline < -4.0 * size_px || baseline > h as f32 + 4.0 * size_px {
        return run.width;
    }
    let clip = text::IntClip {
        x0: 0,
        y0: 0,
        x1: w as i32,
        y1: h as i32,
    };
    let base_y = baseline.round() as i32;
    for glyph in &run.glyphs {
        let pen = x + glyph.x;
        if !pen.is_finite() || pen > w as f32 + size_px || pen < -4.0 * size_px {
            continue;
        }
        if let Some(bitmap) =
            fonts.rasterize(glyph.font, glyph.glyph_id, size_px, pen - pen.floor())
        {
            text::blend_glyph(
                target.data_mut(),
                w,
                &bitmap,
                (pen.floor() as i32).saturating_add(bitmap.left),
                base_y.saturating_add(bitmap.top),
                color,
                clip,
                None,
            );
        }
    }
    run.width
}

/// An opacity group being rendered.
struct Layer {
    pixmap: Pixmap,
    opacity: f32,
    /// Region drawn into so far (device px, area-local).
    dirty: DevRect,
}

/// Rasteriser state while walking a display list.
pub(crate) struct Painter<'a> {
    fonts: &'a FontSystem,
    images: &'a dyn ImageProvider,
    /// Pixels of the drawing area (premultiplied RGBA, `width` × `height`).
    base: &'a mut [u8],
    width: u32,
    height: u32,
    scale: f32,
    /// Device translation for document coordinates.
    doc: (f32, f32),
    /// Device translation for viewport (fixed) coordinates.
    fixed: (f32, f32),
    fixed_depth: usize,
    /// Clip stack (empty = whole area).
    clips: Vec<ClipState>,
    /// Clip stacks suspended by `PushFixed`.
    saved_clips: Vec<Vec<ClipState>>,
    /// Opacity groups (`None` for groups drawn without a layer).
    layers: Vec<Option<Layer>>,
    /// Cleared layer pixmaps ready for reuse.
    pool: Vec<Pixmap>,
    /// Recycled clip/ring masks.
    masks: MaskPool,
    /// Most layers live at once (memory budget).
    max_layers: usize,
    /// Nesting depth of invisible (opacity 0) groups being skipped.
    skip: usize,
    /// The "no clip" state.
    full: ClipState,
}

/// How a shape must be clipped.
pub(crate) enum ClipUse {
    /// Entirely outside the clip: draw nothing.
    Skip,
    /// Entirely inside the clip (or only cut by the area edges).
    Free,
    /// Partially clipped: draw through this mask.
    Mask(Rc<Mask>),
}

impl<'a> Painter<'a> {
    fn item(&mut self, item: &DisplayItem) {
        match item {
            DisplayItem::PushClip { rect, radii } => {
                let dev = self.to_dev(*rect);
                let radii = shapes::scale_radii(*radii, self.scale, dev);
                let next = self.clip().push(dev, radii);
                self.clips.push(next);
            }
            DisplayItem::PopClip => {
                if let Some(state) = self.clips.pop() {
                    state.recycle(&self.masks);
                }
            }
            DisplayItem::PushOpacity(opacity) => self.push_opacity(*opacity),
            DisplayItem::PopOpacity => self.pop_opacity(),
            DisplayItem::PushFixed => {
                self.fixed_depth += 1;
                self.saved_clips.push(std::mem::take(&mut self.clips));
            }
            DisplayItem::PopFixed => {
                if self.fixed_depth > 0 {
                    self.fixed_depth -= 1;
                    let restored = self.saved_clips.pop().unwrap_or_default();
                    for state in std::mem::replace(&mut self.clips, restored) {
                        state.recycle(&self.masks);
                    }
                }
            }
            _ if self.skip > 0 => {}
            DisplayItem::Rect { rect, color, radii } => self.fill_rect(*rect, *color, *radii),
            DisplayItem::Border { rect, sides, radii } => {
                border::draw_border(self, *rect, sides, *radii)
            }
            DisplayItem::LinearGradient {
                rect,
                angle_deg,
                stops,
                radii,
            } => shapes::draw_linear_gradient(self, *rect, *angle_deg, stops, *radii),
            DisplayItem::Text(run) => text::draw_run(self, run),
            // Cull before asking the provider: pages hold many off-screen
            // images and a lookup may lock or clone.
            DisplayItem::NodeImage { rect, node } => {
                if self.rect_clip(self.to_dev(*rect)).is_some() {
                    if let Some(image) = self.images.node_image(*node) {
                        images::draw_image(self, *rect, &image);
                    }
                }
            }
            DisplayItem::UrlImage { rect, url } => {
                if self.rect_clip(self.to_dev(*rect)).is_some() {
                    if let Some(image) = self.images.url_image(url) {
                        images::draw_image(self, *rect, &image);
                    }
                }
            }
            DisplayItem::BackgroundImage {
                url,
                clip,
                origin,
                tile,
                repeat,
            } => images::draw_background(self, url, *clip, *origin, *tile, *repeat),
            DisplayItem::DeferredBackgroundImage {
                url,
                area,
                clip,
                size,
                position,
                repeat,
            } => {
                images::draw_deferred_background(self, url, *area, *clip, *size, *position, *repeat)
            }
            DisplayItem::Ellipse {
                rect,
                color,
                fill,
                stroke_width,
            } => shapes::draw_ellipse(self, *rect, *color, *fill, *stroke_width),
            DisplayItem::Polyline {
                points,
                width,
                color,
            } => shapes::draw_polyline(self, points, *width, *color),
            DisplayItem::Polygon { points, color } => shapes::draw_polygon(self, points, *color),
        }
    }

    /// Composites opacity groups left open by an unbalanced list.
    fn finish(&mut self) {
        while !self.layers.is_empty() {
            self.pop_opacity();
        }
    }

    // ---------------------------------------------------------------
    // Coordinates
    // ---------------------------------------------------------------

    fn translation(&self) -> (f32, f32) {
        if self.fixed_depth > 0 {
            self.fixed
        } else {
            self.doc
        }
    }

    /// CSS px rect → device rect (area-local).
    pub(crate) fn to_dev(&self, r: Rect) -> DevRect {
        let (tx, ty) = self.translation();
        let s = self.scale;
        let fix = |v: f32| if v.is_finite() { v } else { 0.0 };
        let (x, y) = (fix(r.x), fix(r.y));
        let (w, h) = (fix(r.width).max(0.0), fix(r.height).max(0.0));
        DevRect::new(x * s + tx, y * s + ty, (x + w) * s + tx, (y + h) * s + ty)
    }

    /// CSS px point → device point.
    pub(crate) fn to_dev_point(&self, p: Point) -> (f32, f32) {
        let (tx, ty) = self.translation();
        let fix = |v: f32| if v.is_finite() { v } else { 0.0 };
        (fix(p.x) * self.scale + tx, fix(p.y) * self.scale + ty)
    }

    pub(crate) fn scale(&self) -> f32 {
        self.scale
    }

    pub(crate) fn area(&self) -> DevRect {
        DevRect::new(0.0, 0.0, self.width as f32, self.height as f32)
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub(crate) fn fonts(&self) -> &'a FontSystem {
        self.fonts
    }

    pub(crate) fn images(&self) -> &'a dyn ImageProvider {
        self.images
    }

    pub(crate) fn masks(&self) -> &MaskPool {
        &self.masks
    }

    // ---------------------------------------------------------------
    // Clipping
    // ---------------------------------------------------------------

    pub(crate) fn clip(&self) -> &ClipState {
        self.clips.last().unwrap_or(&self.full)
    }

    /// Clipping needed for a shape with device bounds `bounds`.
    pub(crate) fn clip_for(&self, bounds: DevRect) -> ClipUse {
        let clip = self.clip();
        let visible = bounds.intersect(&self.area());
        if visible.is_empty() || !visible.intersects(&clip.rect) {
            return ClipUse::Skip;
        }
        if !clip.rect.contains(&visible) || clip.needs_mask_within(&visible) {
            match clip.mask(self.width, self.height, &self.masks) {
                Some(mask) => ClipUse::Mask(mask),
                None => ClipUse::Skip,
            }
        } else {
            ClipUse::Free
        }
    }

    /// For shapes clipped geometrically to a rect: the visible part of
    /// `bounds` and the rounded-clip mask to apply, if any. `None` when
    /// nothing is visible.
    pub(crate) fn rect_clip(&self, bounds: DevRect) -> Option<(DevRect, Option<Rc<Mask>>)> {
        let clip = self.clip();
        let visible = bounds.intersect(&self.area()).intersect(&clip.rect);
        if visible.is_empty() {
            return None;
        }
        let mask = if clip.needs_mask_within(&visible) {
            Some(clip.mask(self.width, self.height, &self.masks)?)
        } else {
            None
        };
        Some((visible, mask))
    }

    // ---------------------------------------------------------------
    // Targets and opacity layers
    // ---------------------------------------------------------------

    /// The pixmap drawn into (top opacity layer, or the area).
    pub(crate) fn target(&mut self) -> Option<PixmapMut<'_>> {
        if let Some(layer) = self.layers.iter_mut().rev().flatten().next() {
            return Some(layer.pixmap.as_mut());
        }
        PixmapMut::from_bytes(self.base, self.width, self.height)
    }

    /// Records that `r` (device) was drawn into the current target. Every
    /// drawing must stay inside the rects it marks: layers are composited
    /// and cleared over their dirty region only. A one-pixel margin absorbs
    /// anti-aliasing.
    pub(crate) fn mark_dirty(&mut self, r: DevRect) {
        if r.is_empty() {
            return;
        }
        let area = self.area();
        if let Some(layer) = self.layers.iter_mut().rev().flatten().next() {
            layer.dirty = layer.dirty.union(&r.outset(1.0).intersect(&area));
        }
    }

    fn push_opacity(&mut self, opacity: f32) {
        if self.skip > 0 {
            self.skip += 1;
            return;
        }
        let opacity = if opacity.is_nan() {
            1.0
        } else {
            opacity.clamp(0.0, 1.0)
        };
        if opacity <= 0.0 {
            self.skip = 1;
            return;
        }
        let live = self.layers.iter().flatten().count();
        if opacity >= 1.0 || live >= self.max_layers {
            self.layers.push(None);
            return;
        }
        let pixmap = self
            .pool
            .pop()
            .or_else(|| Pixmap::new(self.width, self.height));
        self.layers.push(pixmap.map(|pixmap| Layer {
            pixmap,
            opacity,
            dirty: DevRect::EMPTY,
        }));
    }

    fn pop_opacity(&mut self) {
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        let Some(Some(mut layer)) = self.layers.pop() else {
            return;
        };
        let dirty = layer.dirty.round_out().intersect(&self.area());
        if !dirty.is_empty() {
            let width = self.width;
            if let Some(mut target) = self.target() {
                composite(
                    target.data_mut(),
                    layer.pixmap.data(),
                    width,
                    dirty,
                    layer.opacity,
                );
            }
            self.mark_dirty(dirty);
            clear(layer.pixmap.data_mut(), width, dirty);
        }
        self.pool.push(layer.pixmap);
    }

    // ---------------------------------------------------------------
    // Rectangles
    // ---------------------------------------------------------------

    fn fill_rect(&mut self, rect: Rect, color: Color, radii: [f32; 4]) {
        if color.is_transparent() {
            return;
        }
        let dev = self.to_dev(rect);
        let radii = shapes::scale_radii(radii, self.scale, dev);
        if radii.iter().all(|&r| r <= 0.0) {
            self.fill_dev_rect(dev.snap(), color);
        } else {
            shapes::fill_rounded(self, dev, radii, color);
        }
    }

    /// Fills an axis-aligned device rect (geometrically clipped).
    pub(crate) fn fill_dev_rect(&mut self, dev: DevRect, color: Color) {
        let Some((visible, mask)) = self.rect_clip(dev) else {
            return;
        };
        let Some(rect) = visible.to_skia() else {
            return;
        };
        let paint = shapes::solid_paint(color);
        if let Some(mut target) = self.target() {
            target.fill_rect(
                rect,
                &paint,
                tiny_skia::Transform::identity(),
                mask.as_deref(),
            );
        }
        self.mark_dirty(visible);
    }
}

/// `px × f / 255` on each 8-bit channel of a packed pixel (`f` ≤ 255),
/// rounded exactly like [`div255`]: two channels are processed at once in
/// 16-bit lanes (`c × f + 128` stays below 2¹⁶, so lanes never carry).
#[inline]
pub(crate) fn mul_px(px: u32, f: u32) -> u32 {
    let rb = (px & 0x00ff_00ff) * f + 0x0080_0080;
    let rb = ((rb + ((rb >> 8) & 0x00ff_00ff)) >> 8) & 0x00ff_00ff;
    let ag = ((px >> 8) & 0x00ff_00ff) * f + 0x0080_0080;
    let ag = (ag + ((ag >> 8) & 0x00ff_00ff)) & 0xff00_ff00;
    rb | ag
}

/// Source-over composites `layer` (premultiplied, same size as `dst`) with
/// `opacity` over `dst`, within `rect` (integer-aligned, inside the area).
/// Pixels are processed as packed words (channel sums of valid
/// premultiplied colors never exceed 255, so they cannot carry).
fn composite(dst: &mut [u8], layer: &[u8], width: u32, rect: DevRect, opacity: f32) {
    let op = (opacity * 255.0).round() as u32;
    if op == 0 {
        return;
    }
    let (x0, x1) = (rect.x0 as usize, rect.x1 as usize);
    let stride = width as usize * 4;
    for y in rect.y0 as usize..rect.y1 as usize {
        let range = y * stride + x0 * 4..y * stride + x1 * 4;
        let (Some(d), Some(s)) = (dst.get_mut(range.clone()), layer.get(range)) else {
            continue;
        };
        composite_row(d, s, op);
    }
}

/// One row of [`composite`] (whole pixels of `dst` and `src`).
#[inline]
fn composite_row(dst: &mut [u8], src: &[u8], op: u32) {
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let s = mul_px(u32::from_le_bytes([s[0], s[1], s[2], s[3]]), op);
        let out = s + mul_px(
            u32::from_le_bytes([d[0], d[1], d[2], d[3]]),
            255 - (s >> 24),
        );
        d.copy_from_slice(&out.to_le_bytes());
    }
}

/// Zeroes `rect` (integer-aligned, inside the pixmap) of premultiplied
/// RGBA `data` of width `width`.
fn clear(data: &mut [u8], width: u32, rect: DevRect) {
    let (x0, x1) = (rect.x0 as usize, rect.x1 as usize);
    let stride = width as usize * 4;
    for y in rect.y0 as usize..rect.y1 as usize {
        if let Some(row) = data.get_mut(y * stride + x0 * 4..y * stride + x1 * 4) {
            row.fill(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearing_a_region() {
        let mut data = vec![9u8; 3 * 2 * 4];
        clear(&mut data, 3, DevRect::new(1.0, 1.0, 3.0, 2.0));
        assert_eq!(&data[..12], &[9; 12]);
        assert_eq!(&data[12..16], &[9; 4]);
        assert_eq!(&data[16..], &[0; 8]);
    }

    #[test]
    fn div255_is_exact() {
        for x in 0..=255u32 * 255 {
            assert_eq!(div255(x), (x as f32 / 255.0).round() as u32, "{x}");
        }
    }

    #[test]
    fn packed_multiply_matches_div255() {
        for c in 0..=255u32 {
            for f in [0u32, 1, 77, 128, 200, 254, 255] {
                let px = c | ((255 - c) << 8) | (c << 16) | (c / 2) << 24;
                let out = mul_px(px, f);
                assert_eq!(out & 0xff, div255(c * f));
                assert_eq!((out >> 8) & 0xff, div255((255 - c) * f));
                assert_eq!((out >> 16) & 0xff, div255(c * f));
                assert_eq!(out >> 24, div255(c / 2 * f));
            }
        }
    }

    #[test]
    fn composite_half() {
        let mut dst = vec![255u8; 4];
        let layer = vec![0u8, 0, 255, 255];
        composite(&mut dst, &layer, 1, DevRect::new(0.0, 0.0, 1.0, 1.0), 0.5);
        assert_eq!(dst, vec![127, 127, 255, 255]);
    }
}
