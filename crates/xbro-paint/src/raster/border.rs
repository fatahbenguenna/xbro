//! Borders: solid rings, per-side trapezoids (diagonal joins between
//! differing colors), dashed/dotted strokes, double lines and the two-tone
//! 3D styles (groove, ridge, inset, outset).

use std::rc::Rc;

use tiny_skia::{FillRule, LineCap, Mask, Path, PathBuilder, Stroke, StrokeDash, Transform};
use xbro_layout::Rect;
use xbro_style::{BorderStyle, Color};

use super::clip::DevRect;
use super::shapes::{self, clamp_to_area, push_rounded_rect, rounded_rect_path, solid_paint};
use super::{ClipUse, Painter};
use crate::display_list::BorderSideItem;

/// Sides in display list order.
const TOP: usize = 0;
const RIGHT: usize = 1;
const BOTTOM: usize = 2;
const LEFT: usize = 3;

/// One side in device px.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Side {
    width: f32,
    style: BorderStyle,
    color: Color,
}

impl Side {
    fn visible(&self) -> bool {
        self.width > 0.0 && self.style.is_visible() && !self.color.is_transparent()
    }
}

/// Darker shade used by the 3D border styles.
fn dark(c: Color) -> Color {
    let f = |v: u8| (f32::from(v) * 0.6).round() as u8;
    Color::rgba(f(c.r), f(c.g), f(c.b), c.a)
}

/// Color of `side` for the band `outer` (outer half/whole) of a 3D style.
fn tone(style: BorderStyle, side: usize, color: Color, outer: bool) -> Color {
    let top_left = side == TOP || side == LEFT;
    let darken = match style {
        BorderStyle::Inset => top_left,
        BorderStyle::Outset => !top_left,
        BorderStyle::Groove => top_left == outer,
        BorderStyle::Ridge => top_left != outer,
        _ => false,
    };
    if darken {
        dark(color)
    } else {
        color
    }
}

/// `r` shrunk by `f` × each side's width.
fn inset_by(r: DevRect, widths: [f32; 4], f: f32) -> DevRect {
    DevRect::new(
        r.x0 + widths[LEFT] * f,
        r.y0 + widths[TOP] * f,
        (r.x1 - widths[RIGHT] * f).max(r.x0 + widths[LEFT] * f),
        (r.y1 - widths[BOTTOM] * f).max(r.y0 + widths[TOP] * f),
    )
}

/// Radii of a rect inset by `f` × widths (circular approximation).
fn inset_radii(radii: [f32; 4], widths: [f32; 4], f: f32) -> [f32; 4] {
    [
        (radii[0] - widths[TOP].max(widths[LEFT]) * f).max(0.0),
        (radii[1] - widths[TOP].max(widths[RIGHT]) * f).max(0.0),
        (radii[2] - widths[BOTTOM].max(widths[RIGHT]) * f).max(0.0),
        (radii[3] - widths[BOTTOM].max(widths[LEFT]) * f).max(0.0),
    ]
}

/// Quadrilateral of `side` between the outer rect `a` and the inner rect `b`.
fn side_quad(side: usize, a: DevRect, b: DevRect) -> Option<Path> {
    let pts = match side {
        TOP => [(a.x0, a.y0), (a.x1, a.y0), (b.x1, b.y0), (b.x0, b.y0)],
        RIGHT => [(a.x1, a.y0), (a.x1, a.y1), (b.x1, b.y1), (b.x1, b.y0)],
        BOTTOM => [(a.x1, a.y1), (a.x0, a.y1), (b.x0, b.y1), (b.x1, b.y1)],
        _ => [(a.x0, a.y1), (a.x0, a.y0), (b.x0, b.y0), (b.x0, b.y1)],
    };
    let mut pb = PathBuilder::new();
    pb.move_to(pts[0].0, pts[0].1);
    for &(x, y) in &pts[1..] {
        pb.line_to(x, y);
    }
    pb.close();
    pb.finish()
}

