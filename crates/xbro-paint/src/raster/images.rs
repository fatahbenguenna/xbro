//! Images: scaled replaced content, tiled backgrounds and SVG rendering
//! (with a bounded cache of rendered SVG pixmaps), plus a bounded cache of
//! box-filtered reductions for raster images drawn much smaller than their
//! natural size.
//!
//! An SVG image is rendered whole at its destination size when that is at
//! most [`SVG_WHOLE_AREA`] device px; larger destinations only render the
//! part that is visible (expanded to a [`SVG_PART_GRID`] so that small
//! scrolls reuse the rendering). resvg's cost grows with the rendered area
//! (a blur filter over 4096 × 4096 px takes seconds), so it is bounded by
//! the size of the drawing area instead of the size of the image. SVG
//! filters are the exception: resvg computes them over the filtered
//! element's whole region at the rendering scale, whatever part is drawn,
//! so SVGs with filters are always rendered whole, at most
//! [`MAX_FILTERED_SVG_AREA`] px, and scaled up (filters are mostly blurs
//! and shadows, which lose little).

use std::sync::{Arc, Mutex, Weak};

use tiny_skia::{FilterQuality, Paint, Pixmap, SpreadMode, Transform};
use xbro_layout::{Point, Rect, Size};
use xbro_style::{BackgroundRepeat, BackgroundSize, LengthPercentage};

use super::clip::DevRect;
use super::shapes::fill_rect_aa;
use super::Painter;
use crate::display_list::background_tile;
use crate::image::{with_svg_stack, Image};

/// Largest side of an SVG rendering, in device px (larger destinations
/// are rendered at this resolution and scaled up).
const MAX_SVG_SIDE: f32 = 4096.0;
/// Largest SVG rendering area, in device px.
const MAX_SVG_AREA: f32 = 4096.0 * 4096.0;
/// Largest rendering area of an SVG using filters, in device px.
const MAX_FILTERED_SVG_AREA: f32 = 512.0 * 512.0;
/// Maximum number of cached SVG renderings.
const SVG_CACHE_ENTRIES: usize = 64;
/// Maximum memory held by cached SVG renderings.
const SVG_CACHE_BYTES: usize = 64 << 20;

/// Largest destination area (device px) at which an SVG image is
/// rendered whole; larger ones render their visible part only.
const SVG_WHOLE_AREA: f32 = 1024.0 * 1024.0;
/// The visible part of a large SVG destination is expanded to multiples
/// of this many device px from the destination's top-left corner.
const SVG_PART_GRID: f32 = 256.0;
/// Most tile copies of a huge SVG background tile drawn one by one.
const MAX_SVG_TILE_COPIES: usize = 256;

