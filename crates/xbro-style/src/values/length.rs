//! Specified lengths as linear combinations of units, and their resolution
//! to computed [`LengthPercentage`]s.

use xbro_css::LengthUnit;

use crate::computed::LengthPercentage;

/// Largest magnitude a computed length may have (keeps layout arithmetic finite).
const MAX_PX: f32 = 1.0e7;

/// Replaces NaN by 0 and clamps to `±MAX_PX`, so arbitrary input never
/// produces non-finite computed values.
pub(crate) fn sane(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(-MAX_PX, MAX_PX)
    }
}

/// A length written as a sum of unit terms: `calc()` sums and products by
/// numbers keep lengths linear, so every `calc()` without `min()`/`max()`/
/// `clamp()` folds into one of these at parse time.
///
/// Absolute units are folded into `px`, `ex` into `em` (`1ex = 0.5em`).
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub(crate) struct LinearLen {
    pub px: f32,
    pub em: f32,
    pub rem: f32,
    pub ch: f32,
    pub vw: f32,
    pub vh: f32,
    pub vmin: f32,
    pub vmax: f32,
    pub percent: f32,
}

impl LinearLen {
    pub const ZERO: Self = Self {
        px: 0.0,
        em: 0.0,
        rem: 0.0,
        ch: 0.0,
        vw: 0.0,
        vh: 0.0,
        vmin: 0.0,
        vmax: 0.0,
        percent: 0.0,
    };

    pub const fn px(px: f32) -> Self {
        Self { px, ..Self::ZERO }
    }

    pub const fn percent(percent: f32) -> Self {
        Self {
            percent,
            ..Self::ZERO
        }
    }

    /// `value` in `unit`.
    pub fn from_unit(value: f32, unit: LengthUnit) -> Self {
        let mut l = Self::ZERO;
        match unit {
            LengthUnit::Em => l.em = value,
            LengthUnit::Ex => l.em = value * 0.5,
            LengthUnit::Rem => l.rem = value,
            LengthUnit::Ch => l.ch = value,
            LengthUnit::Vw => l.vw = value,
            LengthUnit::Vh => l.vh = value,
            LengthUnit::Vmin => l.vmin = value,
            LengthUnit::Vmax => l.vmax = value,
            LengthUnit::Percent => l.percent = value,
            absolute => l.px = value * absolute.absolute_px().unwrap_or(1.0),
        }
        l
    }

    fn terms(&self) -> [f32; 9] {
        [
            self.px,
            self.em,
            self.rem,
            self.ch,
            self.vw,
            self.vh,
            self.vmin,
            self.vmax,
            self.percent,
        ]
    }

    fn from_terms(t: [f32; 9]) -> Self {
        Self {
            px: t[0],
            em: t[1],
            rem: t[2],
            ch: t[3],
            vw: t[4],
            vh: t[5],
            vmin: t[6],
            vmax: t[7],
            percent: t[8],
        }
    }

    pub fn add(self, other: Self) -> Self {
        let (a, b) = (self.terms(), other.terms());
        Self::from_terms(std::array::from_fn(|i| a[i] + b[i]))
    }

    pub fn scale(self, k: f32) -> Self {
        let a = self.terms();
        Self::from_terms(std::array::from_fn(|i| a[i] * k))
    }

    pub fn has_percent(&self) -> bool {
        self.percent != 0.0
    }

    /// Resolves the font- and viewport-relative terms with `r`.
    pub fn resolve(&self, r: &Resolver) -> LengthPercentage {
        let mut px = self.px
            + self.em * r.em
            + self.rem * r.rem
            + self.ch * r.ch
            + self.vw * r.vw
            + self.vh * r.vh
            + self.vmin * r.vw.min(r.vh)
            + self.vmax * r.vw.max(r.vh);
        let mut percent = self.percent;
        if let Some(basis) = r.percent_basis {
            px += percent * basis / 100.0;
            percent = 0.0;
        }
        LengthPercentage {
            px: sane(px),
            percent: sane(percent),
        }
    }
}