/// Ring between two (rounded) rects, filled even-odd.
fn ring_path(
    outer: DevRect,
    outer_radii: [f32; 4],
    inner: DevRect,
    inner_radii: [f32; 4],
) -> Option<Path> {
    let mut pb = PathBuilder::new();
    push_rounded_rect(&mut pb, outer, outer_radii);
    if !inner.is_empty() {
        push_rounded_rect(&mut pb, inner, inner_radii);
    }
    pb.finish()
}

/// Ring masks of a rounded border, keyed by band (bits of `f0`, `f1`).
type RingCache = Vec<((u32, u32), Option<Rc<Mask>>)>;

struct BorderGeometry {
    outer: DevRect,
    widths: [f32; 4],
    radii: [f32; 4],
    rounded: bool,
}

impl BorderGeometry {
    fn band(&self, f: f32) -> DevRect {
        inset_by(self.outer, self.widths, f)
    }

    fn band_radii(&self, f: f32) -> [f32; 4] {
        inset_radii(self.radii, self.widths, f)
    }
}

/// Draws a `Border` item.
pub(crate) fn draw_border(
    p: &mut Painter<'_>,
    rect: Rect,
    sides: &[BorderSideItem; 4],
    radii: [f32; 4],
) {
    let scale = p.scale();
    let dev = p.to_dev(rect);
    let device_radii = shapes::scale_radii(radii, scale, dev);
    let rounded = device_radii.iter().any(|&r| r > 0.0);
    // Square borders are snapped to whole pixels for crisp edges.
    let outer = if rounded { dev } else { dev.snap() };
    let sides: [Side; 4] = sides.map(|s| {
        let mut width = if s.width.is_finite() {
            (s.width * scale).max(0.0)
        } else {
            0.0
        };
        if !rounded && width > 0.0 {
            width = width.round().max(1.0);
        }
        Side {
            width,
            style: s.style,
            color: s.color,
        }
    });
    if outer.is_empty() || !sides.iter().any(Side::visible) {
        return;
    }
    let widths = sides.map(|s| if s.visible() { s.width } else { 0.0 });
    let max_r = device_radii.iter().fold(0.0f32, |a, &b| a.max(b));
    // Cull before any work.
    if matches!(p.clip_for(outer), ClipUse::Skip) {
        return;
    }
    let geo = BorderGeometry {
        outer: clamp_to_area(
            outer,
            p.area(),
            max_r + widths.iter().fold(0.0f32, |a, &b| a.max(b)),
        ),
        widths,
        radii: device_radii,
        rounded,
    };
    // Skip the interior when the border cannot be seen (large boxes).
    let inner = geo.band(1.0);
    let inner_hidden = DevRect::new(
        inner.x0 + max_r,
        inner.y0 + max_r,
        inner.x1 - max_r,
        inner.y1 - max_r,
    );
    if inner_hidden.contains(&p.clip().rect.intersect(&p.area())) && !inner_hidden.is_empty() {
        return;
    }

    let visible: Vec<usize> = (0..4).filter(|&i| sides[i].visible()).collect();
    let first = sides[visible[0]];
    let uniform = visible
        .iter()
        .all(|&i| sides[i].style == first.style && sides[i].color == first.color);

    if uniform && first.style == BorderStyle::Solid {
        // One even-odd ring: no seams between sides of the same color.
        fill_ring(p, &geo, 0.0, 1.0, first.color);
        return;
    }
    if uniform && first.style == BorderStyle::Double && widths.iter().all(|&w| w == 0.0 || w >= 3.0)
    {
        fill_ring(p, &geo, 0.0, 1.0 / 3.0, first.color);
        fill_ring(p, &geo, 2.0 / 3.0, 1.0, first.color);
        return;
    }
    let all_four_same_width = visible.len() == 4 && widths.iter().all(|&w| w == widths[0]);
    if uniform
        && rounded
        && all_four_same_width
        && matches!(first.style, BorderStyle::Dashed | BorderStyle::Dotted)
    {
        stroke_rounded_dashed(p, &geo, first);
        return;
    }

    // Per-side drawing; rounded borders are masked by their ring.
    let mut rings: RingCache = Vec::new();
    for &i in &visible {
        let side = sides[i];
        match side.style {
            BorderStyle::Dashed | BorderStyle::Dotted => stroke_side(p, &geo, i, side),
            BorderStyle::Double if side.width >= 3.0 => {
                fill_band(p, &geo, &mut rings, i, 0.0, 1.0 / 3.0, side.color);
                fill_band(p, &geo, &mut rings, i, 2.0 / 3.0, 1.0, side.color);
            }
            BorderStyle::Groove | BorderStyle::Ridge => {
                let outer = tone(side.style, i, side.color, true);
                let inner = tone(side.style, i, side.color, false);
                fill_band(p, &geo, &mut rings, i, 0.0, 0.5, outer);
                fill_band(p, &geo, &mut rings, i, 0.5, 1.0, inner);
            }
            BorderStyle::Inset | BorderStyle::Outset => {
                let color = tone(side.style, i, side.color, true);
                fill_band(p, &geo, &mut rings, i, 0.0, 1.0, color);
            }
            _ => fill_band(p, &geo, &mut rings, i, 0.0, 1.0, side.color),
        }
    }
}

