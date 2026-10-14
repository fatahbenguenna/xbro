//! Per-element computation: winning declarations → [`ComputedStyle`].
//!
//! Given the matched declaration blocks of an element in cascade order, the
//! winning declaration of every longhand is found, custom properties are
//! computed, and each longhand is computed against the parent style. Font
//! properties and `color` are computed first since other values depend on
//! them (`em`, `currentcolor`). Finally the style adjustments run
//! (border widths of `none` borders, blockification, decorations, ...).

mod fields;

use std::sync::Arc;

use xbro_dom::ElementData;

use crate::computed::*;
use crate::properties::{
    CssWide, LonghandId, PropertyDeclaration, Spec, SpecBgSize, SpecContent, SpecContentItem,
    SpecFontSize, SpecFontWeight, SpecImage, SpecLineHeight, SpecLpa, SpecMax, SpecTextAlign,
    SpecVerticalAlign, UnparsedValue, WsCollapse, ALL_LONGHANDS, LONGHAND_COUNT,
};
use crate::values::{sane, MathExpr, Resolver, SpecColor};
use crate::vars::{CustomMap, VarContext};
use fields::{
    border_style_field, border_width_index, color_field, copy_field, lp_field, lpa_field,
};

/// Computed values of the internal longhands that are combined into a
/// single `ComputedStyle` field; all of them are inherited.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct Extra {
    pub ws_collapse: WsCollapse,
    pub nowrap: bool,
    pub overflow_wrap: bool,
    pub word_break: bool,
}

/// Document-wide inputs.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Env {
    pub viewport: (f32, f32),
    /// Computed font size of the root element (16px while computing the root itself).
    pub root_font_size: f32,
}

/// What an element inherits from.
pub(crate) struct Parent<'a> {
    pub style: &'a ComputedStyle,
    pub extra: &'a Extra,
    pub customs: &'a CustomMap,
    /// Display of the parent box (nearest ancestor that is not
    /// `display: contents`); flex and grid containers blockify their children.
    pub box_display: Option<Display>,
}

/// A matched declaration block, with its origin.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Block<'a> {
    /// Longhand declarations.
    pub decls: &'a [PropertyDeclaration],
    /// Custom property declarations. The slice must stay alive at the same
    /// address while the [`VarContext`] is in use (it is a memoization key).
    pub customs: &'a [PropertyDeclaration],
    /// From the user-agent origin (`revert` in author blocks rolls back to these).
    pub user_agent: bool,
}

/// Result of computing one element.
pub(crate) struct Computed {
    pub style: ComputedStyle,
    pub extra: Extra,
    pub customs: CustomMap,
}

/// The initial values (`ComputedStyle::default()`), built once.
pub(crate) fn initial_style() -> &'static ComputedStyle {
    ComputedStyle::initial()
}

/// Longhands computed before the others, in this order.
const PRIORITY: [LonghandId; 4] = [
    LonghandId::FontFamily,
    LonghandId::FontSize,
    LonghandId::LineHeight,
    LonghandId::Color,
];

/// True when the family list asks for a monospace font (for the `ch` unit).
pub(crate) fn is_monospace(families: &[String]) -> bool {
    families.iter().take(4).any(|f| {
        matches!(f.as_str(), "monospace" | "ui-monospace")
            || (f.contains("mono") && !f.contains("monotype"))
            || f.contains("courier")
            || f == "consolas"
            || f == "menlo"
            || f == "monaco"
    })
}

