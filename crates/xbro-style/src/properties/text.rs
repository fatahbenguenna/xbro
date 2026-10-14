//! Text properties: color, alignment, spacing, white space, decoration,
//! vertical alignment and table text properties.

use xbro_css::ComponentValue as CV;

use super::{only, push, LonghandId, Out, Spec, SpecTextAlign, SpecVerticalAlign, WsCollapse};
use crate::computed::*;
use crate::values::{
    color as color_value, keyword, length, single, ParseCtx, ANY_LP, NON_NEG_LENGTH,
};

pub(super) fn color(items: &[&CV], ids: &[LonghandId], _: &ParseCtx, out: &mut Out) -> Option<()> {
    push(out, only(ids)?, Spec::Color(color_value(single(items)?)?));
    Some(())
}

pub(super) fn text_align(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let v = single(items)?;
    let a = if v.is_ident("match-parent") || v.is_ident("-webkit-match-parent") {
        SpecTextAlign::MatchParent
    } else {
        SpecTextAlign::Align(keyword(
            v,
            &[
                ("start", TextAlign::Left),
                ("left", TextAlign::Left),
                ("-webkit-left", TextAlign::Left),
                ("-moz-left", TextAlign::Left),
                ("end", TextAlign::Right),
                ("right", TextAlign::Right),
                ("-webkit-right", TextAlign::Right),
                ("-moz-right", TextAlign::Right),
                ("center", TextAlign::Center),
                ("-webkit-center", TextAlign::Center),
                ("-moz-center", TextAlign::Center),
                ("-khtml-center", TextAlign::Center),
                ("-internal-center", TextAlign::Center),
                ("justify", TextAlign::Justify),
                ("justify-all", TextAlign::Justify),
            ],
        )?)
    };
    push(out, only(ids)?, Spec::TextAlign(a));
    Some(())
}

/// `text-indent: <length-percentage> && hanging? && each-line?` (keywords ignored).
pub(super) fn text_indent(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let mut lp = None;
    for v in items {
        if v.is_ident("hanging") || v.is_ident("each-line") {
            continue;
        }
        if lp.is_some() {
            return None;
        }
        lp = Some(length(v, ANY_LP, cx)?);
    }
    push(out, only(ids)?, Spec::Lp(lp?));
    Some(())
}

pub(super) fn text_transform(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let mut transform = None;
    for v in items {
        if v.is_ident("full-width") || v.is_ident("full-size-kana") || v.is_ident("math-auto") {
            continue;
        }
        if transform.is_some() {
            return None;
        }
        transform = Some(keyword(
            v,
            &[
                ("none", TextTransform::None),
                ("uppercase", TextTransform::Uppercase),
                ("lowercase", TextTransform::Lowercase),
                ("capitalize", TextTransform::Capitalize),
            ],
        )?);
    }
    push(
        out,
        only(ids)?,
        Spec::TextTransform(transform.unwrap_or_default()),
    );
    Some(())
}

/// `letter-spacing` / `word-spacing`: `normal` or a length (percentages of the font size).
pub(super) fn spacing(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let v = single(items)?;
    let s = if v.is_ident("normal") {
        None
    } else {
        Some(length(v, ANY_LP, cx)?)
    };
    push(out, only(ids)?, Spec::Spacing(s));
    Some(())
}

fn collapse_keyword(v: &CV) -> Option<WsCollapse> {
    keyword(
        v,
        &[
            ("collapse", WsCollapse::Collapse),
            ("discard", WsCollapse::Collapse),
            ("preserve", WsCollapse::Preserve),
            ("preserve-spaces", WsCollapse::Preserve),
            ("preserve-breaks", WsCollapse::PreserveBreaks),
            ("break-spaces", WsCollapse::BreakSpaces),
        ],
    )
}

