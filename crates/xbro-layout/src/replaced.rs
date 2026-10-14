//! Sizing of replaced elements and form controls (CSS 2.1 §10.3.2, §10.6.2
//! and the min/max table of §10.4).
//!
//! * Natural sizes come from [`ReplacedSizes`](crate::ReplacedSizes)
//!   (`node_size` for elements, `url_size` for `content: url()` and list
//!   images). Without one, `<img>`/`<svg>` use their `width`/`height`
//!   attributes for the aspect ratio, `<svg>` its `viewBox`.
//! * An `<img>` with no natural size but a non-empty `alt` is sized like an
//!   inline-block around its alt text (its `lines`), unless `width`/`height`
//!   are specified; with no `alt` it is 0×0 (or its specified size).
//! * `<svg>` defaults to 300×150; `<iframe>`, `<video>`, `<canvas>`,
//!   `<embed>`, `<object>` to 300×150 (`<audio controls>` 300×54).
//! * Form controls get a default content size (see [`ControlSizing`]) that
//!   CSS `width`/`height` override.

use xbro_dom::NodeId;

use crate::block::Cb;
use crate::box_tree::{BoxNode, Control, ControlSizing, Kind, Replaced};
use crate::cx::LayoutCx;
use crate::geom::Size;
use crate::inline::ifc_intrinsic;
use crate::intrinsic::Intrinsic;
use crate::sizing::{
    box_edges, clamp_size, height_limits, sane_size, shrink_to_fit, specified_height,
    specified_width, width_limits, BoxEdges, MAX_LEN,
};

/// Average advance of a text field character, in em.
const TEXT_FIELD_CHAR_EM: f32 = 0.55;

/// Room for the drop-down arrow of a `<select>`, in px.
const SELECT_ARROW: f32 = 24.0;

/// Natural dimensions of replaced content.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Natural {
    width: Option<f32>,
    height: Option<f32>,
    /// Width / height.
    ratio: Option<f32>,
    /// Used for missing dimensions when no ratio can supply them.
    default: Size,
    /// Width to use when only a ratio is known and both sizes are auto
    /// (CSS: fill the containing block).
    fill: bool,
}

impl Natural {
    fn sized(w: f32, h: f32, keep_ratio: bool) -> Self {
        let (w, h) = (sane_size(w), sane_size(h));
        Natural {
            width: Some(w),
            height: Some(h),
            ratio: keep_ratio.then(|| ratio_of(w, h)).flatten(),
            default: Size::new(w, h),
            fill: false,
        }
    }
}

/// A sane aspect ratio `w / h`, if both are positive.
fn ratio_of(w: f32, h: f32) -> Option<f32> {
    let r = w / h;
    (w > 0.0 && h > 0.0 && r.is_finite() && (1.0e-6..=1.0e6).contains(&r)).then_some(r)
}

/// Length of an SVG `width`/`height` attribute.
#[derive(Copy, Clone, Debug, PartialEq)]
enum SvgLen {
    Px(f32),
    Percent(f32),
}

fn parse_svg_len(s: &str) -> Option<SvgLen> {
    let t = s.trim();
    if let Some(p) = t.strip_suffix('%') {
        let v: f32 = p.trim().parse().ok()?;
        return (v.is_finite() && v >= 0.0).then_some(SvgLen::Percent(v.min(1.0e4)));
    }
    let t = t.strip_suffix("px").unwrap_or(t).trim();
    let v: f32 = t.parse().ok()?;
    (v.is_finite() && v >= 0.0).then_some(SvgLen::Px(v.min(MAX_LEN)))
}

/// Aspect ratio of an SVG `viewBox` attribute.
fn viewbox_ratio(s: &str) -> Option<f32> {
    let nums: Vec<f32> = s
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<f32>().ok())
        .collect::<Option<Vec<f32>>>()?;
    match nums.as_slice() {
        [_, _, w, h] => ratio_of(*w, *h),
        _ => None,
    }
}

/// Positive number of an HTML dimension attribute (`width="600"`).
fn attr_px(s: &str) -> Option<f32> {
    let t = s.trim();
    let t = t.strip_suffix("px").unwrap_or(t);
    let v: f32 = t.trim().parse().ok()?;
    (v.is_finite() && v > 0.0).then_some(v.min(MAX_LEN))
}

impl<'t> LayoutCx<'t> {
    /// Natural size of an element from the engine, sanitised.
    fn node_natural(&self, node: Option<NodeId>) -> Option<Size> {
        let s = self.replaced.node_size(node?)?;
        (s.width.is_finite() && s.height.is_finite())
            .then(|| Size::new(sane_size(s.width), sane_size(s.height)))
    }

