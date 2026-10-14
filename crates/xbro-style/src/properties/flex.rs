//! Flexbox, box alignment and gaps.

use xbro_css::{ComponentValue as CV, Token};

use super::{only, per_item, push, LonghandId, Out, Spec, SpecLpa};
use crate::computed::{AlignItems, FlexDirection, FlexWrap, JustifyContent};
use crate::values::{integer, keyword, length, number, single, ParseCtx, NON_NEG_LP};

fn direction(v: &CV) -> Option<FlexDirection> {
    keyword(
        v,
        &[
            ("row", FlexDirection::Row),
            ("row-reverse", FlexDirection::RowReverse),
            ("column", FlexDirection::Column),
            ("column-reverse", FlexDirection::ColumnReverse),
        ],
    )
}

fn wrap(v: &CV) -> Option<FlexWrap> {
    keyword(
        v,
        &[
            ("nowrap", FlexWrap::NoWrap),
            ("wrap", FlexWrap::Wrap),
            ("wrap-reverse", FlexWrap::WrapReverse),
        ],
    )
}

pub(super) fn flex_direction(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(
        out,
        only(ids)?,
        Spec::FlexDirection(direction(single(items)?)?),
    );
    Some(())
}

pub(super) fn flex_wrap(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(out, only(ids)?, Spec::FlexWrap(wrap(single(items)?)?));
    Some(())
}

pub(super) fn flex_flow(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let [dir_id, wrap_id] = ids else {
        return None;
    };
    let (mut d, mut w) = (None, None);
    for v in items {
        if d.is_none() {
            if let Some(x) = direction(v) {
                d = Some(x);
                continue;
            }
        }
        if w.is_none() {
            if let Some(x) = wrap(v) {
                w = Some(x);
                continue;
            }
        }
        return None;
    }
    if items.is_empty() {
        return None;
    }
    push(out, *dir_id, Spec::FlexDirection(d.unwrap_or_default()));
    push(out, *wrap_id, Spec::FlexWrap(w.unwrap_or_default()));
    Some(())
}

/// A non-negative flex factor.
fn factor(v: &CV) -> Option<f32> {
    number(v).filter(|n| *n >= 0.0)
}

pub(super) fn flex_factor(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(out, only(ids)?, Spec::Number(factor(single(items)?)?));
    Some(())
}

/// `flex-basis`: `auto | content | <length-percentage>` (intrinsic keywords ≈ auto).
fn basis(v: &CV, cx: &ParseCtx) -> Option<SpecLpa> {
    if [
        "auto",
        "content",
        "min-content",
        "max-content",
        "fit-content",
    ]
    .iter()
    .any(|k| v.is_ident(k))
    {
        return Some(SpecLpa::Auto);
    }
    length(v, NON_NEG_LP, cx).map(SpecLpa::Lp)
}

pub(super) fn flex_basis(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(out, only(ids)?, Spec::Lpa(basis(single(items)?, cx)?));
    Some(())
}

/// `flex: none | [<grow> <shrink>?] || <basis>`.
pub(super) fn flex(items: &[&CV], ids: &[LonghandId], cx: &ParseCtx, out: &mut Out) -> Option<()> {
    let [grow_id, shrink_id, basis_id] = ids else {
        return None;
    };
    let (grow, shrink, b) = match items {
        [v] if v.is_ident("none") => (0.0, 0.0, SpecLpa::Auto),
        [v] if v.is_ident("auto") => (1.0, 1.0, SpecLpa::Auto),
        _ => {
            let (mut grow, mut shrink, mut b) = (None, None, None);
            let mut prev_was_grow = false;
            for v in items {
                let is_number = matches!(v, CV::Token(Token::Number { .. })) || number(v).is_some();
                if is_number && grow.is_none() {
                    grow = Some(factor(v)?);
                    prev_was_grow = true;
                    continue;
                }
                if is_number && shrink.is_none() && prev_was_grow {
                    shrink = Some(factor(v)?);
                    prev_was_grow = false;
                    continue;
                }
                prev_was_grow = false;
                if b.is_none() {
                    // A unitless zero after both factors is the basis.
                    if let Some(x) = basis(v, cx) {
                        b = Some(x);
                        continue;
                    }
                }
                return None;
            }
            if grow.is_none() && b.is_none() {
                return None;
            }
            (
                grow.unwrap_or(1.0),
                shrink.unwrap_or(1.0),
                // `flex: <grow>` implies a 0% basis.
                b.unwrap_or(SpecLpa::Lp(crate::values::MathExpr::percent(0.0))),
            )
        }
    };
    push(out, *grow_id, Spec::Number(grow));
    push(out, *shrink_id, Spec::Number(shrink));
    push(out, *basis_id, Spec::Lpa(b));
    Some(())
}

