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
//! * images with bilinear filtering, SVG rendered by resvg at the
//!   destination size (bounded cache);
//! * clips as device rects (geometric clipping) plus lazily built masks
//!   for rounded clips; `PushFixed` starts a fresh clip stack because fixed
//!   boxes are not clipped by their ancestors' overflow;
//! * opacity groups in transparent layers composited on `PopOpacity`
//!   (only their dirty region).

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
use clip::{ClipState, DevRect};

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
        skip: 0,
        full: ClipState::full(width, height),
    };
    for item in &list.items {
        painter.item(item);
    }
    painter.finish();
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
    let clip = text::IntClip {
        x0: 0,
        y0: 0,
        x1: w as i32,
        y1: h as i32,
    };
    let base_y = baseline.round();
    for glyph in &run.glyphs {
        let pen = x + glyph.x;
        if pen > w as f32 + size_px || pen < -4.0 * size_px {
            continue;
        }
        if let Some(bitmap) =
            fonts.rasterize(glyph.font, glyph.glyph_id, size_px, pen - pen.floor())
        {
            text::blend_glyph(
                target.data_mut(),
                w,
                &bitmap,
                pen.floor() as i32 + bitmap.left,
                base_y as i32 + bitmap.top,
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
    /// Opacity groups (`None` for groups that need no layer).
    layers: Vec<Option<Layer>>,
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
                self.clips.pop();
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
                    self.clips = self.saved_clips.pop().unwrap_or_default();
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
            DisplayItem::NodeImage { rect, node } => {
                if let Some(image) = self.images.node_image(*node) {
                    images::draw_image(self, *rect, &image);
                }
            }
            DisplayItem::UrlImage { rect, url } => {
                if let Some(image) = self.images.url_image(url) {
                    images::draw_image(self, *rect, &image);
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
        if clip.has_rounded() || !clip.rect.contains(&visible) {
            match clip.mask(self.width, self.height) {
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
        let mask = if clip.has_rounded() {
            Some(clip.mask(self.width, self.height)?)
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

    /// Records that `r` (device) was drawn into the current target.
    pub(crate) fn mark_dirty(&mut self, r: DevRect) {
        if let Some(layer) = self.layers.iter_mut().rev().flatten().next() {
            layer.dirty = layer.dirty.union(&r.intersect(&DevRect::new(
                0.0,
                0.0,
                self.width as f32,
                self.height as f32,
            )));
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
        if opacity >= 1.0 {
            self.layers.push(None);
            return;
        }
        let layer = Pixmap::new(self.width, self.height).map(|pixmap| Layer {
            pixmap,
            opacity,
            dirty: DevRect::EMPTY,
        });
        self.layers.push(layer);
    }

    fn pop_opacity(&mut self) {
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        let Some(Some(layer)) = self.layers.pop() else {
            return;
        };
        let dirty = layer.dirty.round_out().intersect(&self.area());
        if dirty.is_empty() {
            return;
        }
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

/// Source-over composites `layer` (premultiplied, same size as `dst`) with
/// `opacity` over `dst`, within `rect` (integer-aligned, inside the area).
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
        for (d, s) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
            let sa = u32::from(s[3]);
            if sa == 0 {
                continue;
            }
            let a = div255(sa * op);
            let inv = 255 - a;
            for i in 0..3 {
                d[i] =
                    (div255(u32::from(s[i]) * op) + div255(u32::from(d[i]) * inv)).min(255) as u8;
            }
            d[3] = (a + div255(u32::from(d[3]) * inv)).min(255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn div255_is_exact() {
        for x in 0..=255u32 * 255 {
            assert_eq!(div255(x), (x as f32 / 255.0).round() as u32, "{x}");
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