    /// Aspect ratio from the `width`/`height` attributes.
    fn attr_ratio(&self, node: Option<NodeId>) -> Option<f32> {
        let n = node?;
        let w = self.doc.attr(n, "width").and_then(attr_px)?;
        let h = self.doc.attr(n, "height").and_then(attr_px)?;
        ratio_of(w, h)
    }

    /// True when an `<img>` shows its alt text instead of an image.
    pub(crate) fn shows_alt_text(&self, b: &BoxNode) -> bool {
        matches!(b.kind, Kind::Replaced(Replaced::Image))
            && b.inline_content
            && !b.children.is_empty()
            && self.node_natural(b.node).is_none()
    }

    /// Default content size of a form control.
    fn control_size(&mut self, b: &'t BoxNode, c: &Control) -> Size {
        let font = self.font(&b.style);
        let line = font.line_height;
        match &c.sizing {
            ControlSizing::TextField { size } => {
                Size::new(sane_size(size * TEXT_FIELD_CHAR_EM * font.size), line)
            }
            ControlSizing::Button => {
                let w = if b.inline_content {
                    ifc_intrinsic(self, b).max
                } else {
                    0.0
                };
                Size::new(w, line)
            }
            ControlSizing::Fixed(s) => *s,
            ControlSizing::Select { options } => {
                let ls = crate::sizing::sane(b.style.letter_spacing);
                let ws = crate::sizing::sane(b.style.word_spacing);
                let widest = options
                    .iter()
                    .map(|o| self.measure(font.id, font.size, o, ls, ws))
                    .fold(0.0f32, f32::max);
                Size::new(sane_size(widest + SELECT_ARROW), line)
            }
            ControlSizing::TextArea { cols, rows } => {
                let adv = self.measure(font.id, font.size, "0", 0.0, 0.0);
                Size::new(sane_size(cols * adv), sane_size(rows * line))
            }
        }
    }

    /// Natural dimensions of `b` (`None` for an alt-text image).
    fn natural(&mut self, b: &'t BoxNode, cb_w: Option<f32>) -> Natural {
        let none = Natural {
            width: None,
            height: None,
            ratio: None,
            default: Size::new(0.0, 0.0),
            fill: false,
        };
        let Kind::Replaced(r) = &b.kind else {
            return none;
        };
        match r {
            Replaced::Image => match self.node_natural(b.node) {
                Some(s) => Natural::sized(s.width, s.height, true),
                None => Natural {
                    ratio: self.attr_ratio(b.node),
                    ..none
                },
            },
            Replaced::Svg => {
                if let Some(s) = self.node_natural(b.node) {
                    return Natural::sized(s.width, s.height, true);
                }
                let attr = |n: &str| {
                    b.node
                        .and_then(|node| self.doc.attr(node, n))
                        .and_then(parse_svg_len)
                };
                let resolve = |l: Option<SvgLen>| match l {
                    Some(SvgLen::Px(v)) => Some(v),
                    Some(SvgLen::Percent(p)) => cb_w.map(|w| sane_size(w * p / 100.0)),
                    None => None,
                };
                let w = resolve(attr("width"));
                let h = resolve(attr("height"));
                let vb = b
                    .node
                    .and_then(|n| self.doc.attr(n, "viewBox"))
                    .and_then(viewbox_ratio);
                let ratio = vb.or_else(|| match (w, h) {
                    (Some(w), Some(h)) => ratio_of(w, h),
                    _ => None,
                });
                Natural {
                    width: w,
                    height: h,
                    ratio,
                    default: Size::new(300.0, 150.0),
                    fill: true,
                }
            }
            Replaced::UrlImage(url) => match self.replaced.url_size(url) {
                Some(s) if s.width.is_finite() && s.height.is_finite() => {
                    Natural::sized(s.width, s.height, true)
                }
                _ => Natural::sized(0.0, 0.0, false),
            },
            Replaced::Media {
                default,
                keep_ratio,
            } => match self.node_natural(b.node) {
                Some(s) => Natural::sized(s.width, s.height, true),
                None => Natural::sized(default.width, default.height, *keep_ratio),
            },
            Replaced::Control(c) => {
                let s = self.control_size(b, c);
                Natural::sized(s.width, s.height, false)
            }
        }
    }

    /// Used content width and height of a replaced box against the
    /// containing block `cb`. The height is `None` only for images showing
    /// their alt text with an auto height (their lines then give the height).
    pub fn replaced_size(
        &mut self,
        b: &'t BoxNode,
        edges: &BoxEdges,
        cb: Cb,
    ) -> (f32, Option<f32>) {
        let avail = cb.width - edges.margin.left - edges.margin.right - edges.pb_h();
        self.replaced_dims(b, edges, Some(cb.width), cb.height, avail)
    }