/// Computes the style of an element (or pseudo-element) from its matched
/// declaration blocks, given in increasing cascade priority.
pub(crate) fn compute(
    blocks: &[Block<'_>],
    parent: &Parent<'_>,
    env: &Env,
    element: Option<&ElementData>,
    is_root: bool,
    vars: &mut VarContext,
) -> Computed {
    // Winning declaration of each longhand (and whether it is from the UA
    // origin), plus the winning UA declaration for `revert`.
    let mut slots: [Option<(&PropertyDeclaration, bool)>; LONGHAND_COUNT] = [None; LONGHAND_COUNT];
    let mut ua_slots: [Option<&PropertyDeclaration>; LONGHAND_COUNT] = [None; LONGHAND_COUNT];
    for block in blocks {
        for decl in block.decls {
            let Some(index) = decl.longhand().map(LonghandId::index) else {
                continue; // custom properties are in `block.customs`
            };
            if let Some(slot) = slots.get_mut(index) {
                *slot = Some((decl, block.user_agent));
            }
            if block.user_agent {
                if let Some(slot) = ua_slots.get_mut(index) {
                    *slot = Some(decl);
                }
            }
        }
    }
    let lists: Vec<&[PropertyDeclaration]> = blocks.iter().map(|b| b.customs).collect();
    let custom_map = vars.custom_properties(parent.customs, &lists);
    let mut state = State::new(parent, env, element, &custom_map, vars);
    let apply = |state: &mut State<'_>, id: LonghandId| {
        let Some((decl, from_ua)) = slots.get(id.index()).copied().flatten() else {
            return;
        };
        if matches!(decl, PropertyDeclaration::Wide(_, CssWide::Revert)) {
            // Roll back to the user-agent origin (no user origin in xbro).
            match ua_slots.get(id.index()).copied().flatten() {
                Some(ua)
                    if !from_ua && !matches!(ua, PropertyDeclaration::Wide(_, CssWide::Revert)) =>
                {
                    state.apply(id, ua)
                }
                _ => state.apply_wide(id, CssWide::Unset),
            }
        } else {
            state.apply(id, decl);
        }
    };
    for id in PRIORITY {
        apply(&mut state, id);
        if id == LonghandId::FontSize {
            state.update_resolver(is_root);
        }
    }
    state.init_current_color();
    for &id in ALL_LONGHANDS {
        if !PRIORITY.contains(&id) {
            apply(&mut state, id);
        }
    }
    let (style, extra) = state.finish(parent, is_root);
    Computed {
        style,
        extra,
        customs: custom_map,
    }
}

struct State<'a> {
    style: ComputedStyle,
    extra: Extra,
    parent: &'a ComputedStyle,
    parent_extra: &'a Extra,
    env: &'a Env,
    element: Option<&'a ElementData>,
    customs: &'a CustomMap,
    /// Memoized `var()` substitutions (shared across elements).
    vars: &'a mut VarContext,
    /// Border widths before zeroing the sides whose style is `none`/`hidden`.
    border_width: [f32; 4],
    outline_width: f32,
    outline_style: BorderStyle,
    /// Color approximating an unsupported background image.
    approx_bg: Option<Color>,
    /// Unit resolution for the element's own font.
    resolver: Resolver,
}

impl<'a> State<'a> {
    fn new(
        parent: &'a Parent<'a>,
        env: &'a Env,
        element: Option<&'a ElementData>,
        customs: &'a CustomMap,
        vars: &'a mut VarContext,
    ) -> Self {
        let style = ComputedStyle::inherit_from(parent.style);
        let resolver = Resolver::new(
            style.font_size,
            env.root_font_size,
            is_monospace(&style.font_family),
            env.viewport,
        );
        State {
            style,
            extra: *parent.extra,
            parent: parent.style,
            parent_extra: parent.extra,
            env,
            element,
            customs,
            vars,
            border_width: [3.0; 4],
            outline_width: 3.0,
            outline_style: BorderStyle::None,
            approx_bg: None,
            resolver,
        }
    }

    /// Resolver for font-relative units of `font-size` itself (parent's font).
    fn parent_resolver(&self) -> Resolver {
        Resolver::new(
            self.parent.font_size,
            self.env.root_font_size,
            is_monospace(&self.parent.font_family),
            self.env.viewport,
        )
        .with_percent_basis(self.parent.font_size)
    }