pub(super) fn order(items: &[&CV], ids: &[LonghandId], _: &ParseCtx, out: &mut Out) -> Option<()> {
    push(out, only(ids)?, Spec::Integer(integer(single(items)?)?));
    Some(())
}

/// Strips `safe` / `unsafe` / `first` / `last` prefixes; returns the keyword
/// (lowercased) and whether `last` was given.
fn alignment_keyword(items: &[&CV]) -> Option<String> {
    let words = items
        .iter()
        .map(|v| v.ident().map(str::to_ascii_lowercase))
        .collect::<Option<Vec<_>>>()?;
    match words.as_slice() {
        [k] => Some(k.clone()),
        [prefix, k] if (prefix == "safe" || prefix == "unsafe") && k != "safe" && k != "unsafe" => {
            Some(k.clone())
        }
        [prefix, k] if (prefix == "first" || prefix == "last") && k == "baseline" => {
            Some("baseline".into())
        }
        _ => None,
    }
}

fn content_distribution(k: &str, for_align: bool) -> Option<JustifyContent> {
    Some(match k {
        "flex-start" | "start" | "left" | "self-start" => JustifyContent::FlexStart,
        "flex-end" | "end" | "right" | "self-end" => JustifyContent::FlexEnd,
        "center" => JustifyContent::Center,
        "space-between" => JustifyContent::SpaceBetween,
        "space-around" => JustifyContent::SpaceAround,
        "space-evenly" => JustifyContent::SpaceEvenly,
        "normal" | "stretch" if for_align => JustifyContent::Stretch,
        "normal" | "stretch" => JustifyContent::FlexStart,
        "baseline" => JustifyContent::FlexStart,
        _ => return None,
    })
}

fn item_alignment(k: &str, allow_auto: bool) -> Option<AlignItems> {
    Some(match k {
        "auto" if allow_auto => AlignItems::Auto,
        "flex-start" | "start" | "self-start" => AlignItems::FlexStart,
        "flex-end" | "end" | "self-end" => AlignItems::FlexEnd,
        "center" => AlignItems::Center,
        "baseline" => AlignItems::Baseline,
        "normal" | "stretch" => AlignItems::Stretch,
        _ => return None,
    })
}

pub(super) fn justify_content(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let k = alignment_keyword(items)?;
    push(
        out,
        only(ids)?,
        Spec::Justify(content_distribution(&k, false)?),
    );
    Some(())
}

pub(super) fn align_content(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let k = alignment_keyword(items)?;
    if k == "left" || k == "right" {
        return None;
    }
    push(
        out,
        only(ids)?,
        Spec::Justify(content_distribution(&k, true)?),
    );
    Some(())
}

pub(super) fn align_items(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let k = alignment_keyword(items)?;
    push(out, only(ids)?, Spec::Align(item_alignment(&k, false)?));
    Some(())
}

pub(super) fn align_self(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let k = alignment_keyword(items)?;
    push(out, only(ids)?, Spec::Align(item_alignment(&k, true)?));
    Some(())
}