/// What a cached SVG rendering shows.
#[derive(Copy, Clone, Debug, PartialEq)]
enum SvgView {
    /// The whole image at this pixel size.
    Whole { width: u32, height: u32 },
    /// The `width` × `height` px rectangle at (`x`, `y`) of the image drawn
    /// at `dest_w` × `dest_h` device px (sizes as f32 bits).
    Part {
        dest_w: u32,
        dest_h: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

impl SvgView {
    fn pixel_size(self) -> (u32, u32) {
        match self {
            SvgView::Whole { width, height } | SvgView::Part { width, height, .. } => {
                (width, height)
            }
        }
    }
}

/// One cached rendering of an SVG tree.
struct SvgEntry {
    tree: Weak<resvg::usvg::Tree>,
    view: SvgView,
    pixmap: Arc<Pixmap>,
}

/// Rendered SVGs, oldest first.
static SVG_CACHE: Mutex<Vec<SvgEntry>> = Mutex::new(Vec::new());

/// Renders `view` of `tree`, using the cache.
fn render_svg(tree: &Arc<resvg::usvg::Tree>, view: SvgView) -> Option<Arc<Pixmap>> {
    let mut cache = SVG_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|e| e.tree.strong_count() > 0);
    if let Some(pos) = cache
        .iter()
        .position(|e| e.view == view && e.tree.upgrade().is_some_and(|t| Arc::ptr_eq(&t, tree)))
    {
        let entry = cache.remove(pos);
        let pixmap = entry.pixmap.clone();
        cache.push(entry);
        return Some(pixmap);
    }
    drop(cache);

    let pixmap = Arc::new(render_svg_uncached(tree, view)?);
    let mut cache = SVG_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.push(SvgEntry {
        tree: Arc::downgrade(tree),
        view,
        pixmap: pixmap.clone(),
    });
    let bytes = |c: &Vec<SvgEntry>| c.iter().map(|e| e.pixmap.data().len()).sum::<usize>();
    while cache.len() > SVG_CACHE_ENTRIES || (cache.len() > 1 && bytes(&cache) > SVG_CACHE_BYTES) {
        cache.remove(0);
    }
    Some(pixmap)
}

fn render_svg_uncached(tree: &resvg::usvg::Tree, view: SvgView) -> Option<Pixmap> {
    let size = tree.size();
    let (sw, sh) = (size.width(), size.height());
    if !(sw > 0.0 && sh > 0.0) {
        return None;
    }
    let (width, height) = view.pixel_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    let transform = match view {
        SvgView::Whole { width, height } => svg_fit(sw, sh, width as f32, height as f32),
        SvgView::Part {
            dest_w,
            dest_h,
            x,
            y,
            ..
        } => svg_fit(sw, sh, f32::from_bits(dest_w), f32::from_bits(dest_h))
            .post_translate(-(x as f32), -(y as f32)),
    };
    if !within_rasterizer_limits(tree, transform) {
        return None;
    }
    // resvg recurses like usvg does: render on the large SVG stack (which
    // also contains panics).
    let target = &mut pixmap;
    with_svg_stack(move || resvg::render(tree, transform, &mut target.as_mut()))?;
    // resvg uses its own tiny-skia version; both store premultiplied RGBA8.
    let size = tiny_skia::IntSize::from_wh(width, height)?;
    Pixmap::from_vec(pixmap.take(), size)
}

/// Largest distance from the origin, in device px, of the content of a
/// rendered SVG (its stroke and filter bounds included).
const MAX_SVG_EXTENT: f32 = 1e8;

/// False when `tree` drawn with `transform` reaches geometry that resvg's
/// tiny-skia cannot rasterise: it panics on coordinates around 1e10 px
/// (`stroke-width="1e6"` in a 10-unit view box drawn 100 000 px wide).
/// Such content is not drawn at all.
fn within_rasterizer_limits(
    tree: &resvg::usvg::Tree,
    transform: resvg::tiny_skia::Transform,
) -> bool {
    let Some(bounds) = tree.root().abs_layer_bounding_box().transform(transform) else {
        return false;
    };
    [bounds.left(), bounds.top(), bounds.right(), bounds.bottom()]
        .iter()
        .all(|v| v.abs() <= MAX_SVG_EXTENT)
}

/// Maps an SVG of size `sw` × `sh` into a `w` × `h` viewport the way
/// browsers draw SVG images with the default `preserveAspectRatio`
/// (`xMidYMid meet`): uniformly scaled to fit and centred. A mismatch of
/// less than two pixels (from rounding device sizes) is absorbed by
/// stretching instead, so no transparent hairline appears at the edges.
fn svg_fit(sw: f32, sh: f32, w: f32, h: f32) -> resvg::tiny_skia::Transform {
    let (sx, sy) = (w / sw, h / sh);
    let s = sx.min(sy);
    let (cw, ch) = (sw * s, sh * s);
    if (w - cw).abs() < 2.0 && (h - ch).abs() < 2.0 {
        return resvg::tiny_skia::Transform::from_scale(sx, sy);
    }
    resvg::tiny_skia::Transform::from_row(s, 0.0, 0.0, s, (w - cw) / 2.0, (h - ch) / 2.0)
}

/// Pixel size at which to render an SVG drawn into `w` × `h` device px,
/// with at most `max_area` pixels.
fn svg_render_size(w: f32, h: f32, max_area: f32) -> Option<(u32, u32)> {
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let mut f = 1.0f32;
    if w.max(h) > MAX_SVG_SIDE {
        f = MAX_SVG_SIDE / w.max(h);
    }
    if w * h * f * f > max_area {
        f = f.min((max_area / (w * h)).sqrt());
    }
    let rw = (w * f).round().clamp(1.0, MAX_SVG_SIDE) as u32;
    let rh = (h * f).round().clamp(1.0, MAX_SVG_SIDE) as u32;
    Some((rw, rh))
}

/// A pixmap of `image` suitable for drawing at `w` × `h` device px:
/// raster images drawn at less than half their size use a box-filtered
/// reduction (bilinear sampling alone skips most source pixels and
/// shimmers with moiré), SVG images are rendered at that size.
fn pixmap_for(image: &Image, w: f32, h: f32) -> Option<Arc<Pixmap>> {
    match image {
        Image::Raster(p) => match mip_level(p.width(), p.height(), w, h) {
            0 => Some(p.clone()),
            level => Some(reduced(p, level).unwrap_or_else(|| p.clone())),
        },
        Image::Svg(tree) => {
            let max_area = if tree.filters().is_empty() {
                MAX_SVG_AREA
            } else {
                MAX_FILTERED_SVG_AREA
            };
            let (width, height) = svg_render_size(w, h, max_area)?;
            render_svg(tree, SvgView::Whole { width, height })
        }
    }
}

/// Maximum number of cached reductions of raster images.
const MIP_CACHE_ENTRIES: usize = 32;
/// Maximum memory held by cached reductions.
const MIP_CACHE_BYTES: usize = 64 << 20;
/// Deepest reduction (a 65536× smaller image).
const MAX_MIP_LEVEL: u32 = 16;

/// One cached reduction of a raster image (halved `level` times).
struct MipEntry {
    source: Weak<Pixmap>,
    level: u32,
    pixmap: Arc<Pixmap>,
}

/// Reductions of raster images, oldest first.
static MIP_CACHE: Mutex<Vec<MipEntry>> = Mutex::new(Vec::new());

/// How many times a `sw` × `sh` pixmap drawn at `w` × `h` device px
/// should be halved first so that the remaining scale on the less reduced
/// axis stays above one half (0 when it is not reduced that much).
fn mip_level(sw: u32, sh: u32, w: f32, h: f32) -> u32 {
    if sw == 0 || sh == 0 || !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return 0;
    }
    let scale = (w / sw as f32).max(h / sh as f32);
    if !(scale > 0.0 && scale <= 0.5) {
        return 0;
    }
    let mut level = 0;
    let mut s = scale;
    // Halving stops at 1 px on the smaller side.
    while s <= 0.5 && level < MAX_MIP_LEVEL && (sw.min(sh) >> (level + 1)) >= 1 {
        s *= 2.0;
        level += 1;
    }
    level
}