    /// Resolver for the element's own font, once its font size is known.
    /// On the root element, `rem` (outside `font-size`) is its own font size.
    fn update_resolver(&mut self, is_root: bool) {
        let rem = if is_root {
            self.style.font_size
        } else {
            self.env.root_font_size
        };
        self.resolver = Resolver::new(
            self.style.font_size,
            rem,
            is_monospace(&self.style.font_family),
            self.env.viewport,
        );
    }

    /// Colors whose initial value is `currentcolor`.
    fn init_current_color(&mut self) {
        let c = self.style.color;
        for side in [
            &mut self.style.border.top,
            &mut self.style.border.right,
            &mut self.style.border.bottom,
            &mut self.style.border.left,
        ] {
            side.color = c;
        }
        self.style.outline_color = c;
    }

    fn apply(&mut self, id: LonghandId, decl: &PropertyDeclaration) {
        match decl {
            PropertyDeclaration::Value(_, spec) => self.compute_value(id, spec),
            PropertyDeclaration::Wide(_, wide) => self.apply_wide(id, *wide),
            PropertyDeclaration::Unparsed(_, unparsed) => self.apply_unparsed(id, unparsed),
            PropertyDeclaration::Custom(..) => {}
        }
    }

    /// Substitutes `var()`s and parses the result; invalid → `unset`.
    fn apply_unparsed(&mut self, id: LonghandId, unparsed: &Arc<UnparsedValue>) {
        let parsed = self.vars.parse_unparsed(unparsed, self.customs);
        let decl = parsed
            .as_deref()
            .and_then(|list| list.iter().rev().find(|d| d.longhand() == Some(id)))
            .cloned();
        match decl {
            Some(PropertyDeclaration::Value(_, spec)) => self.compute_value(id, &spec),
            Some(PropertyDeclaration::Wide(_, wide)) => self.apply_wide(id, wide),
            _ => self.apply_wide(id, CssWide::Unset),
        }
    }

    fn apply_wide(&mut self, id: LonghandId, wide: CssWide) {
        let inherit = match wide {
            CssWide::Inherit => true,
            CssWide::Initial => false,
            CssWide::Unset | CssWide::Revert => id.is_inherited(),
        };
        let initial = !inherit;
        let src: &ComputedStyle = if inherit {
            self.parent
        } else {
            initial_style()
        };
        use LonghandId as L;
        if let Some(i) = border_width_index(id) {
            let parent_widths = [
                src.border.top.width,
                src.border.right.width,
                src.border.bottom.width,
                src.border.left.width,
            ];
            if let (Some(slot), Some(w)) = (self.border_width.get_mut(i), parent_widths.get(i)) {
                *slot = if initial { 3.0 } else { *w };
            }
            return;
        }
        match id {
            L::OutlineWidth => {
                self.outline_width = if initial { 3.0 } else { src.outline_width };
            }
            L::OutlineStyle => {
                // Only the used width is stored: approximate the parent's style.
                self.outline_style = if !initial && src.outline_width > 0.0 {
                    BorderStyle::Solid
                } else {
                    BorderStyle::None
                };
            }
            L::BorderTopColor
            | L::BorderRightColor
            | L::BorderBottomColor
            | L::BorderLeftColor
            | L::OutlineColor
                if initial =>
            {
                let c = self.style.color;
                if let Some(f) = color_field(&mut self.style, id) {
                    *f = c;
                }
            }
            L::WhiteSpaceCollapse | L::TextWrapMode | L::OverflowWrap | L::WordBreak => {
                let src_extra = if inherit {
                    *self.parent_extra
                } else {
                    Extra::default()
                };
                match id {
                    L::WhiteSpaceCollapse => self.extra.ws_collapse = src_extra.ws_collapse,
                    L::TextWrapMode => self.extra.nowrap = src_extra.nowrap,
                    L::OverflowWrap => self.extra.overflow_wrap = src_extra.overflow_wrap,
                    _ => self.extra.word_break = src_extra.word_break,
                }
            }
            _ => copy_field(id, &mut self.style, src),
        }
    }

