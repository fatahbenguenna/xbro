//! Specified values: the parsed form of a declaration, computed per element.

use std::sync::Arc;

use crate::computed::*;
use crate::values::{MathExpr, SpecColor};

/// `auto` or a length/percentage.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpecLpa {
    Auto,
    Lp(MathExpr),
}

/// `none` or a length/percentage (`max-width`, `max-height`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpecMax {
    None,
    Lp(MathExpr),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpecFontSize {
    /// Absolute keyword (`medium` = 16px, ...), already in px.
    Px(f32),
    Smaller,
    Larger,
    /// Length or percentage; `em` and `%` are relative to the parent's font size.
    Length(MathExpr),
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum SpecFontWeight {
    /// 1..=1000.
    Absolute(f32),
    Bolder,
    Lighter,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpecLineHeight {
    Normal,
    Number(f32),
    /// Length or percentage of the element's own font size, computed to px.
    Length(MathExpr),
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum SpecTextAlign {
    Align(TextAlign),
    /// `match-parent` behaves as `inherit` (xbro assumes left-to-right).
    MatchParent,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpecVerticalAlign {
    Keyword(VerticalAlign),
    /// Length, or percentage of the element's line height.
    Length(MathExpr),
}

/// `white-space-collapse` values.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub(crate) enum WsCollapse {
    #[default]
    Collapse,
    Preserve,
    PreserveBreaks,
    BreakSpaces,
}

/// `word-break` values (`break-word` is kept apart: it also affects
/// the computed `overflow_wrap`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub(crate) enum SpecWordBreak {
    #[default]
    Normal,
    BreakAll,
    KeepAll,
    BreakWord,
}

/// A specified grid track size (lengths not yet computed).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpecTrack {
    Length(MathExpr),
    Fr(f32),
    Auto,
    MinContent,
    MaxContent,
    FitContent(MathExpr),
    MinMax(Box<SpecTrack>, Box<SpecTrack>),
}

/// A specified `grid-template-columns` / `-rows` track list (see
/// [`GridTemplate`] for the layout of the fields).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpecGridTemplate {
    pub tracks: Vec<SpecTrack>,
    /// `(auto-fill?, tracks)`.
    pub auto_repeat: Option<(bool, Vec<SpecTrack>)>,
    pub auto_repeat_index: usize,
    /// The computed template when no length depends on the element (fonts,
    /// viewport): computed once at parse time and shared by every element.
    pub computed: Option<Arc<GridTemplate>>,
}

/// One background image layer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpecImage {
    Url(String),
    LinearGradient {
        angle_deg: f32,
        /// Colors with optional positions (fractions of the gradient line).
        stops: Vec<(SpecColor, Option<f32>)>,
    },
    /// Unsupported image (radial/conic gradient): approximated by painting
    /// this color as the background color when the latter is transparent.
    Approx(SpecColor),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpecBgSize {
    Auto,
    Cover,
    Contain,
    Explicit(Option<MathExpr>, Option<MathExpr>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpecContentItem {
    Text(String),
    /// `attr(name)`, with the fallback used when the attribute is missing.
    Attr {
        name: String,
        fallback: String,
    },
    Image(String),
    OpenQuote,
    CloseQuote,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpecContent {
    Normal,
    Items(Vec<SpecContentItem>),
}

/// A specified value. Each longhand accepts exactly one variant.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Spec {
    Display(Display),
    Position(Position),
    Lpa(SpecLpa),
    Lp(MathExpr),
    Max(SpecMax),
    /// Pure length (no percentage), computed to px: border/outline widths.
    Length(MathExpr),
    ZIndex(Option<i32>),
    Float(Float),
    Clear(Clear),
    BoxSizing(BoxSizing),
    BorderStyle(BorderStyle),
    Color(SpecColor),
    Overflow(Overflow),
    AspectRatio(Option<f32>),
    Visibility(Visibility),
    Number(f32),
    BgImage(Option<SpecImage>),
    BgRepeat(BackgroundRepeat),
    BgSize(SpecBgSize),
    Cursor(Cursor),
    FontFamily(Arc<Vec<String>>),
    FontSize(SpecFontSize),
    FontWeight(SpecFontWeight),
    FontStyle(FontStyle),
    LineHeight(SpecLineHeight),
    TextAlign(SpecTextAlign),
    TextTransform(TextTransform),
    /// `letter-spacing` / `word-spacing`: `None` = `normal`.
    Spacing(Option<MathExpr>),
    WsCollapse(WsCollapse),
    /// `text-wrap-mode: nowrap`.
    Bool(bool),
    OverflowWrap(OverflowWrap),
    WordBreak(SpecWordBreak),
    ListStyleType(ListStyleType),
    ListStylePosition(ListStylePosition),
    ListStyleImage(Option<std::sync::Arc<str>>),
    BorderCollapse(BorderCollapse),
    BorderSpacing(MathExpr, MathExpr),
    CaptionSide(CaptionSide),
    TableLayout(TableLayout),
    DecorationLine(TextDecorationLine),
    VerticalAlign(SpecVerticalAlign),
    FlexDirection(FlexDirection),
    FlexWrap(FlexWrap),
    Justify(JustifyContent),
    Align(AlignItems),
    Integer(i32),
    /// `grid-template-columns` / `-rows`; `None` = `none`.
    GridTemplate(Option<Arc<SpecGridTemplate>>),
    GridLine(GridLine),
    Content(SpecContent),
}
