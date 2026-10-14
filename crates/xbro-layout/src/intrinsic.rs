//! Intrinsic inline sizes (CSS Sizing 3 min-content / max-content widths).
//!
//! [`LayoutCx::content_intrinsic`] gives the min/max-content widths of a
//! box's *content box*; it is cached on the [`BoxNode`] for the duration of
//! the layout pass, so nested shrink-to-fit boxes (floats in inline-blocks in
//! table cells…) are measured once each and never exponentially.
//! [`LayoutCx::outer_intrinsic`] gives the *margin box* contributions used by
//! a parent (specified widths, `min-width`/`max-width`, padding, borders and
//! non-auto margins included; percentages count as zero / auto).

use xbro_style::Clear;

use crate::box_tree::{BoxNode, Kind};
use crate::cx::LayoutCx;
use crate::inline::ifc_intrinsic;
use crate::sizing::{
    box_edges, clamp_size, height_limits, ratio_width, sane_size, specified_height,
    specified_width, width_limits,
};
use crate::{flex, table};

/// Min-content and max-content widths, in CSS px.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct Intrinsic {
    pub min: f32,
    pub max: f32,
}

impl Intrinsic {
    /// Sanitised pair (`max >= min >= 0`).
    pub fn new(min: f32, max: f32) -> Self {
        let min = sane_size(min);
        Intrinsic {
            min,
            max: sane_size(max).max(min),
        }
    }

    /// Both widths set to `w`.
    pub fn exact(w: f32) -> Self {
        Self::new(w, w)
    }

    /// Adds `v` to both widths.
    pub fn plus(self, v: f32) -> Self {
        Self::new(self.min + v, self.max + v)
    }
}

impl<'t> LayoutCx<'t> {
    /// Min/max-content widths of the content box of `b` (cached per pass).
    pub fn content_intrinsic(&mut self, b: &'t BoxNode) -> Intrinsic {
        if let Some(i) = b.intrinsic.get() {
            return i;
        }
        // Provisional value in case of (impossible) re-entrance.
        b.intrinsic.set(Some(Intrinsic::default()));
        let i = self.compute_content_intrinsic(b);
        b.intrinsic.set(Some(i));
        i
    }

    fn compute_content_intrinsic(&mut self, b: &'t BoxNode) -> Intrinsic {
        match &b.kind {
            Kind::Flex => flex::flex_intrinsic(self, b),
            Kind::Table => table::table_intrinsic(self, b),
            Kind::TableRowGroup | Kind::TableRow => table::table_part_intrinsic(self, b),
            Kind::Replaced(_) => self.replaced_intrinsic(b),
            Kind::Inline | Kind::Text(_) | Kind::LineBreak | Kind::WordBreak => {
                Intrinsic::default()
            }
            Kind::Block | Kind::TableCell => {
                if b.inline_content {
                    ifc_intrinsic(self, b)
                } else {
                    self.block_children_intrinsic(b)
                }
            }
        }
    }

    /// Intrinsic widths of a block container with block-level children.
    /// Consecutive floats are counted side by side for the max-content width.
    fn block_children_intrinsic(&mut self, b: &'t BoxNode) -> Intrinsic {
        let mut min = 0.0f32;
        let mut max = 0.0f32;
        let mut float_line = 0.0f32;
        for c in &b.children {
            if c.is_absolute() || c.is_text_like() || matches!(c.kind, Kind::Inline) {
                continue;
            }
            let o = self.outer_intrinsic(c);
            min = min.max(o.min);
            if c.is_float() {
                if c.style.clear != Clear::None {
                    float_line = 0.0;
                }
                float_line += o.max;
                max = max.max(float_line);
            } else {
                if c.style.clear != Clear::None {
                    float_line = 0.0;
                }
                max = max.max(o.max);
            }
        }
        Intrinsic::new(min, max)
    }

    /// Margin-box min/max-content contributions of `b` to its parent.
    pub fn outer_intrinsic(&mut self, b: &'t BoxNode) -> Intrinsic {
        let style = &b.style;
        let edges = box_edges(style, 0.0);
        let pb = edges.pb_h();
        let mbp = pb + edges.margin.left + edges.margin.right;
        let (min_w, max_w) = width_limits(style, None, pb);
        let inner = if b.is_replaced() {
            // Replaced sizing already honours width/min/max.
            self.content_intrinsic(b)
        } else {
            let from_ratio = || {
                if matches!(b.kind, Kind::Table | Kind::TableCell) {
                    return None;
                }
                let pb_v = edges.pb_v();
                let (min_h, max_h) = height_limits(style, None, pb_v);
                let h = specified_height(style, None, pb_v)?;
                ratio_width(style, clamp_size(h, min_h, max_h), pb, pb_v)
            };
            match specified_width(style, None, pb).or_else(from_ratio) {
                Some(w) => {
                    let w = clamp_size(w, min_w, max_w);
                    match &b.kind {
                        // Tables are never narrower than their content.
                        Kind::Table => {
                            let i = self.content_intrinsic(b);
                            Intrinsic::exact(w.max(i.min))
                        }
                        _ => Intrinsic::exact(w),
                    }
                }
                None => {
                    let i = self.content_intrinsic(b);
                    let floor = if matches!(b.kind, Kind::Table) {
                        i.min
                    } else {
                        0.0
                    };
                    Intrinsic::new(
                        clamp_size(i.min, min_w, max_w).max(floor),
                        clamp_size(i.max, min_w, max_w).max(floor),
                    )
                }
            }
        };
        inner.plus(mbp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitised() {
        let i = Intrinsic::new(f32::NAN, -3.0);
        assert_eq!(i, Intrinsic { min: 0.0, max: 0.0 });
        let j = Intrinsic::new(10.0, 5.0).plus(2.0);
        assert_eq!(
            j,
            Intrinsic {
                min: 12.0,
                max: 12.0
            }
        );
    }
}