    /// Computes a length expression with the element's font, clamping
    /// negative results for properties that do not accept them.
    fn lp(&self, e: &MathExpr, non_negative: bool) -> LengthPercentage {
        let mut v = e.eval(&self.resolver);
        if non_negative {
            if v.percent == 0.0 {
                v.px = v.px.max(0.0);
            } else if v.px == 0.0 {
                v.percent = v.percent.max(0.0);
            }
        }
        v
    }

    fn line_height_px(&self) -> f32 {
        match self.style.line_height {
            LineHeight::Normal => 1.2 * self.style.font_size,
            LineHeight::Number(n) => n * self.style.font_size,
            LineHeight::Px(px) => px,
        }
    }

    fn compute_value(&mut self, id: LonghandId, spec: &Spec) {
        use LonghandId as L;
        match spec {
            Spec::Display(d) => self.style.display = *d,
            Spec::Position(p) => self.style.position = *p,
            Spec::Lpa(v) => {
                let value = match v {
                    SpecLpa::Auto => LengthPercentageAuto::Auto,
                    SpecLpa::Lp(e) => {
                        LengthPercentageAuto::Length(self.lp(e, id.is_non_negative()))
                    }
                };
                if let Some(f) = lpa_field(&mut self.style, id) {
                    *f = value;
                }
            }
            Spec::Lp(e) => {
                let value = self.lp(e, id.is_non_negative());
                if let Some(f) = lp_field(&mut self.style, id) {
                    *f = value;
                }
            }
            Spec::Max(m) => {
                let value = match m {
                    SpecMax::None => MaxSize::None,
                    SpecMax::Lp(e) => MaxSize::Length(self.lp(e, true)),
                };
                match id {
                    L::MaxWidth => self.style.max_width = value,
                    L::MaxHeight => self.style.max_height = value,
                    _ => {}
                }
            }
            Spec::Length(e) => {
                let px = self.lp(e, true).px.max(0.0);
                if let Some(slot) =
                    border_width_index(id).and_then(|i| self.border_width.get_mut(i))
                {
                    *slot = px;
                } else if id == L::OutlineWidth {
                    self.outline_width = px;
                }
            }
            Spec::ZIndex(z) => self.style.z_index = *z,
            Spec::Float(f) => self.style.float = *f,
            Spec::Clear(c) => self.style.clear = *c,
            Spec::BoxSizing(b) => self.style.box_sizing = *b,
            Spec::BorderStyle(b) => {
                if id == L::OutlineStyle {
                    self.outline_style = *b;
                } else if let Some(f) = border_style_field(&mut self.style, id) {
                    *f = *b;
                }
            }
            Spec::Color(c) => self.set_color(id, *c),
            Spec::Overflow(o) => match id {
                L::OverflowX => self.style.overflow_x = *o,
                L::OverflowY => self.style.overflow_y = *o,
                _ => {}
            },
            Spec::AspectRatio(r) => self.style.aspect_ratio = *r,
            Spec::Visibility(v) => self.style.visibility = *v,
            Spec::Number(n) => match id {
                L::Opacity => self.style.opacity = n.clamp(0.0, 1.0),
                L::FlexGrow => self.style.flex_grow = sane(n.max(0.0)),
                L::FlexShrink => self.style.flex_shrink = sane(n.max(0.0)),
                _ => {}
            },
            Spec::BgImage(img) => self.set_background_image(img.as_ref()),
            Spec::BgRepeat(r) => self.style.background_repeat = *r,
            Spec::BgSize(s) => {
                self.style.background_size = match s {
                    SpecBgSize::Auto => BackgroundSize::Auto,
                    SpecBgSize::Cover => BackgroundSize::Cover,
                    SpecBgSize::Contain => BackgroundSize::Contain,
                    SpecBgSize::Explicit(w, h) => BackgroundSize::Explicit(
                        w.as_ref().map(|e| self.lp(e, true)),
                        h.as_ref().map(|e| self.lp(e, true)),
                    ),
                }
            }
            Spec::Cursor(c) => self.style.cursor = *c,
            Spec::FontFamily(f) => self.style.font_family = f.clone(),
            Spec::FontSize(fs) => self.style.font_size = self.font_size(fs),
            Spec::FontWeight(w) => self.style.font_weight = self.font_weight(*w),
            Spec::FontStyle(s) => self.style.font_style = *s,
            Spec::LineHeight(lh) => {
                self.style.line_height = match lh {
                    SpecLineHeight::Normal => LineHeight::Normal,
                    SpecLineHeight::Number(n) => LineHeight::Number(sane(*n)),
                    SpecLineHeight::Length(e) => {
                        let r = self.resolver.with_percent_basis(self.style.font_size);
                        LineHeight::Px(e.eval(&r).px.max(0.0))
                    }
                }
            }
            Spec::TextAlign(a) => {
                self.style.text_align = match a {
                    SpecTextAlign::Align(t) => *t,
                    SpecTextAlign::MatchParent => self.parent.text_align,
                }
            }
            Spec::TextTransform(t) => self.style.text_transform = *t,
            Spec::Spacing(s) => {
                let fs = self.style.font_size;
                let px = match s {
                    None => 0.0,
                    Some(e) => {
                        // Percentages: of the font size for letter-spacing, of
                        // the space width (≈ 0.25em) for word-spacing.
                        let basis = if id == L::WordSpacing { fs * 0.25 } else { fs };
                        e.eval(&self.resolver.with_percent_basis(basis)).px
                    }
                };
                match id {
                    L::LetterSpacing => self.style.letter_spacing = px,
                    L::WordSpacing => self.style.word_spacing = px,
                    _ => {}
                }
            }
            Spec::WsCollapse(c) => self.extra.ws_collapse = *c,
            Spec::Bool(b) => match id {
                L::TextWrapMode => self.extra.nowrap = *b,
                L::OverflowWrap => self.extra.overflow_wrap = *b,
                L::WordBreak => self.extra.word_break = *b,
                _ => {}
            },
            Spec::ListStyleType(t) => self.style.list_style_type = *t,
            Spec::ListStylePosition(p) => self.style.list_style_position = *p,
            Spec::ListStyleImage(i) => self.style.list_style_image = i.clone(),
            Spec::BorderCollapse(b) => self.style.border_collapse = *b,
            Spec::BorderSpacing(h, v) => {
                self.style.border_spacing = (self.lp(h, true).px, self.lp(v, true).px)
            }
            Spec::DecorationLine(l) => self.style.text_decoration_line = *l,
            Spec::VerticalAlign(v) => {
                self.style.vertical_align = match v {
                    SpecVerticalAlign::Keyword(k) => *k,
                    SpecVerticalAlign::Length(e) => {
                        let r = self.resolver.with_percent_basis(self.line_height_px());
                        VerticalAlign::Length(e.eval(&r).px)
                    }
                }
            }
            Spec::FlexDirection(d) => self.style.flex_direction = *d,
            Spec::FlexWrap(w) => self.style.flex_wrap = *w,
            Spec::Justify(j) => match id {
                L::JustifyContent => self.style.justify_content = *j,
                L::AlignContent => self.style.align_content = *j,
                _ => {}
            },
            Spec::Align(a) => match id {
                L::AlignItems => {
                    self.style.align_items = if *a == AlignItems::Auto {
                        AlignItems::Stretch
                    } else {
                        *a
                    }
                }
                L::AlignSelf => self.style.align_self = *a,
                _ => {}
            },
            Spec::Integer(i) => {
                if id == L::Order {
                    self.style.order = *i;
                }
            }
            Spec::Count(n) => {
                if id == L::GridColumns {
                    self.style.grid_columns = *n;
                }
            }
            Spec::Content(c) => self.style.content = self.content(c),
        }
    }

