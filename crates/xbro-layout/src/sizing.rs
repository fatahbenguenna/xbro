//! Resolution of box-model values (lengths, percentages, `auto`, min/max
//! constraints, `box-sizing`) into used px values, with sanitising so that
//! hostile values (`NaN`, `inf`, `1e30px`) never propagate into geometry.

use xbro_style::{
    BoxSizing, ComputedStyle, LengthPercentage, LengthPercentageAuto, MaxSize, Overflow,
};

use crate::geom::Edges;

/// Largest magnitude of any single resolved length, in CSS px.
pub(crate) const MAX_LEN: f32 = 1.0e7;

/// Tolerance used when comparing widths (line fitting, float placement).
pub(crate) const EPSILON: f32 = 0.01;

/// Finite value clamped to ±[`MAX_LEN`] (`NaN`/`inf` → 0).
#[inline]
pub(crate) fn sane(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(-MAX_LEN, MAX_LEN)
    } else {
        0.0
    }
}

/// Like [`sane`], and never negative.
#[inline]
pub(crate) fn sane_size(v: f32) -> f32 {
    sane(v).max(0.0)
}

/// Resolves a length-percentage against `reference`.
#[inline]
pub(crate) fn lp(v: &LengthPercentage, reference: f32) -> f32 {
    sane(v.resolve(sane(reference)))
}

/// `None` for `auto`, else the resolved value.
#[inline]
pub(crate) fn lpa(v: &LengthPercentageAuto, reference: f32) -> Option<f32> {
    v.resolve(sane(reference)).map(sane)
}

/// `None` for `auto` and for percentages of an indefinite reference.
#[inline]
pub(crate) fn lpa_definite(v: &LengthPercentageAuto, reference: Option<f32>) -> Option<f32> {
    v.resolve_definite(reference.map(sane)).map(sane)
}

/// Used padding, border and margins of a box, plus which margins are `auto`
/// (auto margins are reported as 0 here; callers resolve them).
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct BoxEdges {
    pub padding: Edges,
    pub border: Edges,
    pub margin: Edges,
    pub auto_top: bool,
    pub auto_right: bool,
    pub auto_bottom: bool,
    pub auto_left: bool,
}

impl BoxEdges {
    /// Horizontal padding + border.
    pub fn pb_h(&self) -> f32 {
        self.padding.horizontal() + self.border.horizontal()
    }

    /// Vertical padding + border.
    pub fn pb_v(&self) -> f32 {
        self.padding.vertical() + self.border.vertical()
    }
}

/// Resolves padding, border and margins. Percentages (vertical ones too)
/// refer to the containing block width, per CSS 2.1.
pub(crate) fn box_edges(style: &ComputedStyle, cb_width: f32) -> BoxEdges {
    let p = &style.padding;
    let padding = Edges::new(
        lp(&p.top, cb_width).max(0.0),
        lp(&p.right, cb_width).max(0.0),
        lp(&p.bottom, cb_width).max(0.0),
        lp(&p.left, cb_width).max(0.0),
    );
    let b = style.border_widths();
    let border = Edges::new(
        sane_size(b.top),
        sane_size(b.right),
        sane_size(b.bottom),
        sane_size(b.left),
    );
    let m = &style.margin;
    let margin = Edges::new(
        lpa(&m.top, cb_width).unwrap_or(0.0),
        lpa(&m.right, cb_width).unwrap_or(0.0),
        lpa(&m.bottom, cb_width).unwrap_or(0.0),
        lpa(&m.left, cb_width).unwrap_or(0.0),
    );
    BoxEdges {
        padding,
        border,
        margin,
        auto_top: m.top.is_auto(),
        auto_right: m.right.is_auto(),
        auto_bottom: m.bottom.is_auto(),
        auto_left: m.left.is_auto(),
    }
}

/// Converts a `width`/`height`-like value to a content-box size per `box-sizing`.
#[inline]
fn to_content(style: &ComputedStyle, v: f32, pb: f32) -> f32 {
    match style.box_sizing {
        BoxSizing::ContentBox => v.max(0.0),
        BoxSizing::BorderBox => (v - pb).max(0.0),
    }
}

/// Specified content width, `None` for `auto` (or a percentage of an unknown width).
pub(crate) fn specified_width(style: &ComputedStyle, cb_w: Option<f32>, pb_h: f32) -> Option<f32> {
    lpa_definite(&style.width, cb_w).map(|w| to_content(style, w, pb_h))
}

/// Specified content height, `None` for `auto` or a percentage of an indefinite height.
pub(crate) fn specified_height(style: &ComputedStyle, cb_h: Option<f32>, pb_v: f32) -> Option<f32> {
    lpa_definite(&style.height, cb_h).map(|h| to_content(style, h, pb_v))
}

