//! Geometry helpers for backgrounds and rounded corners.

use xbro_layout::{Edges, Point, Rect, Size};
use xbro_style::{BackgroundSize, LengthPercentage};

/// Largest coordinate/size accepted for geometry (CSS px); larger values are
/// clamped so later arithmetic stays finite.
pub(crate) const MAX_COORD: f32 = 1.0e7;

/// Smallest tile side (CSS px) that is still painted; smaller tiles would
/// need millions of repetitions.
const MIN_TILE: f32 = 0.01;

/// Replaces NaN by 0 and clamps to ±[`MAX_COORD`].
pub(crate) fn sane(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(-MAX_COORD, MAX_COORD)
    }
}

/// Like [`sane`], and never negative.
pub(crate) fn sane_len(v: f32) -> f32 {
    sane(v).max(0.0)
}

/// A rect with finite coordinates and non-negative size.
pub(crate) fn sane_rect(r: Rect) -> Rect {
    Rect::new(sane(r.x), sane(r.y), sane_len(r.width), sane_len(r.height))
}

/// Resolves `border-radius` values for a border box: percentages against
/// the border-box width, negative/NaN values to 0, then scaled down
/// uniformly so adjacent radii fit on every side (CSS Backgrounds §5.5).
/// Order: top-left, top-right, bottom-right, bottom-left.
pub fn resolve_radii(radii: &[LengthPercentage; 4], border_box: Rect) -> [f32; 4] {
    let mut out = [0.0f32; 4];
    for (o, r) in out.iter_mut().zip(radii) {
        *o = sane_len(r.resolve(border_box.width));
    }
    clamp_radii(out, border_box)
}

/// Scales radii down so the sum of adjacent radii never exceeds a side.
pub(crate) fn clamp_radii(mut radii: [f32; 4], rect: Rect) -> [f32; 4] {
    for r in &mut radii {
        *r = sane_len(*r);
    }
    if radii.iter().all(|&r| r == 0.0) {
        return radii;
    }
    let [tl, tr, br, bl] = radii;
    let w = sane_len(rect.width);
    let h = sane_len(rect.height);
    let mut f = 1.0f32;
    for (side, sum) in [(w, tl + tr), (h, tr + br), (w, br + bl), (h, bl + tl)] {
        if sum > side && sum > 0.0 {
            f = f.min(side / sum);
        }
    }
    if f < 1.0 {
        for r in &mut radii {
            *r *= f;
        }
    }
    radii
}

/// Radii of the padding edge given the border-box radii and border widths
/// (circular approximation: each radius shrinks by the larger adjacent width).
pub(crate) fn inner_radii(radii: [f32; 4], border: Edges) -> [f32; 4] {
    if radii.iter().all(|&r| r == 0.0) {
        return radii;
    }
    [
        (radii[0] - border.top.max(border.left)).max(0.0),
        (radii[1] - border.top.max(border.right)).max(0.0),
        (radii[2] - border.bottom.max(border.right)).max(0.0),
        (radii[3] - border.bottom.max(border.left)).max(0.0),
    ]
}

