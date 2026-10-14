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
///
/// Floats are recorded in placement order, which is also the order of
/// their (non-decreasing) top edges since a float may not be placed above
/// an earlier one; band queries binary-search that order, so documents
/// with thousands of floats stay fast.
#[derive(Clone, Debug)]
pub(crate) struct FloatContext {
    floats: Vec<PlacedFloat>,
    /// A float may not be placed above an earlier float's top.
    min_top: f32,
    /// Tallest float margin box so far (bounds the band search).
    max_height: f32,
    /// Lowest bottom edge of left / right floats.
    bottom_left: Option<f32>,
    bottom_right: Option<f32>,
}

impl Default for FloatContext {
    fn default() -> Self {
        Self::new()
    }
}

impl FloatContext {
    pub fn new() -> Self {
        FloatContext {
            floats: Vec::new(),
            min_top: f32::NEG_INFINITY,
            max_height: 0.0,
            bottom_left: None,
            bottom_right: None,
        }
    }

    /// Number of floats placed so far.
    pub fn len(&self) -> usize {
        self.floats.len()
    }

    /// Floats whose vertical extent intersects the band `[y, y + h)`.
    fn in_band(&self, y: f32, h: f32) -> impl Iterator<Item = &PlacedFloat> {
        let h = h.max(EPSILON);
        // Only floats whose top lies in (y - max_height, y + h) can intersect.
        let lo = self
            .floats
            .partition_point(|f| f.rect.y + self.max_height <= y);
        let hi = self.floats.partition_point(|f| f.rect.y < y + h);
        self.floats
            .get(lo..hi.max(lo))
            .unwrap_or(&[])
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
    pub fn place(
        &mut self,
        side: Side,
        w: f32,
        h: f32,
        min_y: f32,
        left: f32,
        right: f32,
    ) -> Point {
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
        // Keep tops non-decreasing (the binary searches rely on it).
        let y = match self.floats.last() {
            Some(last) if y < last.rect.y => last.rect.y,
            _ => y,
        };
        let rect = Rect::new(x, y, w.max(0.0), h.max(0.0));
        self.floats.push(PlacedFloat { rect, side });
        self.min_top = self.min_top.max(y);
        self.max_height = self.max_height.max(rect.height);
        let slot = match side {
            Side::Left => &mut self.bottom_left,
            Side::Right => &mut self.bottom_right,
        };
        *slot = Some(slot.map_or(rect.bottom(), |b| b.max(rect.bottom())));
        Point::new(x, y)
    }

    /// The y a box with `clear` must be moved to (bottom of the relevant
    /// floats), if any float is cleared.
    pub fn clear_y(&self, clear: Clear) -> Option<f32> {
        match clear {
            Clear::None => None,
            Clear::Left => self.bottom_left,
            Clear::Right => self.bottom_right,
            Clear::Both => self.bottom(),
        }
    }

    /// Lowest float bottom edge.
    pub fn bottom(&self) -> Option<f32> {
        match (self.bottom_left, self.bottom_right) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
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
