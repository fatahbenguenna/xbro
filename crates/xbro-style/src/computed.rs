//! Computed style values consumed by layout and paint.
//!
//! All absolute lengths are resolved to CSS px at cascade time (`em`, `rem`,
//! `pt`, `vw`, …). Percentages, which depend on the containing block, are kept
//! as [`LengthPercentage`] and resolved by layout. `currentcolor` is resolved
//! to a concrete [`Color`].

use std::sync::{Arc, OnceLock};

pub use xbro_css::Color;

/// `px + percent% of the reference length`. Represents plain lengths
/// (`percent == 0`), plain percentages (`px == 0`) and `calc()` mixes of both.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub struct LengthPercentage {
    pub px: f32,
    pub percent: f32,
}

impl LengthPercentage {
    pub const ZERO: Self = Self {
        px: 0.0,
        percent: 0.0,
    };

    pub const fn px(px: f32) -> Self {
        Self { px, percent: 0.0 }
    }

    pub const fn percent(percent: f32) -> Self {
        Self { px: 0.0, percent }
    }

    /// Resolves against the reference length (usually the containing block width).
    pub fn resolve(&self, reference: f32) -> f32 {
        self.px + self.percent * reference / 100.0
    }

    pub fn has_percent(&self) -> bool {
        self.percent != 0.0
    }

    pub fn is_zero(&self) -> bool {
        self.px == 0.0 && self.percent == 0.0
    }
}

/// A length/percentage that may be `auto` (widths, heights, margins, insets, flex-basis).
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub enum LengthPercentageAuto {
    #[default]
    Auto,
    Length(LengthPercentage),
}

impl LengthPercentageAuto {
    pub const ZERO: Self = Self::Length(LengthPercentage::ZERO);

    pub const fn px(px: f32) -> Self {
        Self::Length(LengthPercentage::px(px))
    }

    pub const fn percent(p: f32) -> Self {
        Self::Length(LengthPercentage::percent(p))
    }

    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }

    /// `None` for `auto`, otherwise the resolved px value.
    pub fn resolve(&self, reference: f32) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Length(lp) => Some(lp.resolve(reference)),
        }
    }

    /// Like [`resolve`](Self::resolve), but percentages against an unknown
    /// (indefinite) reference behave as `auto`, per CSS 2.1 §10.5.
    pub fn resolve_definite(&self, reference: Option<f32>) -> Option<f32> {
        match (self, reference) {
            (Self::Auto, _) => None,
            (Self::Length(lp), Some(r)) => Some(lp.resolve(r)),
            (Self::Length(lp), None) if !lp.has_percent() => Some(lp.px),
            (Self::Length(_), None) => None,
        }
    }
}

/// `max-width` / `max-height`: `none` or a length/percentage.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub enum MaxSize {
    #[default]
    None,
    Length(LengthPercentage),
}

impl MaxSize {
    pub fn resolve(&self, reference: f32) -> Option<f32> {
        match self {
            Self::None => None,
            Self::Length(lp) => Some(lp.resolve(reference)),
        }
    }
}

/// Per-side values in CSS order of the physical sides.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub struct Sides<T> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

impl<T: Copy> Sides<T> {
    pub const fn all(v: T) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Display {
    None,
    Block,
    #[default]
    Inline,
    InlineBlock,
    ListItem,
    Flex,
    InlineFlex,
    /// `grid` / `inline-grid`: CSS grid with track sizing (fixed, `fr`, intrinsic,
    /// `minmax()`, auto-fill/auto-fit) and line/span placement; areas and named
    /// lines are not supported.
    Grid,
    InlineGrid,
    Table,
    InlineTable,
    TableRowGroup,
    TableHeaderGroup,
    TableFooterGroup,
    TableRow,
    TableCell,
    TableColumn,
    TableColumnGroup,
    TableCaption,
    /// The element generates no box; its children are laid out as if they were the element's siblings.
    Contents,
}

impl Display {
    /// Outer display type is block-level (participates in a block formatting context as a block).
    pub fn is_block_level(self) -> bool {
        matches!(
            self,
            Display::Block | Display::ListItem | Display::Flex | Display::Grid | Display::Table
        )
    }