/// Fills the ring between the bands `f0` and `f1` (fractions of the widths).
fn fill_ring(p: &mut Painter<'_>, geo: &BorderGeometry, f0: f32, f1: f32, color: Color) {
    let (a, b) = (geo.band(f0), geo.band(f1));
    if !geo.rounded {
        // Four rects: exact pixel coverage, no anti-aliased seams.
        let parts = [
            DevRect::new(a.x0, a.y0, a.x1, b.y0),
            DevRect::new(a.x0, b.y1, a.x1, a.y1),
            DevRect::new(a.x0, b.y0, b.x0, b.y1),
            DevRect::new(b.x1, b.y0, a.x1, b.y1),
        ];
        for part in parts {
            if !part.is_empty() {
                p.fill_dev_rect(part, color);
            }
        }
        return;
    }
    if let Some(path) = ring_path(a, geo.band_radii(f0), b, geo.band_radii(f1)) {
        shapes::fill_path(p, &path, a, &solid_paint(color), FillRule::EvenOdd);
    }
}

/// Fills one side's part of a band. Rounded borders draw the side's wedge
/// through the band's ring mask (cached in `rings` per band).
fn fill_band(
    p: &mut Painter<'_>,
    geo: &BorderGeometry,
    rings: &mut RingCache,
    side: usize,
    f0: f32,
    f1: f32,
    color: Color,
) {
    if color.is_transparent() {
        return;
    }
    let (a, b) = (geo.band(f0), geo.band(f1));
    if !geo.rounded {
        if let Some(path) = side_quad(side, a, b) {
            shapes::fill_path(p, &path, a, &solid_paint(color), FillRule::Winding);
        }
        return;
    }
    let key = (f0.to_bits(), f1.to_bits());
    let mask = match rings.iter().find(|(k, _)| *k == key) {
        Some((_, mask)) => mask.clone(),
        None => {
            let mask = ring_mask(p, geo, f0, f1);
            rings.push((key, mask.clone()));
            mask
        }
    };
    let Some(mask) = mask else { return };
    // The wedge spans from the outer corners to the inner rect's corners.
    let Some(path) = side_quad(side, geo.outer, geo.band(1.0)) else {
        return;
    };
    let dirty = a.intersect(&p.clip().rect);
    if let Some(mut target) = p.target() {
        target.fill_path(
            &path,
            &solid_paint(color),
            FillRule::Winding,
            Transform::identity(),
            Some(&mask),
        );
    }
    p.mark_dirty(dirty);
}