/// `source` reduced `level` times, from the cache or built (from the
/// deepest cached shallower reduction).
fn reduced(source: &Arc<Pixmap>, level: u32) -> Option<Arc<Pixmap>> {
    let mut cache = MIP_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|e| e.source.strong_count() > 0);
    let same = |e: &MipEntry| e.source.upgrade().is_some_and(|s| Arc::ptr_eq(&s, source));
    if let Some(pos) = cache.iter().position(|e| e.level == level && same(e)) {
        let entry = cache.remove(pos);
        let pixmap = entry.pixmap.clone();
        cache.push(entry);
        return Some(pixmap);
    }
    let (mut current, mut done) = cache
        .iter()
        .filter(|e| e.level < level && same(e))
        .max_by_key(|e| e.level)
        .map_or((source.clone(), 0), |e| (e.pixmap.clone(), e.level));
    drop(cache);
    while done < level {
        current = Arc::new(halve(&current)?);
        done += 1;
    }
    let mut cache = MIP_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.push(MipEntry {
        source: Arc::downgrade(source),
        level,
        pixmap: current.clone(),
    });
    let bytes = |c: &Vec<MipEntry>| c.iter().map(|e| e.pixmap.data().len()).sum::<usize>();
    while cache.len() > MIP_CACHE_ENTRIES || (cache.len() > 1 && bytes(&cache) > MIP_CACHE_BYTES) {
        cache.remove(0);
    }
    Some(current)
}