    /// Atomic inline-level boxes (laid out as a block, placed on a line as a unit).
    pub fn is_inline_block_like(self) -> bool {
        matches!(
            self,
            Display::InlineBlock | Display::InlineFlex | Display::InlineGrid | Display::InlineTable
        )
    }

    pub fn is_table_part(self) -> bool {
        matches!(
            self,
            Display::TableRowGroup
                | Display::TableHeaderGroup
                | Display::TableFooterGroup
                | Display::TableRow
                | Display::TableCell
                | Display::TableColumn
                | Display::TableColumnGroup
                | Display::TableCaption
        )
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Position {
    #[default]
    Static,
    Relative,
    Absolute,
    Fixed,
    /// Treated as `relative` with no offsets by xbro layout.
    Sticky,
}

impl Position {
    /// Absolutely positioned (taken out of flow).
    pub fn is_out_of_flow(self) -> bool {
        matches!(self, Position::Absolute | Position::Fixed)
    }

    /// Establishes a containing block for absolutely positioned descendants.
    pub fn is_positioned(self) -> bool {
        !matches!(self, Position::Static)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Float {
    #[default]
    None,
    Left,
    Right,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Clear {
    #[default]
    None,
    Left,
    Right,
    Both,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Overflow {
    #[default]
    Visible,
    Hidden,
    /// `clip`
    Clip,
    /// `scroll` / `auto`: xbro clips but does not provide inner scrolling.
    Scroll,
    Auto,
}

impl Overflow {
    pub fn clips(self) -> bool {
        !matches!(self, Overflow::Visible)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Visible,
    Hidden,
    Collapse,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    None,
    Hidden,
    Solid,
    Dotted,
    Dashed,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

impl BorderStyle {
    /// `none` and `hidden` force the used border width to 0.
    pub fn is_visible(self) -> bool {
        !matches!(self, BorderStyle::None | BorderStyle::Hidden)
    }
}

/// One side of a border. `width` is already 0 when `style` is `none`/`hidden`.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub struct BorderSide {
    pub width: f32,
    pub style: BorderStyle,
    pub color: Color,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub enum LineHeight {
    /// Font-dependent (ascent + descent + line gap of the primary font, ~1.2em).
    #[default]
    Normal,
    /// Unitless multiplier of the element's own font-size (inherited as the number).
    Number(f32),
    /// Absolute line height in px (lengths and percentages are computed to px).
    Px(f32),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum TextAlign {
    /// `start` (xbro assumes left-to-right) and `left`.
    #[default]
    Left,
    Right,
    Center,
    Justify,
}

/// Decoration lines. Also used for the decorations "in effect" on a box,
/// which accumulate from ancestors (CSS text-decoration propagation).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct TextDecorationLine {
    pub underline: bool,
    pub overline: bool,
    pub line_through: bool,
}

impl TextDecorationLine {
    pub const NONE: Self = Self {
        underline: false,
        overline: false,
        line_through: false,
    };

    pub fn is_none(&self) -> bool {
        !(self.underline || self.overline || self.line_through)
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            underline: self.underline || other.underline,
            overline: self.overline || other.overline,
            line_through: self.line_through || other.line_through,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum WhiteSpace {
    #[default]
    Normal,
    Pre,
    Nowrap,
    PreWrap,
    PreLine,
    /// `break-spaces` is treated like `pre-wrap`.
    BreakSpaces,
}

impl WhiteSpace {
    /// Sequences of spaces/tabs are preserved.
    pub fn preserves_spaces(self) -> bool {
        matches!(
            self,
            WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
        )
    }

    /// Segment breaks (newlines) are preserved as forced line breaks.
    pub fn preserves_newlines(self) -> bool {
        matches!(
            self,
            WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine | WhiteSpace::BreakSpaces
        )
    }

    /// Lines may wrap at soft wrap opportunities.
    pub fn wraps(self) -> bool {
        !matches!(self, WhiteSpace::Pre | WhiteSpace::Nowrap)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub enum VerticalAlign {
    #[default]
    Baseline,
    Sub,
    Super,
    TextTop,
    TextBottom,
    Middle,
    Top,
    Bottom,
    /// Raise (positive) or lower the baseline by this many px (percentages resolved against line-height).
    Length(f32),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum ListStyleType {
    #[default]
    Disc,
    Circle,
    Square,
    Decimal,
    DecimalLeadingZero,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
    LowerGreek,
    None,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum ListStylePosition {
    #[default]
    Outside,
    Inside,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Auto,
    Default,
    Pointer,
    Text,
    Wait,
    Progress,
    Help,
    Move,
    NotAllowed,
    Crosshair,
    Grab,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum FlexDirection {
    #[default]
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

impl FlexDirection {
    pub fn is_row(self) -> bool {
        matches!(self, FlexDirection::Row | FlexDirection::RowReverse)
    }

    pub fn is_reverse(self) -> bool {
        matches!(
            self,
            FlexDirection::RowReverse | FlexDirection::ColumnReverse
        )
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum FlexWrap {
    #[default]
    NoWrap,
    Wrap,
    WrapReverse,
}

/// `justify-content` (and `align-content`) values.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum JustifyContent {
    #[default]
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
    /// Only meaningful for `align-content` (its initial value is `normal`, which behaves as stretch).
    Stretch,
}

/// `align-items` / `align-self` values. `Auto` is only valid for `align-self`
/// and means "use the parent's `align-items`".
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum AlignItems {
    Auto,
    FlexStart,
    FlexEnd,
    Center,
    Baseline,
    /// Initial value of `align-items` (`normal` behaves as stretch in flex layout).
    #[default]
    Stretch,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum BorderCollapse {
    #[default]
    Separate,
    Collapse,
}

/// `table-layout` (not inherited).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum TableLayout {
    #[default]
    Auto,
    /// Column widths come from the first row / `<col>` widths (CSS 2.1 §17.5.2.1).
    Fixed,
}

/// `caption-side` (inherited). `block-start` / `block-end` map to
/// `top` / `bottom` (horizontal writing mode).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum CaptionSide {
    #[default]
    Top,
    Bottom,
}

/// `overflow-wrap` / `word-wrap` (inherited), CSS Text 3 §5.5.
///
/// Both non-normal values allow breaking an otherwise unbreakable word
/// when it would overflow the line, but only `anywhere` lets those break
/// opportunities reduce the min-content size; `break-word` does not.
/// `word-break: break-word` computes to `BreakWord` here (see [`WordBreak`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum OverflowWrap {
    #[default]
    Normal,
    BreakWord,
    Anywhere,
}

impl OverflowWrap {
    /// May an unbreakable word be broken when it would overflow the line?
    pub fn breaks_overflowing_words(self) -> bool {
        !matches!(self, OverflowWrap::Normal)
    }

    /// Do the extra break opportunities count for the min-content size?
    pub fn reduces_min_content(self) -> bool {
        matches!(self, OverflowWrap::Anywhere)
    }
}

/// `word-break` (inherited), CSS Text 3 §5.2. The deprecated `break-word`
/// keyword computes to `Normal` here and sets
/// [`ComputedStyle::overflow_wrap`] to at least [`OverflowWrap::BreakWord`]
/// (approximation: the spec makes it behave as `overflow-wrap: anywhere`,
/// xbro keeps it from reducing min-content sizes). `auto-phrase` is `Normal`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum WordBreak {
    #[default]
    Normal,
    /// Breaks between any two letters (reduces min-content sizes).
    BreakAll,
    /// No breaks between letters of CJK text (treated as `Normal` by
    /// layouts that do not segment CJK text).
    KeepAll,
}

/// Largest number of explicit tracks of a [`GridTemplate`]: `repeat(N, …)`
/// expansions (and an auto-repeat's own track list) stop there.
pub const MAX_GRID_TRACKS: usize = 1000;

/// Largest absolute grid line number / span stored in a [`GridLine`].
pub const MAX_GRID_LINE: i32 = 10_000;

/// One track sizing function of `grid-template-columns` / `-rows`.
/// Lengths are computed to px; percentages are kept (resolved by layout
/// against the grid container's content box).
#[derive(Clone, Debug, PartialEq)]
pub enum GridTrack {
    /// Non-negative `<length-percentage>`.
    Length(LengthPercentage),
    /// `<flex>` (`1fr`), non-negative.
    Fr(f32),
    Auto,
    MinContent,
    MaxContent,
    /// `fit-content(<length-percentage>)`.
    FitContent(LengthPercentage),
    /// `minmax(min, max)`: `min` is never `Fr`, neither is ever `MinMax` or `FitContent`.
    MinMax(Box<GridTrack>, Box<GridTrack>),
}

impl GridTrack {
    /// A `<fixed-size>`: a length/percentage, or a `minmax()` with one
    /// length/percentage bound (the only tracks allowed next to an auto-repeat).
    pub fn is_fixed(&self) -> bool {
        match self {
            GridTrack::Length(_) => true,
            GridTrack::MinMax(min, max) => {
                matches!(**min, GridTrack::Length(_)) || matches!(**max, GridTrack::Length(_))
            }
            _ => false,
        }
    }
}

/// `repeat(auto-fill | auto-fit, <fixed-size>+)`.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoRepeat {
    /// `auto-fill` (true) or `auto-fit` (false: empty repeated tracks collapse).
    pub fill: bool,
    /// The repeated tracks (at most [`MAX_GRID_TRACKS`]).
    pub tracks: Vec<GridTrack>,
}

/// A computed `grid-template-columns` / `grid-template-rows` track list.
///
/// Line names (`[a b]`) are dropped and `repeat(N, …)` is expanded
/// (at most [`MAX_GRID_TRACKS`] tracks in total). An auto-repeat is kept
/// apart: its tracks, repeated as many times as fit, go between
/// `tracks[..auto_repeat_index]` and `tracks[auto_repeat_index..]`
/// (all of `tracks` are then fixed sizes, see [`GridTrack::is_fixed`]).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct GridTemplate {
    pub tracks: Vec<GridTrack>,
    pub auto_repeat: Option<AutoRepeat>,
    /// Insertion point of the auto-repeated tracks in `tracks` (0 without auto-repeat).
    pub auto_repeat_index: usize,
}

/// One end of a grid item's placement (`grid-column-start`, ...).
/// Named lines (`<custom-ident>`, with or without an integer or `span`)
/// are not supported and compute to `Auto`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum GridLine {
    #[default]
    Auto,
    /// A line number, never 0 (negative numbers count from the end of the
    /// explicit grid), clamped to ±[`MAX_GRID_LINE`].
    Line(i32),
    /// `span N`, N ≥ 1 (clamped to [`MAX_GRID_LINE`]).
    Span(u32),
}

/// `grid-column` / `grid-row`: start and end lines.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct GridPlacement {
    pub start: GridLine,
    pub end: GridLine,
}

impl GridPlacement {
    pub const AUTO: Self = Self {
        start: GridLine::Auto,
        end: GridLine::Auto,
    };

    pub fn is_auto(&self) -> bool {
        *self == Self::AUTO
    }
}

/// One item of the `content` property for `::before` / `::after`.
#[derive(Clone, Debug, PartialEq)]
pub enum ContentItem {
    /// Literal string, or the resolved value of `attr(name)`.
    Text(String),
    /// `url(...)` image (raw, unresolved URL).
    Image(String),
    /// `open-quote` / `close-quote` render as typographic quotes.
    OpenQuote,
    CloseQuote,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Content {
    /// `normal` / `none`: the pseudo-element generates no box.
    #[default]
    Normal,
    Items(Vec<ContentItem>),
}

/// Background image layer (only the first layer is kept by xbro).
#[derive(Clone, Debug, PartialEq)]
pub enum BackgroundImage {
    /// `url(...)` string. Absolute when the engine rewrote the sheet's URLs
    /// (see `Stylesheet::rewrite_urls`); inline `style=""` URLs are relative to the document.
    Url(String),
    /// `linear-gradient(...)` with evenly-spaced or positioned stops.
    LinearGradient {
        /// Angle in degrees, CSS convention (0 = to top, 90 = to right, 180 = to bottom).
        angle_deg: f32,
        /// (color, position 0..=1) — positions are filled in evenly when omitted.
        stops: Vec<(Color, f32)>,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum BackgroundRepeat {
    #[default]
    Repeat,
    RepeatX,
    RepeatY,
    NoRepeat,
}

#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub enum BackgroundSize {
    #[default]
    Auto,
    Cover,
    Contain,
    /// Explicit width / height; `None` means auto.
    Explicit(Option<LengthPercentage>, Option<LengthPercentage>),
}

/// The fully computed style of one element (or pseudo-element / text run, which
/// share their parent's inherited values).
///
/// Field defaults ([`Default`]) are the CSS initial values, except where noted.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedStyle {
    // ---- box generation & positioning ----
    pub display: Display,
    pub position: Position,
    pub top: LengthPercentageAuto,
    pub right: LengthPercentageAuto,
    pub bottom: LengthPercentageAuto,
    pub left: LengthPercentageAuto,
    pub z_index: Option<i32>,
    pub float: Float,
    pub clear: Clear,

    // ---- box model ----
    pub box_sizing: BoxSizing,
    pub width: LengthPercentageAuto,
    pub height: LengthPercentageAuto,
    /// `auto` min sizes are computed as `Length(ZERO)` except for flex items, where layout
    /// may apply the automatic minimum; this field keeps `Auto` when specified as such.
    pub min_width: LengthPercentageAuto,
    pub min_height: LengthPercentageAuto,
    pub max_width: MaxSize,
    pub max_height: MaxSize,
    pub margin: Sides<LengthPercentageAuto>,
    pub padding: Sides<LengthPercentage>,
    pub border: Sides<BorderSide>,
    /// Corner radii: top-left, top-right, bottom-right, bottom-left (circular; percentages
    /// resolve against the border-box width).
    pub border_radius: [LengthPercentage; 4],
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    /// `aspect-ratio: w / h` as w/h; `None` = auto.
    pub aspect_ratio: Option<f32>,

    // ---- visual ----
    pub visibility: Visibility,
    pub opacity: f32,
    pub background_color: Color,
    pub background_image: Option<BackgroundImage>,
    pub background_repeat: BackgroundRepeat,
    pub background_size: BackgroundSize,
    /// `background-position` x / y.
    pub background_position: (LengthPercentage, LengthPercentage),
    /// `box-shadow` is not supported. `outline` is approximated: width/color painted outside the border box.
    pub outline_width: f32,
    pub outline_color: Color,
    pub cursor: Cursor,

    // ---- text & fonts (inherited) ----
    pub color: Color,
    /// Lowercased family names in priority order, including generic families
    /// (`serif`, `sans-serif`, `monospace`, `cursive`, `fantasy`, `system-ui`).
    pub font_family: Arc<Vec<String>>,
    /// Computed font size in px.
    pub font_size: f32,
    /// 100..=900 (`normal` = 400, `bold` = 700, `bolder`/`lighter` relative to parent).
    pub font_weight: u16,
    pub font_style: FontStyle,
    pub line_height: LineHeight,
    pub text_align: TextAlign,
    pub text_indent: LengthPercentage,
    pub text_transform: TextTransform,
    /// Extra px between characters (`normal` = 0).
    pub letter_spacing: f32,
    /// Extra px added to each space (`normal` = 0).
    pub word_spacing: f32,
    pub white_space: WhiteSpace,
    /// `overflow-wrap: anywhere | break-word` or `word-break: break-all | break-word`
    /// (kept for compatibility). Layout should prefer the precise
    /// [`overflow_wrap`](Self::overflow_wrap) / [`word_break`](Self::word_break):
    /// all of these allow breaking overflowing words, but only
    /// `overflow-wrap: anywhere` and `word-break: break-all` reduce the
    /// min-content size (`break-word` does not).
    pub break_anywhere: bool,
    /// `overflow-wrap` (also raised to `BreakWord` by `word-break: break-word`).
    pub overflow_wrap: OverflowWrap,
    /// `word-break` (`break-word` computes to `Normal`, see [`WordBreak`]).
    pub word_break: WordBreak,
    pub list_style_type: ListStyleType,
    pub list_style_position: ListStylePosition,
    /// `list-style-image` raw URL (shared: it is inherited by every descendant).
    pub list_style_image: Option<Arc<str>>,
    pub border_collapse: BorderCollapse,
    /// Horizontal / vertical `border-spacing` in px.
    pub border_spacing: (f32, f32),
    /// `caption-side` (inherited).
    pub caption_side: CaptionSide,
    /// `table-layout` (not inherited).
    pub table_layout: TableLayout,

    // ---- text decoration (not inherited, but propagated) ----
    /// The element's own `text-decoration-line`.
    pub text_decoration_line: TextDecorationLine,
    /// `text-decoration-color`; `None` = currentcolor.
    pub text_decoration_color: Option<Color>,
    /// Decorations in effect for text in this box: union of this element's and all
    /// ancestors' lines (stopping at atomic inlines / out-of-flow boxes per spec,
    /// approximated). Layout/paint use this, painting with `decoration_color`.
    pub decorations_in_effect: TextDecorationLine,
    /// Color for `decorations_in_effect`. Per CSS Text Decoration 3, a
    /// decoration is drawn in the color of the box that *declares* it
    /// (its `text-decoration-color`, by default that box's own `color`),
    /// so a propagated decoration keeps the decorating ancestor's color
    /// whatever the descendant's `color` (`<p style="text-decoration:
    /// underline"><em style="color: blue">` underlines the `em` in the `p`'s
    /// color). Approximation: a single color is kept per box, so when a
    /// box adds lines of its own, its color is used for all its lines,
    /// including the propagated ones.
    pub decoration_color: Color,

    pub vertical_align: VerticalAlign,

    // ---- flexbox ----
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub align_self: AlignItems,
    pub align_content: JustifyContent,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: LengthPercentageAuto,
    pub order: i32,
    pub row_gap: LengthPercentage,
    pub column_gap: LengthPercentage,
    /// Number of explicit columns of `grid_template_columns` (`tracks.len()`);
    /// 0 when not a grid, without a template, or when the template has an
    /// auto-repeat (the count then depends on the container size). Used by the
    /// grid approximation.
    pub grid_columns: u32,
    /// `grid-template-columns` (also set by `grid-template` / `grid`); `None` for
    /// `none` (and the unsupported `subgrid` / `masonry`). Not inherited.
    pub grid_template_columns: Option<Arc<GridTemplate>>,
    /// `grid-template-rows`, like `grid_template_columns`.
    pub grid_template_rows: Option<Arc<GridTemplate>>,
    /// `grid-column-start` / `grid-column-end` (`grid-column`, `grid-area`).
    pub grid_column: GridPlacement,
    /// `grid-row-start` / `grid-row-end` (`grid-row`, `grid-area`).
    pub grid_row: GridPlacement,

    // ---- generated content ----
    pub content: Content,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            display: Display::Inline,
            position: Position::Static,
            top: LengthPercentageAuto::Auto,
            right: LengthPercentageAuto::Auto,
            bottom: LengthPercentageAuto::Auto,
            left: LengthPercentageAuto::Auto,
            z_index: None,
            float: Float::None,
            clear: Clear::None,
            box_sizing: BoxSizing::ContentBox,
            width: LengthPercentageAuto::Auto,
            height: LengthPercentageAuto::Auto,
            min_width: LengthPercentageAuto::Auto,
            min_height: LengthPercentageAuto::Auto,
            max_width: MaxSize::None,
            max_height: MaxSize::None,
            margin: Sides::all(LengthPercentageAuto::ZERO),
            padding: Sides::all(LengthPercentage::ZERO),
            border: Sides::all(BorderSide {
                width: 0.0,
                style: BorderStyle::None,
                color: Color::BLACK,
            }),
            border_radius: [LengthPercentage::ZERO; 4],
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            aspect_ratio: None,
            visibility: Visibility::Visible,
            opacity: 1.0,
            background_color: Color::TRANSPARENT,
            background_image: None,
            background_repeat: BackgroundRepeat::Repeat,
            background_size: BackgroundSize::Auto,
            background_position: (LengthPercentage::ZERO, LengthPercentage::ZERO),
            outline_width: 0.0,
            outline_color: Color::BLACK,
            cursor: Cursor::Auto,
            color: Color::BLACK,
            font_family: Arc::new(vec!["sans-serif".to_string()]),
            font_size: 16.0,
            font_weight: 400,
            font_style: FontStyle::Normal,
            line_height: LineHeight::Normal,
            text_align: TextAlign::Left,
            text_indent: LengthPercentage::ZERO,
            text_transform: TextTransform::None,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            white_space: WhiteSpace::Normal,
            break_anywhere: false,
            overflow_wrap: OverflowWrap::Normal,
            word_break: WordBreak::Normal,
            list_style_type: ListStyleType::Disc,
            list_style_position: ListStylePosition::Outside,
            list_style_image: None,
            border_collapse: BorderCollapse::Separate,
            border_spacing: (0.0, 0.0),
            caption_side: CaptionSide::Top,
            table_layout: TableLayout::Auto,
            text_decoration_line: TextDecorationLine::NONE,
            text_decoration_color: None,
            decorations_in_effect: TextDecorationLine::NONE,
            decoration_color: Color::BLACK,
            vertical_align: VerticalAlign::Baseline,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            align_self: AlignItems::Auto,
            align_content: JustifyContent::Stretch,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: LengthPercentageAuto::Auto,
            order: 0,
            row_gap: LengthPercentage::ZERO,
            column_gap: LengthPercentage::ZERO,
            grid_columns: 0,
            grid_template_columns: None,
            grid_template_rows: None,
            grid_column: GridPlacement::AUTO,
            grid_row: GridPlacement::AUTO,
            content: Content::Normal,
        }
    }
}

impl ComputedStyle {
    /// Style for anonymous boxes and text runs: inherits the inherited
    /// properties of `parent`, with every non-inherited property at its
    /// initial value (except `decorations_in_effect`/`decoration_color`,
    /// which propagate).
    pub fn inherit_from(parent: &ComputedStyle) -> ComputedStyle {
        ComputedStyle {
            color: parent.color,
            font_family: parent.font_family.clone(),
            font_size: parent.font_size,
            font_weight: parent.font_weight,
            font_style: parent.font_style,
            line_height: parent.line_height,
            text_align: parent.text_align,
            text_indent: parent.text_indent,
            text_transform: parent.text_transform,
            letter_spacing: parent.letter_spacing,
            word_spacing: parent.word_spacing,
            white_space: parent.white_space,
            break_anywhere: parent.break_anywhere,
            overflow_wrap: parent.overflow_wrap,
            word_break: parent.word_break,
            list_style_type: parent.list_style_type,
            list_style_position: parent.list_style_position,
            list_style_image: parent.list_style_image.clone(),
            border_collapse: parent.border_collapse,
            border_spacing: parent.border_spacing,
            caption_side: parent.caption_side,
            visibility: parent.visibility,
            cursor: parent.cursor,
            decorations_in_effect: parent.decorations_in_effect,
            decoration_color: parent.decoration_color,
            ..ComputedStyle::initial().clone()
        }
    }

    /// The initial values ([`ComputedStyle::default`]), built once and shared
    /// (cloning it only bumps the `font_family` reference count).
    pub fn initial() -> &'static ComputedStyle {
        static INITIAL: OnceLock<ComputedStyle> = OnceLock::new();
        INITIAL.get_or_init(ComputedStyle::default)
    }

    /// Used line height in px, given the primary font's `normal` line height
    /// (ascent + descent + line gap at this font size).
    pub fn line_height_px(&self, normal_line_height: f32) -> f32 {
        match self.line_height {
            LineHeight::Normal => normal_line_height,
            LineHeight::Number(n) => n * self.font_size,
            LineHeight::Px(px) => px,
        }
    }

    pub fn is_italic(&self) -> bool {
        !matches!(self.font_style, FontStyle::Normal)
    }

    /// Floats and absolutely positioned boxes are out of the normal flow.
    pub fn is_out_of_flow(&self) -> bool {
        self.float != Float::None || self.position.is_out_of_flow()
    }

    /// Used border widths (0 for `none`/`hidden` sides).
    pub fn border_widths(&self) -> Sides<f32> {
        let w = |s: &BorderSide| if s.style.is_visible() { s.width } else { 0.0 };
        Sides {
            top: w(&self.border.top),
            right: w(&self.border.right),
            bottom: w(&self.border.bottom),
            left: w(&self.border.left),
        }
    }
}