/// Used `min-width` as a content width (`auto` → 0).
pub(crate) fn min_width(style: &ComputedStyle, cb_w: Option<f32>, pb_h: f32) -> f32 {
    lpa_definite(&style.min_width, cb_w)
        .map(|w| to_content(style, w, pb_h))
        .unwrap_or(0.0)
}

/// Used `max-width` as a content width (`none` → infinity).
pub(crate) fn max_width(style: &ComputedStyle, cb_w: Option<f32>, pb_h: f32) -> f32 {
    max_size(&style.max_width, cb_w)
        .map(|w| to_content(style, w, pb_h))
        .unwrap_or(f32::INFINITY)
}

/// Used `min-height` as a content height.
pub(crate) fn min_height(style: &ComputedStyle, cb_h: Option<f32>, pb_v: f32) -> f32 {
    lpa_definite(&style.min_height, cb_h)
        .map(|h| to_content(style, h, pb_v))
        .unwrap_or(0.0)
}

/// Used `max-height` as a content height.
pub(crate) fn max_height(style: &ComputedStyle, cb_h: Option<f32>, pb_v: f32) -> f32 {
    max_size(&style.max_height, cb_h)
        .map(|h| to_content(style, h, pb_v))
        .unwrap_or(f32::INFINITY)
}

fn max_size(v: &MaxSize, reference: Option<f32>) -> Option<f32> {
    match (v, reference) {
        (MaxSize::None, _) => None,
        (MaxSize::Length(l), Some(r)) => Some(lp(l, r)),
        (MaxSize::Length(l), None) if !l.has_percent() => Some(sane(l.px)),
        (MaxSize::Length(_), None) => None,
    }
}

/// A scroll container (`overflow` other than `visible`/`clip` on an
/// axis): flex and grid items that are have no automatic minimum size.
pub(crate) fn is_scroll_container(style: &ComputedStyle) -> bool {
    let scrolls = |o: Overflow| !matches!(o, Overflow::Visible | Overflow::Clip);
    scrolls(style.overflow_x) || scrolls(style.overflow_y)
}

/// `max(min, min(value, max))`: `min-*` wins over `max-*`.
#[inline]
pub(crate) fn clamp_size(v: f32, min: f32, max: f32) -> f32 {
    sane_size(v.min(max).max(min))
}

/// Width constraints of a box: min/max in content-box px.
pub(crate) fn width_limits(style: &ComputedStyle, cb_w: Option<f32>, pb_h: f32) -> (f32, f32) {
    (min_width(style, cb_w, pb_h), max_width(style, cb_w, pb_h))
}

/// Height constraints of a box: min/max in content-box px.
pub(crate) fn height_limits(style: &ComputedStyle, cb_h: Option<f32>, pb_v: f32) -> (f32, f32) {
    (min_height(style, cb_h, pb_v), max_height(style, cb_h, pb_v))
}

/// Width and horizontal margins of a block-level, non-replaced box in normal
/// flow (CSS 2.1 §10.3.3, including the min/max rules of §10.4).
///
/// `avail` is the width the margin box must fill (the containing block
/// width, or a narrower band next to floats). Returns
/// `(content_width, margin_left, margin_right)`.
pub(crate) fn block_width(
    style: &ComputedStyle,
    edges: &BoxEdges,
    cb_w: f32,
    avail: f32,
    specified: Option<f32>,
) -> (f32, f32, f32) {
    let pb = edges.pb_h();
    let (min, max) = width_limits(style, Some(cb_w), pb);
    let solve = |w: Option<f32>| -> (f32, f32, f32) {
        let ml = edges.margin.left;
        let mr = edges.margin.right;
        match w {
            None => {
                // Auto width: auto margins are 0, the width fills the space.
                (sane_size(avail - ml - mr - pb), ml, mr)
            }
            Some(w) => {
                let rest = avail - w - pb;
                match (edges.auto_left, edges.auto_right) {
                    (true, true) => {
                        let m = rest / 2.0;
                        // Over-constrained centering never pushes the box left.
                        if m < 0.0 {
                            (w, 0.0, rest)
                        } else {
                            (w, m, m)
                        }
                    }
                    (true, false) => (w, sane(rest - mr), mr),
                    (false, true) => (w, ml, sane(rest - ml)),
                    // Over-constrained: margin-right is ignored (ltr).
                    (false, false) => (w, ml, mr),
                }
            }
        }
    };
    let (w, ml, mr) = solve(specified);
    let clamped = clamp_size(w, min, max);
    if (clamped - w).abs() > EPSILON {
        solve(Some(clamped))
    } else {
        (w, ml, mr)
    }
}