    fn set_color(&mut self, id: LonghandId, c: SpecColor) {
        match id {
            LonghandId::Color => {
                // `color: currentcolor` behaves as `inherit`.
                self.style.color = c.resolve(self.parent.color);
            }
            LonghandId::TextDecorationColor => {
                self.style.text_decoration_color = match c {
                    SpecColor::Color(x) => Some(x),
                    SpecColor::CurrentColor => None,
                };
            }
            _ => {
                let resolved = c.resolve(self.style.color);
                if let Some(f) = color_field(&mut self.style, id) {
                    *f = resolved;
                }
            }
        }
    }

    fn set_background_image(&mut self, img: Option<&SpecImage>) {
        self.approx_bg = None;
        self.style.background_image = match img {
            None => None,
            Some(SpecImage::Url(u)) => Some(BackgroundImage::Url(u.clone())),
            Some(SpecImage::LinearGradient { angle_deg, stops }) => {
                Some(BackgroundImage::LinearGradient {
                    angle_deg: *angle_deg,
                    stops: fill_stops(stops, self.style.color),
                })
            }
            Some(SpecImage::Approx(c)) => {
                self.approx_bg = Some(c.resolve(self.style.color));
                None
            }
        };
    }

    fn font_size(&self, fs: &SpecFontSize) -> f32 {
        let parent = self.parent.font_size;
        let px = match fs {
            SpecFontSize::Px(px) => *px,
            SpecFontSize::Smaller => parent / 1.2,
            SpecFontSize::Larger => parent * 1.2,
            SpecFontSize::Length(e) => e.eval(&self.parent_resolver()).px,
        };
        sane(px).max(0.0)
    }

