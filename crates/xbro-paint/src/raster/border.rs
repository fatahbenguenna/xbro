//! Borders: solid rings, per-side trapezoids (diagonal joins between
//! differing colors), dashed/dotted strokes, double lines and the two-tone
//! 3D styles (groove, ridge, inset, outset).

use std::rc::Rc;

use tiny_skia::{FillRule, LineCap, Mask, Path, PathBuilder, Stroke, StrokeDash, Transform};
use xbro_layout::Rect;
use xbro_style::{BorderStyle, Color};

use super::clip::DevRect;
use super::shapes::{self, clamp_to_area, push_rounded_rect, solid_paint};
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

/// Ring masks of a rounded border, keyed by band (bits of `f0`, `f1`),
/// with the region each was written in (to recycle them).
type RingCache = Vec<((u32, u32), Option<(Rc<Mask>, DevRect)>)>;

struct BorderGeometry {
    /// Outer edge, clamped near the drawing area (see [`clamp_to_area`]).
    outer: DevRect,
    /// Unclamped outer edge: dash patterns are laid out along it so they
    /// stay attached to the box when it is scrolled or clamped.
    full: DevRect,
    /// The region outside which nothing of the border can be seen.
    limit: DevRect,
    widths: [f32; 4],
    radii: [f32; 4],
    rounded: bool,
}

impl BorderGeometry {
    fn band(&self, f: f32) -> DevRect {
        inset_by(self.outer, self.widths, f)
    }