/// Splits `<a> <b>?` where each part is one keyword optionally prefixed
/// (`safe center`, `first baseline`).
fn split_pair<'a, 'b>(items: &'b [&'a CV]) -> Option<(&'b [&'a CV], &'b [&'a CV])> {
    let is_prefix = |v: &CV| {
        ["safe", "unsafe", "first", "last"]
            .iter()
            .any(|k| v.is_ident(k))
    };
    let first_len = if items.first().is_some_and(|v| is_prefix(v)) {
        2
    } else {
        1
    };
    let first = items.get(..first_len)?;
    let rest = items.get(first_len..)?;
    Some((first, rest))
}

pub(super) fn place_content(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let [align_id, justify_id] = ids else {
        return None;
    };
    let (a, j) = split_pair(items)?;
    let ak = alignment_keyword(a)?;
    let align = content_distribution(&ak, true)?;
    let justify = if j.is_empty() {
        if ak == "baseline" {
            JustifyContent::FlexStart
        } else {
            content_distribution(&ak, false)?
        }
    } else {
        content_distribution(&alignment_keyword(j)?, false)?
    };
    push(out, *align_id, Spec::Justify(align));
    push(out, *justify_id, Spec::Justify(justify));
    Some(())
}

fn place_items_like(items: &[&CV], allow_auto: bool) -> Option<AlignItems> {
    let (a, j) = split_pair(items)?;
    let align = item_alignment(&alignment_keyword(a)?, allow_auto)?;
    if !j.is_empty() {
        // justify-items / justify-self: validated loosely, then ignored.
        let k = alignment_keyword(j).or_else(|| {
            // `legacy left` etc.
            (j.len() == 2 && j.iter().any(|v| v.is_ident("legacy"))).then(|| "legacy".to_string())
        })?;
        let known = [
            "auto",
            "normal",
            "stretch",
            "baseline",
            "start",
            "end",
            "self-start",
            "self-end",
            "flex-start",
            "flex-end",
            "center",
            "left",
            "right",
            "legacy",
        ];
        if !known.contains(&k.as_str()) {
            return None;
        }
    }
    Some(align)
}

pub(super) fn place_items(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(
        out,
        only(ids)?,
        Spec::Align(place_items_like(items, false)?),
    );
    Some(())
}

pub(super) fn place_self(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(out, only(ids)?, Spec::Align(place_items_like(items, true)?));
    Some(())
}

