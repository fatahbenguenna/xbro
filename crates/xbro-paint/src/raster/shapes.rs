//! Paths, paints and simple shapes (rounded rects, ellipses, polylines,
//! polygons, linear gradients).

use tiny_skia::{
    FillRule, GradientStop, LineCap, LineJoin, Paint, Path, PathBuilder, Shader, SpreadMode,
    Stroke, Transform,
};
use xbro_layout::{Point, Rect};
use xbro_style::Color;

use super::clip::DevRect;
use super::{ClipUse, Painter};

/// Bézier control distance for quarter circles.
const KAPPA: f32 = 0.552_284_8;

pub(crate) fn skia_color(c: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// An anti-aliased solid paint.
pub(crate) fn solid_paint(c: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(c.r, c.g, c.b, c.a);
    paint.anti_alias = true;
    paint
}

/// `target.fill_rect(rect, paint)` with anti-aliasing (identity transform).
///
/// tiny-skia 0.11 (`scan::hairline_aa::fill_dot8`) debug-asserts on a rect
/// whose fractional left and right edges fall in adjacent pixel columns
/// (no whole column between them, e.g. 11.1 → 12.5); release builds draw
/// it correctly, but in debug builds the assertion aborted the whole
/// rendering. Such a rect is drawn as its two single-column halves.
pub(crate) fn fill_rect_aa(
    target: &mut tiny_skia::PixmapMut<'_>,
    rect: DevRect,
    paint: &Paint<'_>,
    mask: Option<&tiny_skia::Mask>,
) {
    let split = rect.x0.ceil();
    let parts = if rect.x0 < split && split < rect.x1 && rect.x1 - split < 1.0 {
        [
            DevRect::new(rect.x0, rect.y0, split, rect.y1),
            DevRect::new(split, rect.y0, rect.x1, rect.y1),
        ]
    } else {
        [rect, DevRect::EMPTY]
    };
    for part in parts {
        if let Some(r) = part.to_skia() {
            target.fill_rect(r, paint, Transform::identity(), mask);
        }
    }
}

/// Scales CSS radii to device px and clamps them so adjacent radii fit.
pub(crate) fn scale_radii(radii: [f32; 4], scale: f32, rect: DevRect) -> [f32; 4] {
    let mut r = radii.map(|v| {
        if v.is_finite() && v > 0.0 {
            v * scale
        } else {
            0.0
        }
    });
    if r.iter().all(|&v| v == 0.0) {
        return r;
    }
    let (w, h) = (rect.width(), rect.height());
    let mut f = 1.0f32;
    for (side, sum) in [
        (w, r[0] + r[1]),
        (h, r[1] + r[2]),
        (w, r[2] + r[3]),
        (h, r[3] + r[0]),
    ] {
        if sum > side && sum > 0.0 {
            f = f.min(side / sum);
        }
    }
    if f < 1.0 {
        for v in &mut r {
            *v *= f;
        }
    }
    r
}

/// Moves edges lying far outside `area` closer to it without changing what
/// is visible (corners of radius ≤ `max_radius` stay outside). Keeps path
/// coordinates small for tiny-skia.
pub(crate) fn clamp_to_area(r: DevRect, area: DevRect, max_radius: f32) -> DevRect {
    let lim = area.outset(4.0 + max_radius.max(0.0));
    DevRect::new(
        r.x0.max(lim.x0),
        r.y0.max(lim.y0),
        r.x1.min(lim.x1),
        r.y1.min(lim.y1),
    )
}

/// Path of a rectangle with corner radii (tl, tr, br, bl).
pub(crate) fn rounded_rect_path(r: DevRect, radii: [f32; 4]) -> Option<Path> {
    if r.is_empty() {
        return None;
    }
    if radii.iter().all(|&v| v <= 0.0) {
        return Some(PathBuilder::from_rect(r.to_skia()?));
    }
    let [tl, tr, br, bl] = radii.map(|v| v.max(0.0));
    let mut pb = PathBuilder::new();
    pb.move_to(r.x0 + tl, r.y0);
    pb.line_to(r.x1 - tr, r.y0);
    if tr > 0.0 {
        pb.cubic_to(
            r.x1 - tr + tr * KAPPA,
            r.y0,
            r.x1,
            r.y0 + tr - tr * KAPPA,
            r.x1,
            r.y0 + tr,
        );
    }
    pb.line_to(r.x1, r.y1 - br);
    if br > 0.0 {
        pb.cubic_to(
            r.x1,
            r.y1 - br + br * KAPPA,
            r.x1 - br + br * KAPPA,
            r.y1,
            r.x1 - br,
            r.y1,
        );
    }
    pb.line_to(r.x0 + bl, r.y1);
    if bl > 0.0 {
        pb.cubic_to(
            r.x0 + bl - bl * KAPPA,
            r.y1,
            r.x0,
            r.y1 - bl + bl * KAPPA,
            r.x0,
            r.y1 - bl,
        );
    }
    pb.line_to(r.x0, r.y0 + tl);
    if tl > 0.0 {
        pb.cubic_to(
            r.x0,
            r.y0 + tl - tl * KAPPA,
            r.x0 + tl - tl * KAPPA,
            r.y0,
            r.x0 + tl,
            r.y0,
        );
    }
    pb.close();
    pb.finish()
}

/// Appends a rounded rect to `pb` (used for even-odd rings).
pub(crate) fn push_rounded_rect(pb: &mut PathBuilder, r: DevRect, radii: [f32; 4]) {
    if let Some(path) = rounded_rect_path(r, radii) {
        pb.push_path(&path);
    }
}

/// Fills `path` (device bounds `bounds`) through the current clip.
pub(crate) fn fill_path(
    p: &mut Painter<'_>,
    path: &Path,
    bounds: DevRect,
    paint: &Paint<'_>,
    rule: FillRule,
) {
    let mask = match p.clip_for(bounds) {
        ClipUse::Skip => return,
        ClipUse::Free => None,
        ClipUse::Mask(m) => Some(m),
    };
    let dirty = bounds.intersect(&p.clip().rect);
    if let Some(mut target) = p.target() {
        target.fill_path(path, paint, rule, Transform::identity(), mask.as_deref());
    }
    p.mark_dirty(dirty);
}

/// Strokes `path` (device bounds `bounds`, including the stroke) through the current clip.
pub(crate) fn stroke_path(
    p: &mut Painter<'_>,
    path: &Path,
    bounds: DevRect,
    paint: &Paint<'_>,
    stroke: &Stroke,
) {
    let mask = match p.clip_for(bounds) {
        ClipUse::Skip => return,
        ClipUse::Free => None,
        ClipUse::Mask(m) => Some(m),
    };
    let dirty = bounds.intersect(&p.clip().rect);
    if let Some(mut target) = p.target() {
        target.stroke_path(path, paint, stroke, Transform::identity(), mask.as_deref());
    }
    p.mark_dirty(dirty);
}

/// Fills a rounded rect with a solid color.
pub(crate) fn fill_rounded(p: &mut Painter<'_>, dev: DevRect, radii: [f32; 4], color: Color) {
    let max_r = radii.iter().fold(0.0f32, |a, &b| a.max(b));
    let shape = clamp_to_area(dev, p.area(), max_r);
    if shape.is_empty() {
        return;
    }
    if let Some(path) = rounded_rect_path(shape, radii) {
        fill_path(p, &path, shape, &solid_paint(color), FillRule::Winding);
    }
}

/// Gradient stops for tiny-skia: positions sanitised and made monotonic;
/// fully transparent stops take their neighbours' RGB so interpolation
/// approximates CSS's premultiplied interpolation (no grey fringes).
fn gradient_stops(stops: &[(Color, f32)]) -> Vec<GradientStop> {
    let mut out = Vec::with_capacity(stops.len() + 2);
    let mut prev = 0.0f32;
    for (i, &(color, pos)) in stops.iter().enumerate() {
        let pos = if pos.is_nan() {
            prev
        } else {
            pos.clamp(0.0, 1.0).max(prev)
        };
        prev = pos;
        if color.a == 0 {
            let before = i.checked_sub(1).and_then(|j| stops.get(j)).map(|s| s.0);
            let after = stops.get(i + 1).map(|s| s.0);
            let left = before.or(after).unwrap_or(color);
            let right = after.unwrap_or(left);
            out.push(GradientStop::new(
                pos,
                skia_color(Color::rgba(left.r, left.g, left.b, 0)),
            ));
            if (right.r, right.g, right.b) != (left.r, left.g, left.b) {
                out.push(GradientStop::new(
                    pos,
                    skia_color(Color::rgba(right.r, right.g, right.b, 0)),
                ));
            }
        } else {
            out.push(GradientStop::new(pos, skia_color(color)));
        }
    }
    out
}

/// CSS `linear-gradient` over `rect`: the gradient line passes through
/// the centre at `angle_deg` (0 = to top) with length `|w sin θ| + |h cos θ|`.
pub(crate) fn draw_linear_gradient(
    p: &mut Painter<'_>,
    rect: Rect,
    angle_deg: f32,
    stops: &[(Color, f32)],
    radii: [f32; 4],
) {
    let dev = p.to_dev(rect);
    if dev.is_empty() || stops.is_empty() || stops.iter().all(|s| s.0.is_transparent()) {
        return;
    }
    let radii = scale_radii(radii, p.scale(), dev);
    let angle = if angle_deg.is_finite() {
        angle_deg
    } else {
        180.0
    };
    let theta = angle.to_radians();
    let (dx, dy) = (theta.sin(), -theta.cos());
    let len = (dev.width() * dx).abs() + (dev.height() * dy).abs();
    let (cx, cy) = ((dev.x0 + dev.x1) / 2.0, (dev.y0 + dev.y1) / 2.0);
    let shader = if stops.len() == 1 || len < 1e-3 {
        let last = stops.last().map_or(Color::TRANSPARENT, |s| s.0);
        Shader::SolidColor(skia_color(last))
    } else {
        let start = tiny_skia::Point::from_xy(cx - dx * len / 2.0, cy - dy * len / 2.0);
        let end = tiny_skia::Point::from_xy(cx + dx * len / 2.0, cy + dy * len / 2.0);
        match tiny_skia::LinearGradient::new(
            start,
            end,
            gradient_stops(stops),
            SpreadMode::Pad,
            Transform::identity(),
        ) {
            Some(shader) => shader,
            None => return,
        }
    };
    let paint = Paint {
        shader,
        anti_alias: true,
        ..Paint::default()
    };
    let max_r = radii.iter().fold(0.0f32, |a, &b| a.max(b));
    if max_r <= 0.0 {
        let Some((visible, mask)) = p.rect_clip(dev) else {
            return;
        };
        if let Some(mut target) = p.target() {
            fill_rect_aa(&mut target, visible, &paint, mask.as_deref());
        }
        p.mark_dirty(visible);
    } else {
        let shape = clamp_to_area(dev, p.area(), max_r);
        if let Some(path) = rounded_rect_path(shape, radii) {
            fill_path(p, &path, shape, &paint, FillRule::Winding);
        }
    }
}

pub(crate) fn draw_ellipse(
    p: &mut Painter<'_>,
    rect: Rect,
    color: Color,
    fill: bool,
    stroke_width: f32,
) {
    if color.is_transparent() {
        return;
    }
    let dev = p.to_dev(rect);
    let Some(oval) = dev.to_skia() else { return };
    let Some(path) = PathBuilder::from_oval(oval) else {
        return;
    };
    let paint = solid_paint(color);
    if fill {
        fill_path(p, &path, dev, &paint, FillRule::Winding);
    } else {
        let width = stroke_width * p.scale();
        if !(width.is_finite() && width > 0.0) {
            return;
        }
        let stroke = Stroke {
            width,
            ..Stroke::default()
        };
        stroke_path(p, &path, dev.outset(width / 2.0 + 1.0), &paint, &stroke);
    }
}

/// Device path through `points`; returns it with its bounds.
fn device_path(p: &Painter<'_>, points: &[Point], close: bool) -> Option<(Path, DevRect)> {
    let mut pb = PathBuilder::new();
    let mut bounds = DevRect::EMPTY;
    for (i, &pt) in points.iter().enumerate() {
        let (x, y) = p.to_dev_point(pt);
        let (x, y) = (x.clamp(-1.0e7, 1.0e7), y.clamp(-1.0e7, 1.0e7));
        if i == 0 {
            pb.move_to(x, y);
            bounds = DevRect::new(x, y, x, y);
        } else {
            pb.line_to(x, y);
            bounds = DevRect::new(
                bounds.x0.min(x),
                bounds.y0.min(y),
                bounds.x1.max(x),
                bounds.y1.max(y),
            );
        }
    }
    if close {
        pb.close();
    }
    Some((pb.finish()?, bounds))
}

pub(crate) fn draw_polyline(p: &mut Painter<'_>, points: &[Point], width: f32, color: Color) {
    let width = width * p.scale();
    if color.is_transparent() || points.len() < 2 || !(width.is_finite() && width > 0.0) {
        return;
    }
    let Some((path, bounds)) = device_path(p, points, false) else {
        return;
    };
    let stroke = Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    stroke_path(
        p,
        &path,
        bounds.outset(width / 2.0 + 1.0),
        &solid_paint(color),
        &stroke,
    );
}

pub(crate) fn draw_polygon(p: &mut Painter<'_>, points: &[Point], color: Color) {
    if color.is_transparent() || points.len() < 3 {
        return;
    }
    let Some((path, bounds)) = device_path(p, points, true) else {
        return;
    };
    fill_path(
        p,
        &path,
        bounds.outset(1.0),
        &solid_paint(color),
        FillRule::Winding,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radii_scaling() {
        let r = DevRect::new(0.0, 0.0, 20.0, 10.0);
        assert_eq!(
            scale_radii([2.0, 0.0, f32::NAN, -1.0], 2.0, r),
            [4.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(scale_radii([10.0; 4], 1.0, r), [5.0; 4]);
    }

    #[test]
    fn clamping_keeps_visible_part() {
        let area = DevRect::new(0.0, 0.0, 100.0, 100.0);
        let r = clamp_to_area(DevRect::new(-1e6, 10.0, 50.0, 1e6), area, 5.0);
        assert_eq!(r, DevRect::new(-9.0, 10.0, 50.0, 109.0));
    }

    #[test]
    fn transparent_stops_take_neighbour_rgb() {
        let stops = gradient_stops(&[
            (Color::rgb(255, 0, 0), 0.0),
            (Color::TRANSPARENT, 0.5),
            (Color::rgb(0, 0, 255), 1.0),
        ]);
        assert_eq!(stops.len(), 4);
        let stops = gradient_stops(&[
            (Color::rgb(255, 0, 0), 0.5),
            (Color::rgb(0, 255, 0), f32::NAN),
            (Color::BLACK, 0.2),
        ]);
        assert_eq!(stops.len(), 3);
    }

    #[test]
    fn rounded_paths() {
        assert!(rounded_rect_path(DevRect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4]).is_some());
        assert!(rounded_rect_path(DevRect::new(0.0, 0.0, 10.0, 10.0), [5.0; 4]).is_some());
        assert!(rounded_rect_path(DevRect::new(0.0, 0.0, 0.0, 10.0), [5.0; 4]).is_none());
    }
}