/// Everything needed to turn relative units into px.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Resolver {
    /// px per `em`.
    pub em: f32,
    /// px per `rem`.
    pub rem: f32,
    /// px per `ch`.
    pub ch: f32,
    /// px per `vw` (1% of the viewport width).
    pub vw: f32,
    /// px per `vh`.
    pub vh: f32,
    /// When set, percentages are resolved immediately against this length
    /// (font-size, line-height, ...); otherwise they are kept.
    pub percent_basis: Option<f32>,
    /// Reference used for percentages inside `min()`/`max()`/`clamp()` that
    /// mix them with lengths (approximation: the viewport width).
    pub viewport_width: f32,
}

impl Resolver {
    /// A resolver for `font_size`-relative units.
    pub fn new(font_size: f32, root_font_size: f32, monospace: bool, viewport: (f32, f32)) -> Self {
        Self {
            em: font_size,
            rem: root_font_size,
            ch: font_size * if monospace { 0.6 } else { 0.5 },
            vw: viewport.0 / 100.0,
            vh: viewport.1 / 100.0,
            percent_basis: None,
            viewport_width: viewport.0,
        }
    }

    pub fn with_percent_basis(mut self, basis: f32) -> Self {
        self.percent_basis = Some(basis);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver() -> Resolver {
        Resolver::new(20.0, 16.0, false, (1000.0, 500.0))
    }

    #[test]
    fn units_resolve() {
        let r = resolver();
        let cases = [
            (LengthUnit::Px, 1.0),
            (LengthUnit::Em, 20.0),
            (LengthUnit::Ex, 10.0),
            (LengthUnit::Rem, 16.0),
            (LengthUnit::Ch, 10.0),
            (LengthUnit::Pt, 4.0 / 3.0),
            (LengthUnit::Pc, 16.0),
            (LengthUnit::In, 96.0),
            (LengthUnit::Vw, 10.0),
            (LengthUnit::Vh, 5.0),
            (LengthUnit::Vmin, 5.0),
            (LengthUnit::Vmax, 10.0),
        ];
        for (unit, px) in cases {
            let v = LinearLen::from_unit(1.0, unit).resolve(&r);
            assert!((v.px - px).abs() < 1e-4, "{unit:?}: {v:?}");
            assert_eq!(v.percent, 0.0);
        }
        let cm = LinearLen::from_unit(2.54, LengthUnit::Cm).resolve(&r);
        assert!((cm.px - 96.0).abs() < 1e-3);
        let mm = LinearLen::from_unit(25.4, LengthUnit::Mm).resolve(&r);
        assert!((mm.px - 96.0).abs() < 1e-3);
        let q = LinearLen::from_unit(101.6, LengthUnit::Q).resolve(&r);
        assert!((q.px - 96.0).abs() < 1e-3);
    }

    #[test]
    fn monospace_ch_and_percent_basis() {
        let r = Resolver::new(10.0, 16.0, true, (100.0, 100.0));
        assert_eq!(
            LinearLen::from_unit(1.0, LengthUnit::Ch).resolve(&r).px,
            6.0
        );
        let p = LinearLen::percent(50.0);
        assert_eq!(p.resolve(&r), LengthPercentage::percent(50.0));
        assert_eq!(
            p.resolve(&r.with_percent_basis(30.0)),
            LengthPercentage::px(15.0)
        );
    }

    #[test]
    fn arithmetic_and_sanitizing() {
        let a = LinearLen::px(10.0).add(LinearLen::percent(5.0)).scale(2.0);
        assert_eq!(a.px, 20.0);
        assert_eq!(a.percent, 10.0);
        assert_eq!(sane(f32::NAN), 0.0);
        assert_eq!(sane(f32::INFINITY), MAX_PX);
        let huge = LinearLen::px(f32::MAX).scale(10.0).resolve(&resolver());
        assert!(huge.px.is_finite());
    }
}