/// Halves a premultiplied pixmap with a 2×2 box filter (an odd last
/// row/column is averaged with itself).
fn halve(src: &Pixmap) -> Option<Pixmap> {
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    if sw == 0 || sh == 0 {
        return None;
    }
    let (dw, dh) = (sw.div_ceil(2), sh.div_ceil(2));
    let mut dst = Pixmap::new(dw as u32, dh as u32)?;
    let data = src.data();
    let stride = sw * 4;
    for (y, out_row) in dst.data_mut().chunks_exact_mut(dw * 4).enumerate() {
        let (y0, y1) = (2 * y, (2 * y + 1).min(sh - 1));
        let (Some(row0), Some(row1)) = (
            data.get(y0 * stride..(y0 + 1) * stride),
            data.get(y1 * stride..(y1 + 1) * stride),
        ) else {
            continue;
        };
        for (x, out) in out_row.chunks_exact_mut(4).enumerate() {
            let (x0, x1) = (2 * x * 4, (2 * x + 1).min(sw - 1) * 4);
            for (c, v) in out.iter_mut().enumerate() {
                let at = |row: &[u8], i: usize| row.get(i + c).copied().map_or(0, u32::from);
                let sum = at(row0, x0) + at(row0, x1) + at(row1, x0) + at(row1, x1);
                *v = ((sum + 2) / 4) as u8;
            }
        }
    }
    Some(dst)
}

/// Filter quality: nearest when the pixmap maps 1:1 onto whole pixels
/// (crisp sprites, faster), bilinear otherwise.
fn quality(ts: &Transform) -> FilterQuality {
    let unit = |v: f32| (v - 1.0).abs() < 1e-4;
    let whole = |v: f32| (v - v.round()).abs() < 1e-3;
    if unit(ts.sx) && unit(ts.sy) && ts.kx == 0.0 && ts.ky == 0.0 && whole(ts.tx) && whole(ts.ty) {
        FilterQuality::Nearest
    } else {
        FilterQuality::Bilinear
    }
}

/// Fills `region` (device) with `pixmap` mapped by `ts`, through the clip.
fn fill_with_pixmap(
    p: &mut Painter<'_>,
    region: DevRect,
    pixmap: &Pixmap,
    ts: Transform,
    spread: SpreadMode,
) {
    let Some((visible, mask)) = p.rect_clip(region) else {
        return;
    };
    let paint = Paint {
        shader: tiny_skia::Pattern::new(pixmap.as_ref(), spread, quality(&ts), 1.0, ts),
        anti_alias: true,
        ..Paint::default()
    };
    if let Some(mut target) = p.target() {
        fill_rect_aa(&mut target, visible, &paint, mask.as_deref());
    }
    p.mark_dirty(visible);
}

/// True when an SVG drawn into `w` × `h` device px only renders its
/// visible part (large, and without filters).
fn svg_drawn_in_parts(image: &Image, w: f32, h: f32) -> bool {
    matches!(image, Image::Svg(tree) if tree.filters().is_empty()) && w * h > SVG_WHOLE_AREA
}