    fn font_weight(&self, w: SpecFontWeight) -> u16 {
        let parent = f32::from(self.parent.font_weight);
        let weight = match w {
            SpecFontWeight::Absolute(n) => n,
            // CSS Fonts 4 relative weights.
            SpecFontWeight::Bolder => {
                if parent < 350.0 {
                    400.0
                } else if parent < 550.0 {
                    700.0
                } else {
                    parent.max(900.0)
                }
            }
            SpecFontWeight::Lighter => {
                if parent < 100.0 {
                    parent
                } else if parent < 550.0 {
                    100.0
                } else if parent < 750.0 {
                    400.0
                } else {
                    700.0
                }
            }
        };
        weight.round().clamp(100.0, 900.0) as u16
    }

    fn content(&self, c: &SpecContent) -> Content {
        match c {
            SpecContent::Normal => Content::Normal,
            SpecContent::Items(items) => Content::Items(
                items
                    .iter()
                    .map(|item| match item {
                        SpecContentItem::Text(t) => ContentItem::Text(t.clone()),
                        SpecContentItem::Attr { name, fallback } => ContentItem::Text(
                            self.element
                                .and_then(|e| e.attr(name))
                                .unwrap_or(fallback)
                                .to_string(),
                        ),
                        SpecContentItem::Image(u) => ContentItem::Image(u.clone()),
                        SpecContentItem::OpenQuote => ContentItem::OpenQuote,
                        SpecContentItem::CloseQuote => ContentItem::CloseQuote,
                    })
                    .collect(),
            ),
        }
    }