    /// Like [`BorderGeometry::band`], on the unclamped geometry.
    fn full_band(&self, f: f32) -> DevRect {
        inset_by(self.full, self.widths, f)
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
    let margin = max_r + widths.iter().fold(0.0f32, |a, &b| a.max(b));
    let geo = BorderGeometry {
        outer: clamp_to_area(outer, p.area(), margin),
        full: outer,
        limit: p.area().outset(4.0 + margin),
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
    let Some(first) = visible.first().map(|&i| sides[i]) else {
        return;
    };
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
            BorderStyle::Dashed | BorderStyle::Dotted => stroke_side(p, &geo, &mut rings, i, side),
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
    for (_, ring) in rings {
        if let Some((mask, region)) = ring {
            if let Ok(mask) = Rc::try_unwrap(mask) {
                p.masks().give(mask, region.outset(1.0));
            }
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
    let Some(mask) = cached_ring_mask(p, geo, rings, f0, f1) else {
        return;
    };
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

/// The ring mask of bands `f0`..`f1` (with the clip), from `rings` or
/// built and cached there.
fn cached_ring_mask(
    p: &Painter<'_>,
    geo: &BorderGeometry,
    rings: &mut RingCache,
    f0: f32,
    f1: f32,
) -> Option<Rc<Mask>> {
    let key = (f0.to_bits(), f1.to_bits());
    match rings.iter().find(|(k, _)| *k == key) {
        Some((_, mask)) => mask.as_ref().map(|(m, _)| m.clone()),
        None => {
            let mask = ring_mask(p, geo, f0, f1).map(Rc::new);
            rings.push((key, mask.clone().map(|m| (m, geo.band(f0)))));
            mask
        }
    }
}

/// Mask of the ring between bands `f0` and `f1`, intersected with the
/// clip. Non-zero only inside the band's outer rect.
fn ring_mask(p: &Painter<'_>, geo: &BorderGeometry, f0: f32, f1: f32) -> Option<Mask> {
    let (w, h) = p.size();
    let (a, b) = (geo.band(f0), geo.band(f1));
    let path = ring_path(a, geo.band_radii(f0), b, geo.band_radii(f1))?;
    let mut mask = p.masks().take(w, h)?;
    mask.fill_path(&path, FillRule::EvenOdd, true, Transform::identity());
    match p.clip_for(a) {
        ClipUse::Skip => {
            p.masks().give(mask, a.outset(1.0));
            return None;
        }
        ClipUse::Free => {}
        ClipUse::Mask(clip) => {
            // The ring lies inside `a`: only that region needs the product.
            let r = a.round_out().intersect(&p.area());
            let (x0, x1) = (r.x0 as usize, r.x1 as usize);
            let stride = w as usize;
            for y in r.y0 as usize..r.y1 as usize {
                let range = y * stride + x0..y * stride + x1;
                let (Some(row), Some(clip_row)) = (
                    mask.data_mut().get_mut(range.clone()),
                    clip.data().get(range),
                ) else {
                    continue;
                };
                for (m, c) in row.iter_mut().zip(clip_row) {
                    *m = super::div255(u32::from(*m) * u32::from(*c)) as u8;
                }
            }
        }
    }
    Some(mask)
}

/// Dash pattern and cap for a dashed/dotted line of `len` device px drawn
/// `width` thick, adjusted so the pattern fits exactly: dashes start and
/// end flush with the line ends (corners are covered), round dots are
/// spaced evenly between dots centred on both ends (the caller then
/// shortens the line by `width / 2` at each end: see [`dot_inset`]).
/// `offset` is how far into the pattern the stroked path starts (for
/// lines drawn in pieces or clipped at their start).
fn dash_pattern(
    style: BorderStyle,
    width: f32,
    len: f32,
    closed: bool,
    offset: f32,
) -> (Option<StrokeDash>, LineCap) {
    if !(len.is_finite() && len > 0.0 && width > 0.0) {
        return (None, LineCap::Butt);
    }
    let phase = |period: f32| {
        let r = offset.rem_euclid(period);
        if r.is_finite() {
            r
        } else {
            0.0
        }
    };
    if style == BorderStyle::Dotted && width >= 3.0 {
        let n = (len / (2.0 * width)).round().max(1.0);
        let gap = len / n;
        return (StrokeDash::new(vec![0.0, gap], phase(gap)), LineCap::Round);
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
    (StrokeDash::new(vec![d, d], phase(2.0 * d)), LineCap::Butt)
}

/// How far round dots are inset from the line ends (half a dot).
fn dot_inset(style: BorderStyle, width: f32) -> f32 {
    if style == BorderStyle::Dotted && width >= 3.0 {
        width / 2.0
    } else {
        0.0
    }
}

/// The part of the segment from `start` along the unit vector `dir`, of
/// length `len`, that lies within `lim`, as distances from `start`
/// (Liang–Barsky clipping). `None` when the segment misses `lim`.
fn visible_span(start: (f32, f32), dir: (f32, f32), len: f32, lim: DevRect) -> Option<(f32, f32)> {
    let (mut a, mut b) = (0.0f32, len);
    for (s, d, lo, hi) in [
        (start.0, dir.0, lim.x0, lim.x1),
        (start.1, dir.1, lim.y0, lim.y1),
    ] {
        if d.abs() < 1e-6 {
            if s < lo || s > hi {
                return None;
            }
        } else {
            let (t0, t1) = ((lo - s) / d, (hi - s) / d);
            a = a.max(t0.min(t1));
            b = b.min(t0.max(t1));
        }
    }
    (a.is_finite() && b.is_finite() && b >= a).then_some((a, b))
}

/// Strokes `path` with `stroke`, through `mask` when given (a ring mask,
/// which already includes the clip), else through the current clip.
fn stroke_masked(
    p: &mut Painter<'_>,
    path: &Path,
    bounds: DevRect,
    color: Color,
    stroke: &Stroke,
    mask: Option<&Mask>,
) {
    let Some(mask) = mask else {
        shapes::stroke_path(p, path, bounds, &solid_paint(color), stroke);
        return;
    };
    let dirty = bounds.intersect(&p.clip().rect);
    if let Some(mut target) = p.target() {
        target.stroke_path(
            path,
            &solid_paint(color),
            stroke,
            Transform::identity(),
            Some(mask),
        );
    }
    p.mark_dirty(dirty);
}

/// Strokes a dashed/dotted side along its centre line. The pattern is laid
/// out on the whole (unclamped) side and only its visible part is stroked;
/// rounded borders mask the side with their ring.
fn stroke_side(
    p: &mut Painter<'_>,
    geo: &BorderGeometry,
    rings: &mut RingCache,
    side: usize,
    s: Side,
) {
    let o = geo.full;
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
    let mask = if geo.rounded {
        match cached_ring_mask(p, geo, rings, 0.0, 1.0) {
            Some(mask) => Some(mask),
            None => return,
        }
    } else {
        None
    };
    let inset = dot_inset(s.style, w).min(len / 2.0);
    let (ux, uy) = ((x1 - x0) / len, (y1 - y0) / len);
    let (sx, sy) = (x0 + ux * inset, y0 + uy * inset);
    let seg = len - 2.0 * inset;
    let Some((a, b)) = visible_span((sx, sy), (ux, uy), seg, geo.limit) else {
        return;
    };
    let (ax, ay) = (sx + ux * a, sy + uy * a);
    let (bx, by) = (sx + ux * b, sy + uy * b);
    let mut pb = PathBuilder::new();
    pb.move_to(ax, ay);
    pb.line_to(bx, by);
    let Some(path) = pb.finish() else {
        // A single dot.
        let Some(dot) = PathBuilder::from_circle(ax, ay, w / 2.0) else {
            return;
        };
        let bounds = DevRect::new(ax, ay, ax, ay).outset(w);
        match mask.as_deref() {
            Some(mask) => {
                let dirty = bounds.intersect(&p.clip().rect);
                if let Some(mut target) = p.target() {
                    target.fill_path(
                        &dot,
                        &solid_paint(s.color),
                        FillRule::Winding,
                        Transform::identity(),
                        Some(mask),
                    );
                }
                p.mark_dirty(dirty);
            }
            None => shapes::fill_path(p, &dot, bounds, &solid_paint(s.color), FillRule::Winding),
        }
        return;
    };
    let (dash, line_cap) = dash_pattern(s.style, w, seg, false, a);
    let stroke = Stroke {
        width: w,
        line_cap,
        dash,
        ..Stroke::default()
    };
    let bounds = DevRect::new(ax.min(bx), ay.min(by), ax.max(bx), ay.max(by)).outset(w);
    stroke_masked(p, &path, bounds, s.color, &stroke, mask.as_deref());
}

/// One piece of the centre line of a rounded border: a straight segment
/// (`start` → `end`) or a quarter circle given as a cubic Bézier.
enum Piece {
    Line((f32, f32), (f32, f32)),
    Arc([(f32, f32); 4], f32),
}

/// Strokes a uniform rounded dashed/dotted border along its centre line,
/// piece by piece (four sides, four corners). The dash pattern is laid
/// out on the whole, unclamped perimeter, and each visible piece starts at
/// its own offset into it, so the dashes run continuously around the box
/// and stay attached to it whatever part of it is visible.
fn stroke_rounded_dashed(p: &mut Painter<'_>, geo: &BorderGeometry, s: Side) {
    let m = geo.full_band(0.5);
    let [tl, tr, br, bl] = geo.band_radii(0.5);
    const K: f32 = 0.552_284_8;
    let pieces = [
        Piece::Line((m.x0 + tl, m.y0), (m.x1 - tr, m.y0)),
        Piece::Arc(
            [
                (m.x1 - tr, m.y0),
                (m.x1 - tr + tr * K, m.y0),
                (m.x1, m.y0 + tr - tr * K),
                (m.x1, m.y0 + tr),
            ],
            tr,
        ),
        Piece::Line((m.x1, m.y0 + tr), (m.x1, m.y1 - br)),
        Piece::Arc(
            [
                (m.x1, m.y1 - br),
                (m.x1, m.y1 - br + br * K),
                (m.x1 - br + br * K, m.y1),
                (m.x1 - br, m.y1),
            ],
            br,
        ),
        Piece::Line((m.x1 - br, m.y1), (m.x0 + bl, m.y1)),
        Piece::Arc(
            [
                (m.x0 + bl, m.y1),
                (m.x0 + bl - bl * K, m.y1),
                (m.x0, m.y1 - bl + bl * K),
                (m.x0, m.y1 - bl),
            ],
            bl,
        ),
        Piece::Line((m.x0, m.y1 - bl), (m.x0, m.y0 + tl)),
        Piece::Arc(
            [
                (m.x0, m.y0 + tl),
                (m.x0, m.y0 + tl - tl * K),
                (m.x0 + tl - tl * K, m.y0),
                (m.x0 + tl, m.y0),
            ],
            tl,
        ),
    ];
    let length = |piece: &Piece| match piece {
        Piece::Line(a, b) => ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt(),
        Piece::Arc(_, r) => r * std::f32::consts::FRAC_PI_2,
    };
    let perimeter: f32 = pieces.iter().map(length).sum();
    if !(perimeter.is_finite() && perimeter > 0.0) {
        return;
    }
    let w = s.width;
    let mut travelled = 0.0f32;
    for piece in &pieces {
        let len = length(piece);
        let start = travelled;
        travelled += len;
        if len.is_nan() || len <= 0.0 {
            continue;
        }
        let mut pb = PathBuilder::new();
        let (offset, bounds) = match piece {
            Piece::Line(a, b) => {
                let dir = ((b.0 - a.0) / len, (b.1 - a.1) / len);
                let Some((t0, t1)) = visible_span(*a, dir, len, geo.limit) else {
                    continue;
                };
                let p0 = (a.0 + dir.0 * t0, a.1 + dir.1 * t0);
                let p1 = (a.0 + dir.0 * t1, a.1 + dir.1 * t1);
                pb.move_to(p0.0, p0.1);
                pb.line_to(p1.0, p1.1);
                let bounds = DevRect::new(
                    p0.0.min(p1.0),
                    p0.1.min(p1.1),
                    p0.0.max(p1.0),
                    p0.1.max(p1.1),
                );
                (start + t0, bounds)
            }
            Piece::Arc(c, _) => {
                let xs = c.map(|q| q.0);
                let ys = c.map(|q| q.1);
                let bounds = DevRect::new(
                    xs.iter().fold(f32::INFINITY, |a, &b| a.min(b)),
                    ys.iter().fold(f32::INFINITY, |a, &b| a.min(b)),
                    xs.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b)),
                    ys.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b)),
                );
                if !bounds.outset(w).intersects(&geo.limit) {
                    continue;
                }
                pb.move_to(c[0].0, c[0].1);
                pb.cubic_to(c[1].0, c[1].1, c[2].0, c[2].1, c[3].0, c[3].1);
                (start, bounds)
            }
        };
        let Some(path) = pb.finish() else { continue };
        let (dash, line_cap) = dash_pattern(s.style, w, perimeter, true, offset);
        let stroke = Stroke {
            width: w,
            line_cap,
            dash,
            ..Stroke::default()
        };
        shapes::stroke_path(p, &path, bounds.outset(w), &solid_paint(s.color), &stroke);
    }
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
        let (dash, cap) = dash_pattern(BorderStyle::Dashed, 2.0, 100.0, false, 0.0);
        assert!(dash.is_some());
        assert_eq!(cap, LineCap::Butt);
        assert_eq!(
            dash_pattern(BorderStyle::Dotted, 4.0, 100.0, false, 0.0).1,
            LineCap::Round
        );
        assert!(dash_pattern(BorderStyle::Dotted, 1.0, 100.0, true, 0.0)
            .0
            .is_some());
        assert!(dash_pattern(BorderStyle::Dashed, 2.0, 0.0, false, 0.0)
            .0
            .is_none());
        assert!(dash_pattern(BorderStyle::Dashed, 2.0, f32::NAN, false, 0.0)
            .0
            .is_none());
        assert_eq!(dot_inset(BorderStyle::Dotted, 4.0), 2.0);
        assert_eq!(dot_inset(BorderStyle::Dashed, 4.0), 0.0);
    }
}
