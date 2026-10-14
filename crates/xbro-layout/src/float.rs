//! Float placement (CSS 2.1 §9.5): one [`FloatContext`] per block formatting
//! context, in absolute coordinates.

use xbro_style::Clear;

use crate::geom::{Point, Rect};
use crate::sizing::EPSILON;

/// Side a float is attached to.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Left,
    Right,
}

#[derive(Copy, Clone, Debug)]
struct PlacedFloat {
    /// Margin box.
    rect: Rect,
    side: Side,
}

/// Floats placed so far in one block formatting context.
#[derive(Clone, Debug, Default)]
pub(crate) struct FloatContext {
    floats: Vec<PlacedFloat>,
    /// A float may not be placed above an earlier float's top.
    min_top: f32,
}

impl FloatContext {
    pub fn new() -> Self {
        FloatContext {
            floats: Vec::new(),
            min_top: f32::NEG_INFINITY,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.floats.is_empty()
    }

    /// Floats whose vertical extent intersects the band `[y, y + h)`.
    fn in_band(&self, y: f32, h: f32) -> impl Iterator<Item = &PlacedFloat> {
        let h = h.max(EPSILON);
        self.floats
            .iter()
            .filter(move |f| f.rect.y < y + h && f.rect.bottom() > y && f.rect.height > 0.0)
    }

    /// Horizontal space `(left, right)` left by floats in the band `[y, y + h)`
    /// within `[left, right]`.
    pub fn available(&self, y: f32, h: f32, left: f32, right: f32) -> (f32, f32) {
        let mut l = left;
        let mut r = right;
        for f in self.in_band(y, h) {
            match f.side {
                Side::Left => l = l.max(f.rect.right()),
                Side::Right => r = r.min(f.rect.x),
            }
        }
        (l, r.max(l))
    }

    /// True when some float intersects the band and narrows `[left, right]`.
    pub fn narrows(&self, y: f32, h: f32, left: f32, right: f32) -> bool {
        let (l, r) = self.available(y, h, left, right);
        l > left + EPSILON || r < right - EPSILON
    }

    /// Smallest float bottom edge below `y` among floats in the band, used to
    /// move content down until it fits.
    pub fn next_bottom(&self, y: f32, h: f32) -> Option<f32> {
        self.in_band(y, h)
            .map(|f| f.rect.bottom())
            .filter(|&b| b > y + EPSILON / 2.0)
            .min_by(f32::total_cmp)
    }

    /// Places a float with margin box `w × h` whose top may not be above
    /// `min_y`, inside the containing block edges `[left, right]`. Returns
    /// the margin box origin and records the float.
    pub fn place(&mut self, side: Side, w: f32, h: f32, min_y: f32, left: f32, right: f32) -> Point {
        let mut y = min_y.max(self.min_top);
        if !y.is_finite() {
            y = min_y;
        }
        // Each iteration moves below at least one float: bounded.
        for _ in 0..=self.floats.len() {
            let (l, r) = self.available(y, h, left, right);
            let fits = r - l + EPSILON >= w;
            if fits || self.in_band(y, h).next().is_none() {
                return self.commit(side, w, h, l, r, y);
            }
            match self.next_bottom(y, h) {
                Some(next) => y = next,
                None => return self.commit(side, w, h, l, r, y),
            }
        }
        let (l, r) = self.available(y, h, left, right);
        self.commit(side, w, h, l, r, y)
    }

    fn commit(&mut self, side: Side, w: f32, h: f32, l: f32, r: f32, y: f32) -> Point {
        let x = match side {
            Side::Left => l,
            Side::Right => r - w,
        };
        self.floats.push(PlacedFloat {
            rect: Rect::new(x, y, w.max(0.0), h.max(0.0)),
            side,
        });
        self.min_top = self.min_top.max(y);
        Point::new(x, y)
    }

    /// The y a box with `clear` must be moved to (bottom of the relevant
    /// floats), if any float is cleared.
    pub fn clear_y(&self, clear: Clear) -> Option<f32> {
        let wanted = |s: Side| match clear {
            Clear::None => false,
            Clear::Left => s == Side::Left,
            Clear::Right => s == Side::Right,
            Clear::Both => true,
        };
        self.floats
            .iter()
            .filter(|f| wanted(f.side))
            .map(|f| f.rect.bottom())
            .max_by(f32::total_cmp)
    }

    /// Lowest float bottom edge.
    pub fn bottom(&self) -> Option<f32> {
        self.floats
            .iter()
            .map(|f| f.rect.bottom())
            .max_by(f32::total_cmp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_and_right_floats_share_a_band() {
        let mut fc = FloatContext::new();
        let a = fc.place(Side::Left, 100.0, 50.0, 0.0, 0.0, 500.0);
        let b = fc.place(Side::Right, 100.0, 80.0, 0.0, 0.0, 500.0);
        let c = fc.place(Side::Left, 100.0, 20.0, 0.0, 0.0, 500.0);
        assert_eq!(a, Point::new(0.0, 0.0));
        assert_eq!(b, Point::new(400.0, 0.0));
        assert_eq!(c, Point::new(100.0, 0.0));
        assert_eq!(fc.available(10.0, 10.0, 0.0, 500.0), (200.0, 400.0));
        assert_eq!(fc.available(60.0, 10.0, 0.0, 500.0), (0.0, 400.0));
        assert_eq!(fc.clear_y(Clear::Left), Some(50.0));
        assert_eq!(fc.clear_y(Clear::Both), Some(80.0));
        assert_eq!(fc.bottom(), Some(80.0));
    }

    #[test]
    fn float_moves_down_when_it_does_not_fit() {
        let mut fc = FloatContext::new();
        fc.place(Side::Left, 300.0, 50.0, 0.0, 0.0, 500.0);
        let p = fc.place(Side::Left, 300.0, 50.0, 0.0, 0.0, 500.0);
        assert_eq!(p, Point::new(0.0, 50.0));
        // Never above an earlier float.
        let q = fc.place(Side::Right, 10.0, 10.0, 0.0, 0.0, 500.0);
        assert_eq!(q.y, 50.0);
        assert_eq!(fc.next_bottom(0.0, 10.0), Some(50.0));
    }

    #[test]
    fn too_wide_float_is_placed_anyway() {
        let mut fc = FloatContext::new();
        let p = fc.place(Side::Left, 900.0, 10.0, 5.0, 0.0, 500.0);
        assert_eq!(p, Point::new(0.0, 5.0));
    }
}