    /// Used content height of replaced box `b` when its content width is
    /// `width` (set by flex or grid layout): the specified height, else the
    /// height given by its aspect ratio (or natural/default height). `None`
    /// for images showing their alt text with an auto height.
    pub(crate) fn replaced_height_for_width(
        &mut self,
        b: &'t BoxNode,
        edges: &BoxEdges,
        cb: Cb,
        width: f32,
    ) -> Option<f32> {
        let style = &b.style;
        let pb_v = edges.pb_v();
        let (min_h, max_h) = height_limits(style, cb.height, pb_v);
        if let Some(h) = specified_height(style, cb.height, pb_v) {
            return Some(clamp_size(h, min_h, max_h));
        }
        if self.shows_alt_text(b) {
            return self
                .attr_ratio(b.node)
                .map(|r| clamp_size(width / r, min_h, max_h));
        }
        let mut nat = self.natural(b, Some(cb.width));
        if let Some(r) = style.aspect_ratio.and_then(|r| ratio_of(r, 1.0)) {
            nat.ratio = Some(r);
        }
        let h = nat
            .ratio
            .map(|r| sane_size(width) / r)
            .or(nat.height)
            .unwrap_or(nat.default.height);
        Some(clamp_size(h, min_h, max_h))
    }

    fn replaced_dims(
        &mut self,
        b: &'t BoxNode,
        edges: &BoxEdges,
        cb_w: Option<f32>,
        cb_h: Option<f32>,
        avail: f32,
    ) -> (f32, Option<f32>) {
        let style = &b.style;
        let pb_h = edges.pb_h();
        let pb_v = edges.pb_v();
        let spec_w = specified_width(style, cb_w, pb_h);
        let spec_h = specified_height(style, cb_h, pb_v);
        let (min_w, max_w) = width_limits(style, cb_w, pb_h);
        let (min_h, max_h) = height_limits(style, cb_h, pb_v);
        let cw = |w: f32| clamp_size(w, min_w, max_w);
        let ch = |h: f32| clamp_size(h, min_h, max_h);

        if self.shows_alt_text(b) {
            let w = match spec_w {
                Some(w) => cw(w),
                None => {
                    let i = ifc_intrinsic(self, b);
                    cw(shrink_to_fit(i.min, i.max, avail))
                }
            };
            let ratio = self.attr_ratio(b.node);
            let h = spec_h.map(ch).or_else(|| {
                // Only width given (e.g. `height: auto` overriding the attribute).
                spec_w.and(ratio).map(|r| ch(w / r))
            });
            return (w, h);
        }

        let mut nat = self.natural(b, cb_w);
        if let Some(r) = style.aspect_ratio.and_then(|r| ratio_of(r, 1.0)) {
            nat.ratio = Some(r);
        }
        // `<svg width="50%">` behaves like a percentage CSS width.
        let ratio = nat.ratio;
        let (w, h) = match (spec_w, spec_h) {
            (Some(w), Some(h)) => (cw(w), ch(h)),
            (Some(w), None) => {
                let w = cw(w);
                let h = ratio
                    .map(|r| w / r)
                    .or(nat.height)
                    .unwrap_or(nat.default.height);
                (w, ch(h))
            }
            (None, Some(h)) => {
                let h = ch(h);
                let w = ratio
                    .map(|r| h * r)
                    .or(nat.width)
                    .unwrap_or(nat.default.width);
                (cw(w), h)
            }
            (None, None) => {
                let (w, h) = match (nat.width, nat.height, ratio) {
                    (Some(w), Some(h), _) => (w, h),
                    (Some(w), None, Some(r)) => (w, w / r),
                    (None, Some(h), Some(r)) => (h * r, h),
                    (None, None, Some(r)) => {
                        let w = if nat.fill && avail.is_finite() && avail > 0.0 {
                            avail
                        } else {
                            nat.default.width
                        };
                        (w, w / r)
                    }
                    (w, h, _) => (
                        w.unwrap_or(nat.default.width),
                        h.unwrap_or(nat.default.height),
                    ),
                };
                if ratio.is_some() {
                    constrain_with_ratio(sane_size(w), sane_size(h), (min_w, max_w), (min_h, max_h))
                } else {
                    (cw(w), ch(h))
                }
            }
        };
        (sane_size(w), Some(sane_size(h)))
    }