/// The part of a `dest` rectangle (device px) to render for its `visible`
/// part: relative to `dest`'s top-left corner, expanded to the grid and
/// clipped to `dest`, as whole pixels `(x, y, width, height)`.
fn svg_part(dest: DevRect, visible: DevRect) -> Option<(u32, u32, u32, u32)> {
    let g = SVG_PART_GRID;
    let (dw, dh) = (dest.width().ceil(), dest.height().ceil());
    let snap_down = |v: f32, max: f32| ((v / g).floor() * g).clamp(0.0, max);
    let snap_up = |v: f32, max: f32| ((v / g).ceil() * g).clamp(0.0, max);
    let x0 = snap_down(visible.x0 - dest.x0, dw);
    let y0 = snap_down(visible.y0 - dest.y0, dh);
    let x1 = snap_up(visible.x1 - dest.x0, dw);
    let y1 = snap_up(visible.y1 - dest.y0, dh);
    let (w, h) = (x1 - x0, y1 - y0);
    if !(w >= 1.0 && h >= 1.0) {
        return None;
    }
    Some((x0 as u32, y0 as u32, w as u32, h as u32))
}

/// Draws the part of the SVG `tree`, drawn into `dest` (device px), that
/// is visible through `within` and the clip.
fn draw_svg_part(
    p: &mut Painter<'_>,
    tree: &Arc<resvg::usvg::Tree>,
    dest: DevRect,
    within: DevRect,
) {
    let Some((visible, _)) = p.rect_clip(dest.intersect(&within)) else {
        return;
    };
    let Some((x, y, width, height)) = svg_part(dest, visible) else {
        return;
    };
    // The device size of the destination is the difference of two large
    // coordinates that move with scrolling: quantized, so that rounding
    // noise does not defeat the cache.
    let quantize = |v: f32| ((v * 16.0).round() / 16.0).to_bits();
    let view = SvgView::Part {
        dest_w: quantize(dest.width()),
        dest_h: quantize(dest.height()),
        x,
        y,
        width,
        height,
    };
    let Some(pixmap) = render_svg(tree, view) else {
        return;
    };
    let (ox, oy) = (dest.x0 + x as f32, dest.y0 + y as f32);
    let part = DevRect::new(ox, oy, ox + width as f32, oy + height as f32);
    let ts = Transform::from_translate(ox, oy);
    fill_with_pixmap(p, part.intersect(&visible), &pixmap, ts, SpreadMode::Pad);
}

/// Draws `image` scaled into `rect` (CSS px).
pub(crate) fn draw_image(p: &mut Painter<'_>, rect: Rect, image: &Image) {
    let dest = p.to_dev(rect);
    if dest.is_empty() || p.rect_clip(dest).is_none() {
        return;
    }
    if let Image::Svg(tree) = image {
        if svg_drawn_in_parts(image, dest.width(), dest.height()) {
            draw_svg_part(p, tree, dest, dest);
            return;
        }
    }
    let Some(pixmap) = pixmap_for(image, dest.width(), dest.height()) else {
        return;
    };
    let sx = dest.width() / pixmap.width() as f32;
    let sy = dest.height() / pixmap.height() as f32;
    if !(sx.is_finite() && sy.is_finite() && sx > 0.0 && sy > 0.0) {
        return;
    }
    let ts = Transform::from_row(sx, 0.0, 0.0, sy, dest.x0, dest.y0);
    fill_with_pixmap(p, dest, &pixmap, ts, SpreadMode::Pad);
}

/// Draws a resolved background image (`tile` zero = natural size).
pub(crate) fn draw_background(
    p: &mut Painter<'_>,
    url: &str,
    clip: Rect,
    origin: Point,
    tile: Size,
    repeat: BackgroundRepeat,
) {
    let clip_dev = p.to_dev(clip);
    if clip_dev.is_empty() || p.rect_clip(clip_dev).is_none() {
        return;
    }
    let Some(image) = p.images().url_image(url) else {
        return;
    };
    let tile = if tile.width <= 0.0 || tile.height <= 0.0 {
        image.natural_size()
    } else {
        tile
    };
    tile_image(p, &image, clip_dev, origin, tile, repeat);
}