/// Computes the origin and size of the first background tile.
///
/// `area` is the background positioning area (the padding box), `natural`
/// the image's natural size. `size` follows `background-size` (`auto` uses
/// the natural size, `cover`/`contain` scale it to the area, explicit sizes
/// with one `auto` keep the aspect ratio) and `position` follows
/// `background-position` (percentages align the same point of the tile and
/// the area: `(area − tile) × p%`). `None` when the tile would be empty or
/// degenerate.
pub fn background_tile(
    area: Rect,
    natural: Size,
    size: BackgroundSize,
    position: (LengthPercentage, LengthPercentage),
) -> Option<(Point, Size)> {
    let area = sane_rect(area);
    let (nw, nh) = (sane_len(natural.width), sane_len(natural.height));
    let has_natural = nw > 0.0 && nh > 0.0;
    let (tw, th) = match size {
        BackgroundSize::Auto => (nw, nh),
        BackgroundSize::Cover | BackgroundSize::Contain => {
            if !has_natural {
                (area.width, area.height)
            } else {
                let sx = area.width / nw;
                let sy = area.height / nh;
                let s = if size == BackgroundSize::Cover {
                    sx.max(sy)
                } else {
                    sx.min(sy)
                };
                (nw * s, nh * s)
            }
        }
        BackgroundSize::Explicit(w, h) => {
            let w = w.map(|w| sane_len(w.resolve(area.width)));
            let h = h.map(|h| sane_len(h.resolve(area.height)));
            match (w, h) {
                (Some(w), Some(h)) => (w, h),
                (Some(w), None) if has_natural => (w, w * nh / nw),
                (None, Some(h)) if has_natural => (h * nw / nh, h),
                (Some(w), None) => (w, area.height),
                (None, Some(h)) => (area.width, h),
                (None, None) => (nw, nh),
            }
        }
    };
    let (tw, th) = (sane_len(tw), sane_len(th));
    if tw < MIN_TILE || th < MIN_TILE {
        return None;
    }
    let x = area.x + sane(position.0.resolve(area.width - tw));
    let y = area.y + sane(position.1.resolve(area.height - th));
    Some((Point::new(sane(x), sane(y)), Size::new(tw, th)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lp(px: f32, pct: f32) -> LengthPercentage {
        LengthPercentage { px, percent: pct }
    }

    #[test]
    fn radii_resolution_and_clamping() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        let radii = resolve_radii(
            &[
                lp(10.0, 0.0),
                lp(0.0, 10.0),
                lp(-5.0, 0.0),
                lp(f32::NAN, 0.0),
            ],
            r,
        );
        assert_eq!(radii, [10.0, 10.0, 0.0, 0.0]);
        // 50% on every corner of a 100x50 box: 50 + 50 > 50 on the vertical sides.
        let radii = resolve_radii(&[lp(0.0, 50.0); 4], r);
        assert!(radii.iter().all(|&v| (v - 25.0).abs() < 1e-4), "{radii:?}");
        assert_eq!(clamp_radii([1e30, 0.0, 0.0, 0.0], r)[0], 50.0);
    }

    #[test]
    fn inner_radii_shrink() {
        let inner = inner_radii([10.0, 2.0, 0.0, 5.0], Edges::new(3.0, 4.0, 1.0, 1.0));
        assert_eq!(inner, [7.0, 0.0, 0.0, 4.0]);
    }

    #[test]
    fn tile_sizes() {
        let area = Rect::new(10.0, 20.0, 200.0, 100.0);
        let nat = Size::new(50.0, 25.0);
        let zero = (LengthPercentage::ZERO, LengthPercentage::ZERO);
        let (o, t) = background_tile(area, nat, BackgroundSize::Auto, zero).unwrap();
        assert_eq!((o, t), (Point::new(10.0, 20.0), nat));
        let (_, t) = background_tile(area, nat, BackgroundSize::Cover, zero).unwrap();
        assert_eq!(t, Size::new(200.0, 100.0));
        let (_, t) =
            background_tile(area, Size::new(50.0, 50.0), BackgroundSize::Contain, zero).unwrap();
        assert_eq!(t, Size::new(100.0, 100.0));
        let (_, t) =
            background_tile(area, Size::new(50.0, 50.0), BackgroundSize::Cover, zero).unwrap();
        assert_eq!(t, Size::new(200.0, 200.0));
        let explicit = BackgroundSize::Explicit(Some(lp(0.0, 50.0)), None);
        let (_, t) = background_tile(area, nat, explicit, zero).unwrap();
        assert_eq!(t, Size::new(100.0, 50.0));
        let explicit = BackgroundSize::Explicit(None, Some(lp(10.0, 0.0)));
        let (_, t) = background_tile(area, nat, explicit, zero).unwrap();
        assert_eq!(t, Size::new(20.0, 10.0));
        assert!(background_tile(area, Size::new(0.0, 0.0), BackgroundSize::Auto, zero).is_none());
        assert!(background_tile(
            area,
            nat,
            BackgroundSize::Explicit(Some(lp(0.0, 0.0)), None),
            zero
        )
        .is_none());
    }

    #[test]
    fn tile_positions() {
        let area = Rect::new(0.0, 0.0, 200.0, 100.0);
        let nat = Size::new(50.0, 20.0);
        let center = (lp(0.0, 50.0), lp(0.0, 50.0));
        let (o, _) = background_tile(area, nat, BackgroundSize::Auto, center).unwrap();
        assert_eq!(o, Point::new(75.0, 40.0));
        let right_bottom = (lp(0.0, 100.0), lp(-5.0, 100.0));
        let (o, _) = background_tile(area, nat, BackgroundSize::Auto, right_bottom).unwrap();
        assert_eq!(o, Point::new(150.0, 75.0));
        let sprite = (lp(-32.0, 0.0), lp(-64.0, 0.0));
        let (o, _) = background_tile(area, nat, BackgroundSize::Auto, sprite).unwrap();
        assert_eq!(o, Point::new(-32.0, -64.0));
    }
}