    /// Style adjustments once every longhand is computed.
    fn finish(self, parent: &Parent<'_>, is_root: bool) -> (ComputedStyle, Extra) {
        let mut s = self.style;
        let widths = self.border_width;
        for (side, w) in [
            (&mut s.border.top, widths[0]),
            (&mut s.border.right, widths[1]),
            (&mut s.border.bottom, widths[2]),
            (&mut s.border.left, widths[3]),
        ] {
            side.width = if side.style.is_visible() { w } else { 0.0 };
        }
        s.outline_width = if self.outline_style.is_visible() {
            self.outline_width
        } else {
            0.0
        };
        if let Some(c) = self.approx_bg {
            if s.background_color.is_transparent() {
                s.background_color = c;
            }
        }
        s.white_space = white_space(self.extra.ws_collapse, self.extra.nowrap);
        s.break_anywhere = self.extra.overflow_wrap || self.extra.word_break;

        // overflow: visible/clip next to a scrolling axis become auto/hidden.
        let scrolls = |o: Overflow| !matches!(o, Overflow::Visible | Overflow::Clip);
        if scrolls(s.overflow_x) || scrolls(s.overflow_y) {
            let adjust = |o: Overflow| match o {
                Overflow::Visible => Overflow::Auto,
                Overflow::Clip => Overflow::Hidden,
                other => other,
            };
            s.overflow_x = adjust(s.overflow_x);
            s.overflow_y = adjust(s.overflow_y);
        }

        // Positioning and blockification.
        let out_of_flow = s.position.is_out_of_flow();
        let flex_item = matches!(
            parent.box_display,
            Some(Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid)
        );
        if out_of_flow || flex_item {
            // `float` does not apply to absolutely positioned boxes and flex/grid items.
            s.float = Float::None;
        }
        if s.display != Display::None
            && (is_root || s.float != Float::None || out_of_flow || flex_item)
        {
            s.display = blockify(s.display, is_root);
        }
        if !matches!(s.display, Display::Grid | Display::InlineGrid) {
            s.grid_columns = 0;
        }

        // Text decoration propagation.
        let own = s.text_decoration_line;
        let isolated = s.display.is_inline_block_like() || s.float != Float::None || out_of_flow;
        let inherited = if isolated {
            TextDecorationLine::NONE
        } else {
            parent.style.decorations_in_effect
        };
        s.decorations_in_effect = inherited.union(own);
        s.decoration_color = if !own.is_none() {
            s.text_decoration_color.unwrap_or(s.color)
        } else if isolated {
            s.color
        } else {
            parent.style.decoration_color
        };
        (s, self.extra)
    }
}

/// `white-space` from its two longhands.
fn white_space(collapse: WsCollapse, nowrap: bool) -> WhiteSpace {
    match (collapse, nowrap) {
        (WsCollapse::Collapse, false) => WhiteSpace::Normal,
        (WsCollapse::Collapse, true) => WhiteSpace::Nowrap,
        (WsCollapse::Preserve, false) => WhiteSpace::PreWrap,
        (WsCollapse::Preserve, true) => WhiteSpace::Pre,
        // `preserve-breaks nowrap` has no exact equivalent.
        (WsCollapse::PreserveBreaks, _) => WhiteSpace::PreLine,
        (WsCollapse::BreakSpaces, false) => WhiteSpace::BreakSpaces,
        (WsCollapse::BreakSpaces, true) => WhiteSpace::Pre,
    }
}

/// CSS Display §2.7 blockification.
pub(crate) fn blockify(d: Display, is_root: bool) -> Display {
    match d {
        Display::Inline | Display::InlineBlock => Display::Block,
        Display::InlineFlex => Display::Flex,
        Display::InlineGrid => Display::Grid,
        Display::InlineTable => Display::Table,
        Display::TableRowGroup
        | Display::TableHeaderGroup
        | Display::TableFooterGroup
        | Display::TableRow
        | Display::TableCell
        | Display::TableColumn
        | Display::TableColumnGroup
        | Display::TableCaption => Display::Block,
        Display::Contents if is_root => Display::Block,
        other => other,
    }
}

