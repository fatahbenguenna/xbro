//! Images: scaled replaced content, tiled backgrounds and SVG rendering
//! (with a bounded cache of rendered SVG pixmaps).

use std::sync::{Arc, Mutex, Weak};

use tiny_skia::{FilterQuality, Paint, Pixmap, SpreadMode, Transform};
use xbro_layout::{Point, Rect, Size};
use xbro_style::{BackgroundRepeat, BackgroundSize, LengthPercentage};

use super::clip::DevRect;
use super::Painter;
use crate::display_list::background_tile;
use crate::image::{with_svg_stack, Image};

/// Largest side of an SVG rendering, in device px (larger destinations
/// are rendered at this resolution and scaled up).
const MAX_SVG_SIDE: f32 = 4096.0;
/// Largest SVG rendering area, in device px.
const MAX_SVG_AREA: f32 = 4096.0 * 4096.0;
/// Maximum number of cached SVG renderings.
const SVG_CACHE_ENTRIES: usize = 64;
/// Maximum memory held by cached SVG renderings.
const SVG_CACHE_BYTES: usize = 64 << 20;

/// One cached rendering of an SVG tree at a pixel size.
struct SvgEntry {
    tree: Weak<resvg::usvg::Tree>,
    width: u32,
    height: u32,
    pixmap: Arc<Pixmap>,
}

/// Rendered SVGs, oldest first.
static SVG_CACHE: Mutex<Vec<SvgEntry>> = Mutex::new(Vec::new());

/// Renders `tree` at `width` × `height` px, using the cache.
fn render_svg(tree: &Arc<resvg::usvg::Tree>, width: u32, height: u32) -> Option<Arc<Pixmap>> {
    let mut cache = SVG_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|e| e.tree.strong_count() > 0);
    if let Some(pos) = cache.iter().position(|e| {
        e.width == width
            && e.height == height
            && e.tree.upgrade().is_some_and(|t| Arc::ptr_eq(&t, tree))
    }) {
        let entry = cache.remove(pos);
        let pixmap = entry.pixmap.clone();
        cache.push(entry);
        return Some(pixmap);
    }
    drop(cache);

    let pixmap = Arc::new(render_svg_uncached(tree, width, height)?);
    let mut cache = SVG_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.push(SvgEntry {
        tree: Arc::downgrade(tree),
        width,
        height,
        pixmap: pixmap.clone(),
    });
    let bytes = |c: &Vec<SvgEntry>| c.iter().map(|e| e.pixmap.data().len()).sum::<usize>();
    while cache.len() > SVG_CACHE_ENTRIES || (cache.len() > 1 && bytes(&cache) > SVG_CACHE_BYTES) {
        cache.remove(0);
    }
    Some(pixmap)
}

fn render_svg_uncached(tree: &resvg::usvg::Tree, width: u32, height: u32) -> Option<Pixmap> {
    let size = tree.size();
    let (sw, sh) = (size.width(), size.height());
    if !(sw > 0.0 && sh > 0.0) {
        return None;
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    let transform = svg_fit(sw, sh, width as f32, height as f32);
    // resvg recurses like usvg does: render on the large SVG stack (which
    // also contains panics).
    let target = &mut pixmap;
    with_svg_stack(move || resvg::render(tree, transform, &mut target.as_mut()))?;
    // resvg uses its own tiny-skia version; both store premultiplied RGBA8.
    let size = tiny_skia::IntSize::from_wh(width, height)?;
    Pixmap::from_vec(pixmap.take(), size)
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

/// Pixel size at which to render an SVG drawn into `w` × `h` device px.
fn svg_render_size(w: f32, h: f32) -> Option<(u32, u32)> {
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let mut f = 1.0f32;
    if w.max(h) > MAX_SVG_SIDE {
        f = MAX_SVG_SIDE / w.max(h);
    }
    if w * h * f * f > MAX_SVG_AREA {
        f = f.min((MAX_SVG_AREA / (w * h)).sqrt());
    }
    let rw = (w * f).round().clamp(1.0, MAX_SVG_SIDE) as u32;
    let rh = (h * f).round().clamp(1.0, MAX_SVG_SIDE) as u32;
    Some((rw, rh))
}

/// A pixmap of `image` suitable for drawing at `w` × `h` device px.
fn pixmap_for(image: &Image, w: f32, h: f32) -> Option<Arc<Pixmap>> {
    match image {
        Image::Raster(p) => Some(p.clone()),
        Image::Svg(tree) => {
            let (rw, rh) = svg_render_size(w, h)?;
            render_svg(tree, rw, rh)
        }
    }
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
    let Some(rect) = visible.to_skia() else {
        return;
    };
    let paint = Paint {
        shader: tiny_skia::Pattern::new(pixmap.as_ref(), spread, quality(&ts), 1.0, ts),
        anti_alias: true,
        ..Paint::default()
    };
    if let Some(mut target) = p.target() {
        target.fill_rect(rect, &paint, Transform::identity(), mask.as_deref());
    }
    p.mark_dirty(visible);
}

/// Draws `image` scaled into `rect` (CSS px).
pub(crate) fn draw_image(p: &mut Painter<'_>, rect: Rect, image: &Image) {
    let dest = p.to_dev(rect);
    if dest.is_empty() || p.rect_clip(dest).is_none() {
        return;
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
        assert_eq!(svg_render_size(10.4, 20.6), Some((10, 21)));
        assert_eq!(svg_render_size(0.0, 20.0), None);
        assert_eq!(svg_render_size(f32::NAN, 20.0), None);
        let (w, h) = svg_render_size(100_000.0, 50_000.0).unwrap();
        assert!(w <= 4096 && h <= 4096 && (w as f32 * h as f32) <= MAX_SVG_AREA);
        assert_eq!(svg_render_size(0.2, 0.2), Some((1, 1)));
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