/// `gap` (row then column), `row-gap`, `column-gap` and the `grid-` aliases.
pub(super) fn gap(items: &[&CV], ids: &[LonghandId], cx: &ParseCtx, out: &mut Out) -> Option<()> {
    per_item(items, ids, out, |v| {
        if v.is_ident("normal") {
            Some(Spec::Lp(crate::values::MathExpr::ZERO))
        } else {
            length(v, NON_NEG_LP, cx).map(Spec::Lp)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, spec};
    use super::*;
    use crate::values::MathExpr;

    fn flex_of(css: &str) -> Option<(Spec, Spec, Spec)> {
        Some((
            spec("flex", css, LonghandId::FlexGrow)?,
            spec("flex", css, LonghandId::FlexShrink)?,
            spec("flex", css, LonghandId::FlexBasis)?,
        ))
    }

    fn triple(g: f32, s: f32, b: SpecLpa) -> Option<(Spec, Spec, Spec)> {
        Some((Spec::Number(g), Spec::Number(s), Spec::Lpa(b)))
    }

    #[test]
    fn flex_shorthand() {
        let pct0 = SpecLpa::Lp(MathExpr::percent(0.0));
        assert_eq!(flex_of("1"), triple(1.0, 1.0, pct0.clone()));
        assert_eq!(flex_of("auto"), triple(1.0, 1.0, SpecLpa::Auto));
        assert_eq!(flex_of("none"), triple(0.0, 0.0, SpecLpa::Auto));
        assert_eq!(
            flex_of("2 2 10%"),
            triple(2.0, 2.0, SpecLpa::Lp(MathExpr::percent(10.0)))
        );
        assert_eq!(
            flex_of("30px"),
            triple(1.0, 1.0, SpecLpa::Lp(MathExpr::px(30.0)))
        );
        assert_eq!(
            flex_of("1 30px"),
            triple(1.0, 1.0, SpecLpa::Lp(MathExpr::px(30.0)))
        );
        assert_eq!(flex_of("2 3"), triple(2.0, 3.0, pct0.clone()));
        assert_eq!(
            flex_of("0 0 0"),
            triple(0.0, 0.0, SpecLpa::Lp(MathExpr::ZERO))
        );
        assert_eq!(
            flex_of("10px 2"),
            triple(2.0, 1.0, SpecLpa::Lp(MathExpr::px(10.0)))
        );
        assert_eq!(flex_of("1 1 auto"), triple(1.0, 1.0, SpecLpa::Auto));
        // `initial` is a CSS-wide keyword handled generically.
        assert!(matches!(
            parse("flex", "initial").unwrap()[0],
            super::super::PropertyDeclaration::Wide(..)
        ));
        assert!(parse("flex", "-1").is_none());
        assert!(parse("flex", "1 2 3").is_none());
        assert!(parse("flex", "auto auto").is_none());
    }

    #[test]
    fn alignment() {
        let jc = |css: &str| spec("justify-content", css, LonghandId::JustifyContent);
        assert_eq!(
            jc("space-between"),
            Some(Spec::Justify(JustifyContent::SpaceBetween))
        );
        assert_eq!(
            jc("safe center"),
            Some(Spec::Justify(JustifyContent::Center))
        );
        assert_eq!(jc("right"), Some(Spec::Justify(JustifyContent::FlexEnd)));
        assert_eq!(jc("normal"), Some(Spec::Justify(JustifyContent::FlexStart)));
        let ac = |css: &str| spec("align-content", css, LonghandId::AlignContent);
        assert_eq!(ac("normal"), Some(Spec::Justify(JustifyContent::Stretch)));
        assert_eq!(ac("end"), Some(Spec::Justify(JustifyContent::FlexEnd)));
        let ai = |css: &str| spec("align-items", css, LonghandId::AlignItems);
        assert_eq!(
            ai("first baseline"),
            Some(Spec::Align(AlignItems::Baseline))
        );
        assert_eq!(ai("self-end"), Some(Spec::Align(AlignItems::FlexEnd)));
        assert_eq!(ai("normal"), Some(Spec::Align(AlignItems::Stretch)));
        assert_eq!(ai("auto"), None);
        assert_eq!(
            spec("align-self", "auto", LonghandId::AlignSelf),
            Some(Spec::Align(AlignItems::Auto))
        );
        assert_eq!(
            spec(
                "place-content",
                "center space-between",
                LonghandId::JustifyContent
            ),
            Some(Spec::Justify(JustifyContent::SpaceBetween))
        );
        assert_eq!(
            spec("place-content", "center", LonghandId::JustifyContent),
            Some(Spec::Justify(JustifyContent::Center))
        );
        assert_eq!(
            spec("place-items", "center start", LonghandId::AlignItems),
            Some(Spec::Align(AlignItems::Center))
        );
        assert_eq!(
            spec("place-self", "end", LonghandId::AlignSelf),
            Some(Spec::Align(AlignItems::FlexEnd))
        );
        assert!(parse("place-items", "center bogus").is_none());
    }

    #[test]
    fn gaps_and_order() {
        assert_eq!(
            spec("gap", "10px 5%", LonghandId::ColumnGap),
            Some(Spec::Lp(MathExpr::percent(5.0)))
        );
        assert_eq!(
            spec("gap", "normal", LonghandId::RowGap),
            Some(Spec::Lp(MathExpr::ZERO))
        );
        assert_eq!(
            spec("grid-row-gap", "1px", LonghandId::RowGap),
            Some(Spec::Lp(MathExpr::px(1.0)))
        );
        assert_eq!(
            spec("order", "-2", LonghandId::Order),
            Some(Spec::Integer(-2))
        );
        assert!(parse("order", "1.5").is_none());
        assert_eq!(
            spec("flex-flow", "column wrap", LonghandId::FlexWrap),
            Some(Spec::FlexWrap(FlexWrap::Wrap))
        );
    }
}