/// Resolves stop colors and fills in missing positions (CSS Images §3.5.3):
/// first/last default to 0/1, positions never decrease, and runs of
/// missing positions are spread evenly between their neighbours.
fn fill_stops(stops: &[(SpecColor, Option<f32>)], current: Color) -> Vec<(Color, f32)> {
    let mut pos: Vec<Option<f32>> = stops.iter().map(|(_, p)| *p).collect();
    if let Some(first) = pos.first_mut() {
        first.get_or_insert(0.0);
    }
    if let Some(last) = pos.last_mut() {
        last.get_or_insert(1.0);
    }
    let mut max = f32::NEG_INFINITY;
    for p in pos.iter_mut().flatten() {
        if *p < max {
            *p = max;
        } else {
            max = *p;
        }
    }
    let mut prev: Option<(usize, f32)> = None;
    let mut i = 0;
    while i < pos.len() {
        match pos.get(i).copied().flatten() {
            Some(p) => {
                prev = Some((i, p));
                i += 1;
            }
            None => {
                let (start, from) = prev.unwrap_or((0, 0.0));
                let end = (i..pos.len())
                    .find(|&j| pos.get(j).copied().flatten().is_some())
                    .unwrap_or(pos.len().saturating_sub(1));
                let to = pos.get(end).copied().flatten().unwrap_or(1.0);
                let span = (end - start).max(1) as f32;
                for k in i..end {
                    if let Some(slot) = pos.get_mut(k) {
                        *slot = Some(from + (to - from) * (k - start) as f32 / span);
                    }
                }
                i = end.max(i + 1);
            }
        }
    }
    stops
        .iter()
        .zip(pos)
        .map(|((c, _), p)| (c.resolve(current), p.unwrap_or(0.0).clamp(0.0, 1.0)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_filling() {
        let c = SpecColor::Color(Color::BLACK);
        let got: Vec<f32> = fill_stops(&[(c, None), (c, None), (c, None)], Color::BLACK)
            .into_iter()
            .map(|(_, p)| p)
            .collect();
        assert_eq!(got, vec![0.0, 0.5, 1.0]);
        let got: Vec<f32> = fill_stops(
            &[
                (c, Some(0.2)),
                (c, None),
                (c, None),
                (c, Some(0.8)),
                (c, Some(0.5)),
            ],
            Color::BLACK,
        )
        .into_iter()
        .map(|(_, p)| p)
        .collect();
        let expected = [0.2, 0.4, 0.6, 0.8, 0.8];
        for (g, e) in got.iter().zip(expected) {
            assert!((g - e).abs() < 1e-5, "{got:?}");
        }
        let got: Vec<f32> = fill_stops(&[(c, Some(-0.5)), (c, Some(2.0))], Color::BLACK)
            .into_iter()
            .map(|(_, p)| p)
            .collect();
        assert_eq!(got, vec![0.0, 1.0]);
        assert!(fill_stops(&[], Color::BLACK).is_empty());
    }

    #[test]
    fn blockification_table() {
        assert_eq!(blockify(Display::Inline, false), Display::Block);
        assert_eq!(blockify(Display::InlineFlex, false), Display::Flex);
        assert_eq!(blockify(Display::InlineGrid, false), Display::Grid);
        assert_eq!(blockify(Display::InlineTable, false), Display::Table);
        assert_eq!(blockify(Display::TableCell, false), Display::Block);
        assert_eq!(blockify(Display::ListItem, false), Display::ListItem);
        assert_eq!(blockify(Display::Contents, false), Display::Contents);
        assert_eq!(blockify(Display::Contents, true), Display::Block);
    }

    #[test]
    fn white_space_combinations() {
        assert_eq!(white_space(WsCollapse::Collapse, false), WhiteSpace::Normal);
        assert_eq!(white_space(WsCollapse::Collapse, true), WhiteSpace::Nowrap);
        assert_eq!(white_space(WsCollapse::Preserve, true), WhiteSpace::Pre);
        assert_eq!(
            white_space(WsCollapse::Preserve, false),
            WhiteSpace::PreWrap
        );
        assert_eq!(
            white_space(WsCollapse::PreserveBreaks, false),
            WhiteSpace::PreLine
        );
        assert_eq!(
            white_space(WsCollapse::BreakSpaces, false),
            WhiteSpace::BreakSpaces
        );
    }

    #[test]
    fn monospace_detection() {
        assert!(is_monospace(&["monospace".into()]));
        assert!(is_monospace(&["courier new".into(), "serif".into()]));
        assert!(!is_monospace(&["arial".into(), "sans-serif".into()]));
    }
}