/// Draws a background whose geometry is resolved now from the image's natural size.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_deferred_background(
    p: &mut Painter<'_>,
    url: &str,
    area: Rect,
    clip: Rect,
    size: BackgroundSize,
    position: (LengthPercentage, LengthPercentage),
    repeat: BackgroundRepeat,
) {
    let clip_dev = p.to_dev(clip);
    if clip_dev.is_empty() || p.rect_clip(clip_dev).is_none() {
        return;
    }
    let Some(image) = p.images().url_image(url) else {
        return;
    };
    let Some((origin, tile)) = background_tile(area, image.natural_size(), size, position) else {
        return;
    };
    tile_image(p, &image, clip_dev, origin, tile, repeat);
}

/// Tiles `image` from `origin` (CSS px) with tile size `tile` (CSS px)
/// within `clip_dev` (device).
fn tile_image(
    p: &mut Painter<'_>,
    image: &Image,
    clip_dev: DevRect,
    origin: Point,
    tile: Size,
    repeat: BackgroundRepeat,
) {
    let scale = p.scale();
    let (ox, oy) = p.to_dev_point(origin);
    let (tw, th) = (tile.width * scale, tile.height * scale);
    // Sub-0.01px tiles would repeat millions of times for nothing visible.
    if !(tw.is_finite() && th.is_finite() && tw >= 0.01 && th >= 0.01) {
        return;
    }
    let tile_rect = DevRect::new(ox, oy, ox + tw, oy + th);
    let region = match repeat {
        BackgroundRepeat::Repeat => clip_dev,
        BackgroundRepeat::RepeatX => {
            clip_dev.intersect(&DevRect::new(clip_dev.x0, oy, clip_dev.x1, oy + th))
        }
        BackgroundRepeat::RepeatY => {
            clip_dev.intersect(&DevRect::new(ox, clip_dev.y0, ox + tw, clip_dev.y1))
        }
        BackgroundRepeat::NoRepeat => clip_dev.intersect(&tile_rect),
    };
    if region.is_empty() || p.rect_clip(region).is_none() {
        return;
    }
    if let Image::Svg(tree) = image {
        if svg_drawn_in_parts(image, tw, th) {
            draw_svg_tiles(p, tree, region, (ox, oy), (tw, th), repeat);
            return;
        }
    }
    let Some(pixmap) = pixmap_for(image, tw, th) else {
        return;
    };
    let sx = tw / pixmap.width() as f32;
    let sy = th / pixmap.height() as f32;
    if !(sx.is_finite() && sy.is_finite() && sx > 0.0 && sy > 0.0) {
        return;
    }
    // Keep the pattern origin near the region along repeating axes so f32
    // precision holds far down long pages.
    let (ox, oy) = match repeat {
        BackgroundRepeat::Repeat => (rebase(ox, tw, region.x0), rebase(oy, th, region.y0)),
        BackgroundRepeat::RepeatX => (rebase(ox, tw, region.x0), oy),
        BackgroundRepeat::RepeatY => (ox, rebase(oy, th, region.y0)),
        BackgroundRepeat::NoRepeat => (ox, oy),
    };
    let ts = Transform::from_row(sx, 0.0, 0.0, sy, ox, oy);
    let spread = if repeat == BackgroundRepeat::NoRepeat {
        SpreadMode::Pad
    } else {
        SpreadMode::Repeat
    };
    fill_with_pixmap(p, region, &pixmap, ts, spread);
}