/// Sanitised `aspect-ratio` (width / height) of a box, if any.
pub(crate) fn aspect_ratio(style: &ComputedStyle) -> Option<f32> {
    style
        .aspect_ratio
        .filter(|r| r.is_finite() && (1.0e-6..=1.0e6).contains(r))
}

/// Content height that `aspect-ratio` gives a box of content width `w`
/// (the ratio applies to the box selected by `box-sizing`).
pub(crate) fn ratio_height(style: &ComputedStyle, w: f32, pb_h: f32, pb_v: f32) -> Option<f32> {
    let r = aspect_ratio(style)?;
    Some(match style.box_sizing {
        BoxSizing::ContentBox => sane_size(w / r),
        BoxSizing::BorderBox => sane_size((w + pb_h) / r - pb_v),
    })
}

/// Content width that `aspect-ratio` gives a box of content height `h`.
pub(crate) fn ratio_width(style: &ComputedStyle, h: f32, pb_h: f32, pb_v: f32) -> Option<f32> {
    let r = aspect_ratio(style)?;
    Some(match style.box_sizing {
        BoxSizing::ContentBox => sane_size(h * r),
        BoxSizing::BorderBox => sane_size((h + pb_v) * r - pb_h),
    })
}

/// Used content height of a non-replaced box whose height comes from its
/// `aspect-ratio` (`ratio_h`) rather than from `height`, given its auto
/// (content) height: `min-height`/`max-height` apply, and with `min-height:
/// auto` the box still grows to fit its content unless it is a scroll
/// container (CSS Sizing 4 §5.1, automatic minimum size).
pub(crate) fn ratio_used_height(
    style: &ComputedStyle,
    ratio_h: f32,
    auto_h: f32,
    (min_h, max_h): (f32, f32),
) -> f32 {
    let h = clamp_size(ratio_h, min_h, max_h);
    let clips = style.overflow_x.clips() || style.overflow_y.clips();
    if style.min_height.is_auto() && !clips {
        h.max(sane_size(auto_h).min(max_h))
    } else {
        h
    }
}

/// Shrink-to-fit width (CSS 2.1 §10.3.5): `min(max(min_content, avail), max_content)`.
#[inline]
pub(crate) fn shrink_to_fit(min_content: f32, max_content: f32, avail: f32) -> f32 {
    sane_size(min_content.max(avail.min(max_content)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use xbro_style::Sides;

    fn style() -> ComputedStyle {
        ComputedStyle::default()
    }

    #[test]
    fn sanitising() {
        assert_eq!(sane(f32::NAN), 0.0);
        assert_eq!(sane(f32::INFINITY), 0.0);
        assert_eq!(sane(1e30), MAX_LEN);
        assert_eq!(sane_size(-5.0), 0.0);
    }

    #[test]
    fn auto_width_fills() {
        let mut s = style();
        s.margin = Sides::all(LengthPercentageAuto::px(10.0));
        s.padding = Sides::all(LengthPercentage::px(5.0));
        let e = box_edges(&s, 500.0);
        assert_eq!(block_width(&s, &e, 500.0, 500.0, None), (470.0, 10.0, 10.0));
    }

    #[test]
    fn auto_margins_center() {
        let mut s = style();
        s.margin.left = LengthPercentageAuto::Auto;
        s.margin.right = LengthPercentageAuto::Auto;
        s.width = LengthPercentageAuto::px(200.0);
        let e = box_edges(&s, 500.0);
        let w = specified_width(&s, Some(500.0), e.pb_h());
        assert_eq!(block_width(&s, &e, 500.0, 500.0, w), (200.0, 150.0, 150.0));
    }

    #[test]
    fn border_box_and_limits() {
        let mut s = style();
        s.box_sizing = BoxSizing::BorderBox;
        s.width = LengthPercentageAuto::percent(50.0);
        s.padding = Sides::all(LengthPercentage::px(10.0));
        s.max_width = MaxSize::Length(LengthPercentage::px(100.0));
        let e = box_edges(&s, 400.0);
        let w = specified_width(&s, Some(400.0), e.pb_h());
        assert_eq!(w, Some(180.0));
        let (cw, _, _) = block_width(&s, &e, 400.0, 400.0, w);
        assert_eq!(cw, 80.0);
    }

    #[test]
    fn indefinite_percent_height_is_auto() {
        let mut s = style();
        s.height = LengthPercentageAuto::percent(50.0);
        assert_eq!(specified_height(&s, None, 0.0), None);
        assert_eq!(specified_height(&s, Some(300.0), 0.0), Some(150.0));
    }
}