/// `white-space`: CSS 2 keywords, or the CSS Text 4 shorthand
/// `<white-space-collapse> || <text-wrap-mode> || <white-space-trim>`.
pub(super) fn white_space(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let [collapse_id, wrap_id] = ids else {
        return None;
    };
    let legacy = match items {
        [v] => keyword(
            v,
            &[
                ("normal", (WsCollapse::Collapse, false)),
                ("pre", (WsCollapse::Preserve, true)),
                ("nowrap", (WsCollapse::Collapse, true)),
                ("pre-wrap", (WsCollapse::Preserve, false)),
                ("pre-line", (WsCollapse::PreserveBreaks, false)),
                ("break-spaces", (WsCollapse::BreakSpaces, false)),
                ("-webkit-nowrap", (WsCollapse::Collapse, true)),
                ("-moz-pre-wrap", (WsCollapse::Preserve, false)),
                ("-o-pre-wrap", (WsCollapse::Preserve, false)),
            ],
        ),
        _ => None,
    };
    let (collapse, nowrap) = match legacy {
        Some(pair) => pair,
        None => {
            let (mut collapse, mut nowrap) = (None, None);
            for v in items {
                if collapse.is_none() {
                    if let Some(c) = collapse_keyword(v) {
                        collapse = Some(c);
                        continue;
                    }
                }
                if nowrap.is_none() {
                    if let Some(w) = keyword(v, &[("wrap", false), ("nowrap", true)]) {
                        nowrap = Some(w);
                        continue;
                    }
                }
                // white-space-trim keywords are accepted and ignored.
                if ["discard-before", "discard-after", "discard-inner"]
                    .iter()
                    .any(|k| v.is_ident(k))
                {
                    continue;
                }
                return None;
            }
            (collapse.unwrap_or_default(), nowrap.unwrap_or(false))
        }
    };
    push(out, *collapse_id, Spec::WsCollapse(collapse));
    push(out, *wrap_id, Spec::Bool(nowrap));
    Some(())
}

pub(super) fn white_space_collapse(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(
        out,
        only(ids)?,
        Spec::WsCollapse(collapse_keyword(single(items)?)?),
    );
    Some(())
}

/// `text-wrap-mode` and the `text-wrap` shorthand (wrap style keywords are ignored).
pub(super) fn text_wrap(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let mut nowrap = None;
    for v in items {
        if let Some(w) = keyword(v, &[("wrap", false), ("nowrap", true)]) {
            if nowrap.replace(w).is_some() {
                return None;
            }
        } else if !["balance", "pretty", "stable", "auto"]
            .iter()
            .any(|k| v.is_ident(k))
        {
            return None;
        }
    }
    push(out, only(ids)?, Spec::Bool(nowrap.unwrap_or(false)));
    Some(())
}

pub(super) fn overflow_wrap(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let b = keyword(
        single(items)?,
        &[("normal", false), ("break-word", true), ("anywhere", true)],
    )?;
    push(out, only(ids)?, Spec::Bool(b));
    Some(())
}

pub(super) fn word_break(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let b = keyword(
        single(items)?,
        &[
            ("normal", false),
            ("keep-all", false),
            ("auto-phrase", false),
            ("manual", false),
            ("break-all", true),
            ("break-word", true),
        ],
    )?;
    push(out, only(ids)?, Spec::Bool(b));
    Some(())
}

/// Decoration line keywords seen so far (each may appear only once).
#[derive(Default)]
struct LineParser {
    line: TextDecorationLine,
    blink: bool,
}

impl LineParser {
    /// Adds a decoration line keyword; false if `v` is not one or repeats one.
    fn add(&mut self, v: &CV) -> bool {
        let Some(k) = v.ident() else {
            return false;
        };
        let flag = match k.to_ascii_lowercase().as_str() {
            "underline" => &mut self.line.underline,
            "overline" => &mut self.line.overline,
            "line-through" => &mut self.line.line_through,
            "blink" => &mut self.blink,
            _ => return false,
        };
        !std::mem::replace(flag, true)
    }
}

fn line_value(items: &[&CV]) -> Option<TextDecorationLine> {
    match items {
        [v] if v.is_ident("none") => return Some(TextDecorationLine::NONE),
        // `spelling-error` / `grammar-error` are only valid alone.
        [v] if v.is_ident("spelling-error") || v.is_ident("grammar-error") => {
            return Some(TextDecorationLine::NONE)
        }
        _ => {}
    }
    let mut parser = LineParser::default();
    for v in items {
        if !parser.add(v) {
            return None;
        }
    }
    Some(parser.line)
}

pub(super) fn text_decoration_line(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(out, only(ids)?, Spec::DecorationLine(line_value(items)?));
    Some(())
}

pub(super) fn text_decoration_color(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    push(out, only(ids)?, Spec::Color(color_value(single(items)?)?));
    Some(())
}