/// Tiles an SVG whose tiles are too large to be rendered whole: each tile
/// copy meeting `region` (device) draws its visible part.
fn draw_svg_tiles(
    p: &mut Painter<'_>,
    tree: &Arc<resvg::usvg::Tree>,
    region: DevRect,
    (ox, oy): (f32, f32),
    (tw, th): (f32, f32),
    repeat: BackgroundRepeat,
) {
    let copies = |origin: f32, tile: f32, from: f32, to: f32, repeats: bool| {
        if !repeats {
            return (0.0, 0.0);
        }
        let first = ((from - origin) / tile).floor();
        let last = ((to - origin) / tile).ceil() - 1.0;
        (first, last.max(first))
    };
    let repeat_x = matches!(repeat, BackgroundRepeat::Repeat | BackgroundRepeat::RepeatX);
    let repeat_y = matches!(repeat, BackgroundRepeat::Repeat | BackgroundRepeat::RepeatY);
    let (i0, i1) = copies(ox, tw, region.x0, region.x1, repeat_x);
    let (j0, j1) = copies(oy, th, region.y0, region.y1, repeat_y);
    if !(i0.is_finite() && i1.is_finite() && j0.is_finite() && j1.is_finite()) {
        return;
    }
    let mut drawn = 0usize;
    let mut j = j0;
    while j <= j1 {
        let mut i = i0;
        while i <= i1 {
            drawn += 1;
            if drawn > MAX_SVG_TILE_COPIES {
                return;
            }
            let (x, y) = (ox + i * tw, oy + j * th);
            draw_svg_part(p, tree, DevRect::new(x, y, x + tw, y + th), region);
            i += 1.0;
        }
        j += 1.0;
    }
}