/// Mask of the ring between bands `f0` and `f1`, intersected with the clip.
fn ring_mask(p: &Painter<'_>, geo: &BorderGeometry, f0: f32, f1: f32) -> Option<Rc<Mask>> {
    let (w, h) = p.size();
    let (a, b) = (geo.band(f0), geo.band(f1));
    let path = ring_path(a, geo.band_radii(f0), b, geo.band_radii(f1))?;
    let mut mask = Mask::new(w, h)?;
    mask.fill_path(&path, FillRule::EvenOdd, true, Transform::identity());
    match p.clip_for(a) {
        ClipUse::Skip => return None,
        ClipUse::Free => {}
        ClipUse::Mask(clip) => {
            for (m, c) in mask.data_mut().iter_mut().zip(clip.data()) {
                *m = super::div255(u32::from(*m) * u32::from(*c)) as u8;
            }
        }
    }
    Some(Rc::new(mask))
}

/// Dash pattern and cap for a dashed/dotted line of `len` device px drawn
/// `width` thick, adjusted so the pattern fits exactly: dashes start and
/// end flush with the line ends (corners are covered), round dots are
/// spaced evenly between dots centred on both ends (the caller then
/// shortens the line by `width / 2` at each end: see [`dot_inset`]).
fn dash_pattern(
    style: BorderStyle,
    width: f32,
    len: f32,
    closed: bool,
) -> (Option<StrokeDash>, LineCap) {
    if !(len.is_finite() && len > 0.0 && width > 0.0) {
        return (None, LineCap::Butt);
    }
    if style == BorderStyle::Dotted && width >= 3.0 {
        let n = (len / (2.0 * width)).round().max(1.0);
        return (StrokeDash::new(vec![0.0, len / n], 0.0), LineCap::Round);
    }
    let unit = if style == BorderStyle::Dotted {
        width
    } else {
        width * 3.0
    };
    let d = if closed {
        let n = (len / (2.0 * unit)).round().max(1.0);
        len / (2.0 * n)
    } else {
        let n = ((len / unit + 1.0) / 2.0).round().max(1.0);
        len / (2.0 * n - 1.0)
    };
    (StrokeDash::new(vec![d, d], 0.0), LineCap::Butt)
}

/// How far round dots are inset from the line ends (half a dot).
fn dot_inset(style: BorderStyle, width: f32) -> f32 {
    if style == BorderStyle::Dotted && width >= 3.0 {
        width / 2.0
    } else {
        0.0
    }
}

/// Strokes a dashed/dotted side along its centre line.
fn stroke_side(p: &mut Painter<'_>, geo: &BorderGeometry, side: usize, s: Side) {
    let o = geo.outer;
    let w = s.width;
    let (x0, y0, x1, y1) = match side {
        TOP => (o.x0, o.y0 + w / 2.0, o.x1, o.y0 + w / 2.0),
        RIGHT => (o.x1 - w / 2.0, o.y0, o.x1 - w / 2.0, o.y1),
        BOTTOM => (o.x1, o.y1 - w / 2.0, o.x0, o.y1 - w / 2.0),
        _ => (o.x0 + w / 2.0, o.y1, o.x0 + w / 2.0, o.y0),
    };
    let len = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    if len.is_nan() || len <= 0.0 {
        return;
    }
    let inset = dot_inset(s.style, w).min(len / 2.0);
    let (ux, uy) = ((x1 - x0) / len, (y1 - y0) / len);
    let (sx, sy) = (x0 + ux * inset, y0 + uy * inset);
    let (ex, ey) = (x1 - ux * inset, y1 - uy * inset);
    let mut pb = PathBuilder::new();
    pb.move_to(sx, sy);
    pb.line_to(ex, ey);
    let Some(path) = pb.finish() else {
        // A single dot.
        let Some(dot) = PathBuilder::from_circle(sx, sy, w / 2.0) else {
            return;
        };
        shapes::fill_path(
            p,
            &dot,
            DevRect::new(sx, sy, sx, sy).outset(w),
            &solid_paint(s.color),
            FillRule::Winding,
        );
        return;
    };
    let (dash, line_cap) = dash_pattern(s.style, w, len - 2.0 * inset, false);
    let stroke = Stroke {
        width: w,
        line_cap,
        dash,
        ..Stroke::default()
    };
    let bounds = DevRect::new(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)).outset(w);
    shapes::stroke_path(p, &path, bounds, &solid_paint(s.color), &stroke);
}