/// `text-decoration: <line> || <style> || <color> || <thickness>`.
pub(super) fn text_decoration(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let [line_id, color_id] = ids else {
        return None;
    };
    let mut lines = LineParser::default();
    let (mut has_line, mut none) = (false, false);
    let (mut style, mut col, mut thickness) = (false, None, false);
    for v in items {
        if v.is_ident("none") && !has_line && !none {
            none = true;
            continue;
        }
        if !none && lines.add(v) {
            has_line = true;
            continue;
        }
        if !style
            && ["solid", "double", "dotted", "dashed", "wavy"]
                .iter()
                .any(|k| v.is_ident(k))
        {
            style = true;
            continue;
        }
        if col.is_none() {
            if let Some(c) = color_value(v) {
                col = Some(c);
                continue;
            }
        }
        if !thickness
            && (v.is_ident("auto") || v.is_ident("from-font") || length(v, ANY_LP, cx).is_some())
        {
            thickness = true;
            continue;
        }
        return None;
    }
    push(out, *line_id, Spec::DecorationLine(lines.line));
    push(
        out,
        *color_id,
        Spec::Color(col.unwrap_or(crate::values::SpecColor::CurrentColor)),
    );
    Some(())
}

pub(super) fn vertical_align(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let v = single(items)?;
    let va = match keyword(
        v,
        &[
            ("baseline", VerticalAlign::Baseline),
            ("sub", VerticalAlign::Sub),
            ("super", VerticalAlign::Super),
            ("text-top", VerticalAlign::TextTop),
            ("text-bottom", VerticalAlign::TextBottom),
            ("middle", VerticalAlign::Middle),
            ("-webkit-baseline-middle", VerticalAlign::Middle),
            ("top", VerticalAlign::Top),
            ("bottom", VerticalAlign::Bottom),
        ],
    ) {
        Some(k) => SpecVerticalAlign::Keyword(k),
        None => SpecVerticalAlign::Length(length(v, ANY_LP, cx)?),
    };
    push(out, only(ids)?, Spec::VerticalAlign(va));
    Some(())
}

pub(super) fn border_collapse(
    items: &[&CV],
    ids: &[LonghandId],
    _: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let b = keyword(
        single(items)?,
        &[
            ("separate", BorderCollapse::Separate),
            ("collapse", BorderCollapse::Collapse),
        ],
    )?;
    push(out, only(ids)?, Spec::BorderCollapse(b));
    Some(())
}