/// Moves a tiling origin by whole tiles so it lies at or before `target`
/// and within one tile of it.
fn rebase(origin: f32, tile: f32, target: f32) -> f32 {
    if origin <= target && target - origin < tile {
        return origin;
    }
    let n = ((target - origin) / tile).floor();
    if n.is_finite() {
        origin + n * tile
    } else {
        origin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_sizes() {
        let size = |w, h| svg_render_size(w, h, MAX_SVG_AREA);
        assert_eq!(size(10.4, 20.6), Some((10, 21)));
        assert_eq!(size(0.0, 20.0), None);
        assert_eq!(size(f32::NAN, 20.0), None);
        let (w, h) = size(100_000.0, 50_000.0).unwrap();
        assert!(w <= 4096 && h <= 4096 && (w as f32 * h as f32) <= MAX_SVG_AREA);
        assert_eq!(size(0.2, 0.2), Some((1, 1)));
        let (w, h) = svg_render_size(2000.0, 1000.0, MAX_FILTERED_SVG_AREA).unwrap();
        assert!(
            (w as f32 * h as f32) <= MAX_FILTERED_SVG_AREA * 1.01 && w == 2 * h,
            "{w}x{h}"
        );
    }

    #[test]
    fn svg_parts_snap_to_the_grid() {
        let dest = DevRect::new(-1000.5, 10.0, 9000.0, 5010.0);
        // Visible part expanded to 256-px cells of the destination.
        let part = svg_part(dest, DevRect::new(0.0, 20.0, 300.0, 100.0)).unwrap();
        assert_eq!(part, (768, 0, 768, 256));
        // A small scroll stays in the same cells.
        assert_eq!(
            svg_part(dest, DevRect::new(5.0, 30.0, 305.0, 110.0)),
            Some(part)
        );
        // Clamped to the destination (its size rounded up).
        let end = svg_part(dest, DevRect::new(8900.0, 4900.0, 9000.0, 5010.0)).unwrap();
        assert_eq!(end, (9728, 4864, 273, 136));
        assert_eq!(svg_part(dest, DevRect::EMPTY), None);
    }

    #[test]
    fn svg_fitting() {
        // Same aspect ratio (or within a pixel): plain scaling.
        let t = svg_fit(20.0, 10.0, 40.0, 20.0);
        assert_eq!((t.sx, t.sy, t.tx, t.ty), (2.0, 2.0, 0.0, 0.0));
        let t = svg_fit(20.0, 10.0, 41.0, 20.0);
        assert_eq!((t.sx, t.sy), (2.05, 2.0));
        // Taller viewport: fit the width, centre vertically.
        let t = svg_fit(20.0, 10.0, 20.0, 20.0);
        assert_eq!((t.sx, t.sy, t.tx, t.ty), (1.0, 1.0, 0.0, 5.0));
        // Wider viewport: fit the height, centre horizontally.
        let t = svg_fit(10.0, 10.0, 30.0, 10.0);
        assert_eq!((t.sx, t.sy, t.tx, t.ty), (1.0, 1.0, 10.0, 0.0));
    }

    #[test]
    fn reduction_levels() {
        // Drawn at half size or more: no reduction.
        assert_eq!(mip_level(100, 100, 100.0, 100.0), 0);
        assert_eq!(mip_level(100, 100, 51.0, 51.0), 0);
        assert_eq!(mip_level(100, 100, 400.0, 10.0), 0, "one axis enlarged");
        // Remaining scale above one half after halving.
        assert_eq!(mip_level(100, 100, 50.0, 50.0), 1);
        assert_eq!(mip_level(1000, 600, 100.0, 60.0), 3);
        assert_eq!(mip_level(1000, 600, 250.0, 150.0), 2);
        // Stops at one pixel on the smaller side, and on bad input.
        assert_eq!(mip_level(4, 1000, 0.001, 0.001), 2);
        assert_eq!(mip_level(1, 1, 0.01, 0.01), 0);
        assert_eq!(mip_level(0, 10, 1.0, 1.0), 0);
        assert_eq!(mip_level(100, 100, f32::NAN, 1.0), 0);
        assert_eq!(mip_level(100, 100, 0.0, 0.0), 0);
        assert!(mip_level(u32::MAX, u32::MAX, 1e-30, 1e-30) <= MAX_MIP_LEVEL);
    }

    #[test]
    fn halving_averages_premultiplied_pixels() {
        let mut src = Pixmap::new(3, 3).unwrap();
        for (i, px) in src.data_mut().chunks_exact_mut(4).enumerate() {
            let v = (i as u8) * 10;
            px.copy_from_slice(&[v, 0, 0, 255]);
        }
        let half = halve(&src).unwrap();
        assert_eq!((half.width(), half.height()), (2, 2));
        let at = |x: usize, y: usize| half.data()[(y * 2 + x) * 4];
        // (0,0): pixels 0, 1, 3, 4 → (0 + 10 + 30 + 40) / 4.
        assert_eq!(at(0, 0), 20);
        // Odd edge: the last column/row is averaged with itself.
        assert_eq!(at(1, 0), 35, "(20 + 20 + 50 + 50) / 4");
        assert_eq!(at(1, 1), 80, "pixel 8 alone");
        assert_eq!(half.data()[3], 255);
        let one = halve(&Pixmap::new(1, 1).unwrap()).unwrap();
        assert_eq!((one.width(), one.height()), (1, 1));
    }

    #[test]
    fn reductions_are_cached_per_image() {
        let source = Arc::new(Pixmap::new(64, 64).unwrap());
        let a = reduced(&source, 2).unwrap();
        assert_eq!((a.width(), a.height()), (16, 16));
        assert!(Arc::ptr_eq(&a, &reduced(&source, 2).unwrap()));
        let deeper = reduced(&source, 3).unwrap();
        assert_eq!(deeper.width(), 8);
        // Entries die with their image.
        let weak = Arc::downgrade(&source);
        drop(source);
        assert!(weak.upgrade().is_none());
        let other = Arc::new(Pixmap::new(8, 8).unwrap());
        assert_eq!(reduced(&other, 1).unwrap().width(), 4);
        let cache = MIP_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        assert!(cache.iter().all(|e| e.source.strong_count() > 0));
    }

    #[test]
    fn rebasing() {
        assert_eq!(rebase(5.0, 10.0, 7.0), 5.0);
        assert_eq!(rebase(5.0, 10.0, 100.0), 95.0);
        assert_eq!(rebase(-1000.0, 16.0, 0.0), -8.0);
        assert_eq!(rebase(50.0, 10.0, 0.0), 0.0);
    }

    #[test]
    fn filter_choice() {
        assert_eq!(
            quality(&Transform::from_translate(3.0, 4.0)),
            FilterQuality::Nearest
        );
        assert_eq!(
            quality(&Transform::from_translate(3.5, 4.0)),
            FilterQuality::Bilinear
        );
        assert_eq!(
            quality(&Transform::from_scale(2.0, 2.0)),
            FilterQuality::Bilinear
        );
    }
}