/// Strokes a uniform rounded dashed/dotted border along its centre line.
fn stroke_rounded_dashed(p: &mut Painter<'_>, geo: &BorderGeometry, s: Side) {
    let mid = geo.band(0.5);
    let radii = geo.band_radii(0.5);
    let Some(path) = rounded_rect_path(mid, radii) else {
        return;
    };
    // Perimeter of the rounded rect: straight parts plus quarter circles.
    let corners: f32 = radii
        .iter()
        .map(|r| r * (std::f32::consts::FRAC_PI_2 - 2.0))
        .sum();
    let perimeter = 2.0 * (mid.width() + mid.height()) + corners;
    let (dash, line_cap) = dash_pattern(s.style, s.width, perimeter, true);
    let stroke = Stroke {
        width: s.width,
        line_cap,
        dash,
        ..Stroke::default()
    };
    shapes::stroke_path(
        p,
        &path,
        geo.outer.outset(s.width),
        &solid_paint(s.color),
        &stroke,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tones() {
        let c = Color::rgb(100, 200, 50);
        let d = dark(c);
        assert_eq!(d, Color::rgb(60, 120, 30));
        assert_eq!(tone(BorderStyle::Inset, TOP, c, true), d);
        assert_eq!(tone(BorderStyle::Inset, BOTTOM, c, true), c);
        assert_eq!(tone(BorderStyle::Outset, LEFT, c, true), c);
        assert_eq!(tone(BorderStyle::Outset, RIGHT, c, true), d);
        assert_eq!(tone(BorderStyle::Groove, TOP, c, true), d);
        assert_eq!(tone(BorderStyle::Groove, TOP, c, false), c);
        assert_eq!(tone(BorderStyle::Ridge, TOP, c, true), c);
        assert_eq!(tone(BorderStyle::Ridge, BOTTOM, c, true), d);
    }

    #[test]
    fn insets() {
        let r = DevRect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(
            inset_by(r, [1.0, 2.0, 3.0, 4.0], 1.0),
            DevRect::new(4.0, 1.0, 8.0, 7.0)
        );
        assert_eq!(
            inset_by(r, [8.0, 8.0, 8.0, 8.0], 1.0),
            DevRect::new(8.0, 8.0, 8.0, 8.0)
        );
        assert_eq!(
            inset_radii([5.0; 4], [1.0, 2.0, 3.0, 4.0], 1.0),
            [1.0, 3.0, 2.0, 1.0]
        );
    }

    #[test]
    fn dashes_fit_the_line() {
        // 100px dashed line, 2px wide: dashes of ~6px, odd count, flush ends.
        let (dash, cap) = dash_pattern(BorderStyle::Dashed, 2.0, 100.0, false);
        assert!(dash.is_some());
        assert_eq!(cap, LineCap::Butt);
        assert_eq!(
            dash_pattern(BorderStyle::Dotted, 4.0, 100.0, false).1,
            LineCap::Round
        );
        assert!(dash_pattern(BorderStyle::Dotted, 1.0, 100.0, true)
            .0
            .is_some());
        assert!(dash_pattern(BorderStyle::Dashed, 2.0, 0.0, false)
            .0
            .is_none());
        assert!(dash_pattern(BorderStyle::Dashed, 2.0, f32::NAN, false)
            .0
            .is_none());
        assert_eq!(dot_inset(BorderStyle::Dotted, 4.0), 2.0);
        assert_eq!(dot_inset(BorderStyle::Dashed, 4.0), 0.0);
    }
}