pub(super) fn border_spacing(
    items: &[&CV],
    ids: &[LonghandId],
    cx: &ParseCtx,
    out: &mut Out,
) -> Option<()> {
    let (h, v) = match items {
        [a] => {
            let a = length(a, NON_NEG_LENGTH, cx)?;
            (a.clone(), a)
        }
        [a, b] => (
            length(a, NON_NEG_LENGTH, cx)?,
            length(b, NON_NEG_LENGTH, cx)?,
        ),
        _ => return None,
    };
    push(out, only(ids)?, Spec::BorderSpacing(h, v));
    Some(())
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{parse, spec};
    use super::*;
    use crate::values::{MathExpr, SpecColor};
    use xbro_css::Color;

    #[test]
    fn alignment_and_transform() {
        let ta = |css: &str| spec("text-align", css, LonghandId::TextAlign);
        assert_eq!(
            ta("start"),
            Some(Spec::TextAlign(SpecTextAlign::Align(TextAlign::Left)))
        );
        assert_eq!(
            ta("end"),
            Some(Spec::TextAlign(SpecTextAlign::Align(TextAlign::Right)))
        );
        assert_eq!(
            ta("-webkit-center"),
            Some(Spec::TextAlign(SpecTextAlign::Align(TextAlign::Center)))
        );
        assert_eq!(
            ta("match-parent"),
            Some(Spec::TextAlign(SpecTextAlign::MatchParent))
        );
        assert_eq!(ta("middle"), None);
        assert_eq!(
            spec(
                "text-transform",
                "uppercase full-width",
                LonghandId::TextTransform
            ),
            Some(Spec::TextTransform(TextTransform::Uppercase))
        );
        assert_eq!(
            spec("text-indent", "2em hanging", LonghandId::TextIndent),
            Some(Spec::Lp(MathExpr::Linear(crate::values::LinearLen {
                em: 2.0,
                ..crate::values::LinearLen::ZERO
            })))
        );
        assert_eq!(
            spec("letter-spacing", "normal", LonghandId::LetterSpacing),
            Some(Spec::Spacing(None))
        );
    }

    #[test]
    fn white_space_forms() {
        let ws = |css: &str| {
            (
                spec("white-space", css, LonghandId::WhiteSpaceCollapse),
                spec("white-space", css, LonghandId::TextWrapMode),
            )
        };
        assert_eq!(
            ws("pre"),
            (
                Some(Spec::WsCollapse(WsCollapse::Preserve)),
                Some(Spec::Bool(true))
            )
        );
        assert_eq!(
            ws("pre-line"),
            (
                Some(Spec::WsCollapse(WsCollapse::PreserveBreaks)),
                Some(Spec::Bool(false))
            )
        );
        assert_eq!(
            ws("preserve nowrap"),
            (
                Some(Spec::WsCollapse(WsCollapse::Preserve)),
                Some(Spec::Bool(true))
            )
        );
        assert_eq!(
            ws("nowrap"),
            (
                Some(Spec::WsCollapse(WsCollapse::Collapse)),
                Some(Spec::Bool(true))
            )
        );
        assert!(parse("white-space", "pre pre").is_none());
        assert_eq!(
            spec("text-wrap", "nowrap", LonghandId::TextWrapMode),
            Some(Spec::Bool(true))
        );
        assert_eq!(
            spec("text-wrap", "balance", LonghandId::TextWrapMode),
            Some(Spec::Bool(false))
        );
        assert_eq!(
            spec("word-wrap", "break-word", LonghandId::OverflowWrap),
            Some(Spec::Bool(true))
        );
        assert_eq!(
            spec("word-break", "keep-all", LonghandId::WordBreak),
            Some(Spec::Bool(false))
        );
        assert_eq!(
            spec("word-break", "break-all", LonghandId::WordBreak),
            Some(Spec::Bool(true))
        );
    }

    #[test]
    fn decorations() {
        let line = |css: &str| spec("text-decoration", css, LonghandId::TextDecorationLine);
        assert_eq!(
            line("underline dotted red"),
            Some(Spec::DecorationLine(TextDecorationLine {
                underline: true,
                ..TextDecorationLine::NONE
            }))
        );
        assert_eq!(
            spec(
                "text-decoration",
                "underline dotted red",
                LonghandId::TextDecorationColor
            ),
            Some(Spec::Color(SpecColor::Color(Color::rgb(255, 0, 0))))
        );
        assert_eq!(
            line("none"),
            Some(Spec::DecorationLine(TextDecorationLine::NONE))
        );
        assert_eq!(
            line("overline line-through 2px"),
            Some(Spec::DecorationLine(TextDecorationLine {
                overline: true,
                line_through: true,
                underline: false
            }))
        );
        assert!(parse("text-decoration", "none underline").is_none());
        assert!(parse("text-decoration", "red blue").is_none());
        assert!(parse("text-decoration-line", "none underline").is_none());
        // Each line keyword at most once.
        assert!(parse("text-decoration-line", "underline underline").is_none());
        assert!(parse("text-decoration", "underline red underline").is_none());
        assert!(parse("text-decoration-line", "blink blink").is_none());
        assert_eq!(
            spec(
                "text-decoration-line",
                "blink underline",
                LonghandId::TextDecorationLine
            ),
            Some(Spec::DecorationLine(TextDecorationLine {
                underline: true,
                ..TextDecorationLine::NONE
            }))
        );
        // `spelling-error` / `grammar-error` only stand alone.
        assert_eq!(
            spec(
                "text-decoration-line",
                "spelling-error",
                LonghandId::TextDecorationLine
            ),
            Some(Spec::DecorationLine(TextDecorationLine::NONE))
        );
        assert!(parse("text-decoration-line", "underline grammar-error").is_none());
        assert_eq!(
            spec("text-align", "-webkit-match-parent", LonghandId::TextAlign),
            Some(Spec::TextAlign(SpecTextAlign::MatchParent))
        );
    }

    #[test]
    fn vertical_align_and_tables() {
        assert_eq!(
            spec("vertical-align", "middle", LonghandId::VerticalAlign),
            Some(Spec::VerticalAlign(SpecVerticalAlign::Keyword(
                VerticalAlign::Middle
            )))
        );
        assert_eq!(
            spec("vertical-align", "-2px", LonghandId::VerticalAlign),
            Some(Spec::VerticalAlign(SpecVerticalAlign::Length(
                MathExpr::px(-2.0)
            )))
        );
        assert_eq!(
            spec("border-spacing", "2px 4px", LonghandId::BorderSpacing),
            Some(Spec::BorderSpacing(MathExpr::px(2.0), MathExpr::px(4.0)))
        );
        assert!(parse("border-spacing", "10%").is_none());
        assert!(parse("border-spacing", "1px 2px 3px").is_none());
    }
}