    /// Intrinsic widths of a replaced box's content box. Percentage widths
    /// (or max-widths) make the min-content contribution zero
    /// (compressible replaced elements).
    pub(crate) fn replaced_intrinsic(&mut self, b: &'t BoxNode) -> Intrinsic {
        let style = &b.style;
        let edges = box_edges(style, 0.0);
        if self.shows_alt_text(b) {
            return match specified_width(style, None, edges.pb_h()) {
                Some(w) => {
                    let (min_w, max_w) = width_limits(style, None, edges.pb_h());
                    Intrinsic::exact(clamp_size(w, min_w, max_w))
                }
                None => ifc_intrinsic(self, b),
            };
        }
        let (w, _) = self.replaced_dims(b, &edges, None, None, f32::NAN);
        let compressible = match (&style.width, &style.max_width) {
            (xbro_style::LengthPercentageAuto::Length(l), _) if l.has_percent() => true,
            (_, xbro_style::MaxSize::Length(l)) if l.has_percent() => true,
            _ => false,
        };
        if compressible {
            Intrinsic::new(0.0, w)
        } else {
            Intrinsic::exact(w)
        }
    }
}

/// CSS 2.1 §10.4 table: both `width` and `height` auto with an aspect
/// ratio, natural size `w × h`.
fn constrain_with_ratio(
    w: f32,
    h: f32,
    (min_w, max_w): (f32, f32),
    (min_h, max_h): (f32, f32),
) -> (f32, f32) {
    if w <= 0.0 || h <= 0.0 {
        return (clamp_size(w, min_w, max_w), clamp_size(h, min_h, max_h));
    }
    let max_w = max_w.max(min_w);
    let max_h = max_h.max(min_h);
    let over_w = w > max_w;
    let under_w = w < min_w;
    let over_h = h > max_h;
    let under_h = h < min_h;
    let (rw, rh) = match (over_w, under_w, over_h, under_h) {
        (false, false, false, false) => (w, h),
        (true, _, false, false) => (max_w, (max_w * h / w).max(min_h)),
        (_, true, false, false) => (min_w, (min_w * h / w).min(max_h)),
        (false, false, true, _) => ((max_h * w / h).max(min_w), max_h),
        (false, false, _, true) => ((min_h * w / h).min(max_w), min_h),
        (true, _, true, _) => {
            if max_w / w <= max_h / h {
                (max_w, min_h.max(max_w * h / w))
            } else {
                (min_w.max(max_h * w / h), max_h)
            }
        }
        (_, true, _, true) => {
            if min_w / w <= min_h / h {
                (max_w.min(min_h * w / h), min_h)
            } else {
                (min_w, max_h.min(min_w * h / w))
            }
        }
        (_, true, true, _) => (min_w, max_h),
        (true, _, _, true) => (max_w, min_h),
    };
    (sane_size(rw), sane_size(rh))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INF: f32 = f32::INFINITY;

    #[test]
    fn ratio_table() {
        // No constraint.
        assert_eq!(
            constrain_with_ratio(200.0, 100.0, (0.0, INF), (0.0, INF)),
            (200.0, 100.0)
        );
        // max-width: scales the height.
        assert_eq!(
            constrain_with_ratio(200.0, 100.0, (0.0, 100.0), (0.0, INF)),
            (100.0, 50.0)
        );
        // max-height.
        assert_eq!(
            constrain_with_ratio(200.0, 100.0, (0.0, INF), (0.0, 50.0)),
            (100.0, 50.0)
        );
        // min-width.
        assert_eq!(
            constrain_with_ratio(20.0, 10.0, (40.0, INF), (0.0, INF)),
            (40.0, 20.0)
        );
        // Both too big, width more constrained.
        assert_eq!(
            constrain_with_ratio(400.0, 100.0, (0.0, 100.0), (0.0, 50.0)),
            (100.0, 25.0)
        );
        // Degenerate.
        assert_eq!(
            constrain_with_ratio(0.0, 10.0, (0.0, INF), (0.0, 5.0)),
            (0.0, 5.0)
        );
    }

    #[test]
    fn attribute_parsing() {
        assert_eq!(parse_svg_len("24"), Some(SvgLen::Px(24.0)));
        assert_eq!(parse_svg_len(" 24px "), Some(SvgLen::Px(24.0)));
        assert_eq!(parse_svg_len("50%"), Some(SvgLen::Percent(50.0)));
        assert_eq!(parse_svg_len("1em"), None);
        assert_eq!(parse_svg_len("-3"), None);
        assert_eq!(viewbox_ratio("0 0 24 12"), Some(2.0));
        assert_eq!(viewbox_ratio("0,0,10,0"), None);
        assert_eq!(viewbox_ratio("0 0 a 1"), None);
        assert_eq!(attr_px("600"), Some(600.0));
        assert_eq!(attr_px("0"), None);
        assert_eq!(ratio_of(1.0, 0.0), None);
    }
}
