//! The registry of supported properties: name → longhands + parser.

use std::collections::HashMap;
use std::sync::OnceLock;

use xbro_css::ComponentValue as CV;

use super::ids::{LonghandId, ALL_LONGHANDS};
use super::{
    background, border, box_model, content, declaration::PropertyDeclaration, flex, font, grid,
    text,
};
use crate::values::ParseCtx;

/// Parses the significant (non-whitespace) items of a value for the given
/// longhands, appending declarations. `None` means invalid.
pub(crate) type ParseFn =
    fn(&[&CV], &[LonghandId], &ParseCtx, &mut Vec<PropertyDeclaration>) -> Option<()>;

/// A supported property.
#[derive(Debug)]
pub(crate) struct PropertyDef {
    pub name: &'static str,
    /// Longhands set by the property (one for a longhand, several for a shorthand).
    pub longhands: &'static [LonghandId],
    pub parse: ParseFn,
    /// Subject to the quirks-mode unitless length quirk.
    pub quirk: bool,
}

use LonghandId as L;

const SIDES_MARGIN: &[L] = &[L::MarginTop, L::MarginRight, L::MarginBottom, L::MarginLeft];
const SIDES_PADDING: &[L] = &[
    L::PaddingTop,
    L::PaddingRight,
    L::PaddingBottom,
    L::PaddingLeft,
];
const SIDES_INSET: &[L] = &[L::Top, L::Right, L::Bottom, L::Left];
const BORDER_WIDTHS: &[L] = &[
    L::BorderTopWidth,
    L::BorderRightWidth,
    L::BorderBottomWidth,
    L::BorderLeftWidth,
];
const BORDER_STYLES: &[L] = &[
    L::BorderTopStyle,
    L::BorderRightStyle,
    L::BorderBottomStyle,
    L::BorderLeftStyle,
];
const BORDER_COLORS: &[L] = &[
    L::BorderTopColor,
    L::BorderRightColor,
    L::BorderBottomColor,
    L::BorderLeftColor,
];
const BORDER_TOP: &[L] = &[L::BorderTopWidth, L::BorderTopStyle, L::BorderTopColor];
const BORDER_RIGHT: &[L] = &[
    L::BorderRightWidth,
    L::BorderRightStyle,
    L::BorderRightColor,
];
const BORDER_BOTTOM: &[L] = &[
    L::BorderBottomWidth,
    L::BorderBottomStyle,
    L::BorderBottomColor,
];
const BORDER_LEFT: &[L] = &[L::BorderLeftWidth, L::BorderLeftStyle, L::BorderLeftColor];
const BORDER_ALL: &[L] = &[
    L::BorderTopWidth,
    L::BorderTopStyle,
    L::BorderTopColor,
    L::BorderRightWidth,
    L::BorderRightStyle,
    L::BorderRightColor,
    L::BorderBottomWidth,
    L::BorderBottomStyle,
    L::BorderBottomColor,
    L::BorderLeftWidth,
    L::BorderLeftStyle,
    L::BorderLeftColor,
];
const BORDER_INLINE: &[L] = &[
    L::BorderLeftWidth,
    L::BorderLeftStyle,
    L::BorderLeftColor,
    L::BorderRightWidth,
    L::BorderRightStyle,
    L::BorderRightColor,
];
const BORDER_BLOCK: &[L] = &[
    L::BorderTopWidth,
    L::BorderTopStyle,
    L::BorderTopColor,
    L::BorderBottomWidth,
    L::BorderBottomStyle,
    L::BorderBottomColor,
];
const RADII: &[L] = &[
    L::BorderTopLeftRadius,
    L::BorderTopRightRadius,
    L::BorderBottomRightRadius,
    L::BorderBottomLeftRadius,
];
const BACKGROUND: &[L] = &[
    L::BackgroundColor,
    L::BackgroundImage,
    L::BackgroundRepeat,
    L::BackgroundSize,
    L::BackgroundPositionX,
    L::BackgroundPositionY,
];
const FONT: &[L] = &[
    L::FontStyle,
    L::FontWeight,
    L::FontSize,
    L::LineHeight,
    L::FontFamily,
];

const GRID_TEMPLATES: &[L] = &[L::GridTemplateRows, L::GridTemplateColumns];
/// `grid-area` order: row-start / column-start / row-end / column-end.
const GRID_AREA: &[L] = &[
    L::GridRowStart,
    L::GridColumnStart,
    L::GridRowEnd,
    L::GridColumnEnd,
];

macro_rules! defs {
    ($( $name:literal => $longhands:expr, $parse:path $(, $quirk:ident)? ;)*) => {
        &[$(PropertyDef {
            name: $name,
            longhands: $longhands,
            parse: $parse,
            quirk: defs!(@quirk $($quirk)?),
        }),*]
    };
    (@quirk quirk) => { true };
    (@quirk) => { false };
}

/// Every supported property.
pub(crate) static PROPERTIES: &[PropertyDef] = defs! {
    // ---- box generation & positioning ----
    "display" => &[L::Display], box_model::display;
    "position" => &[L::Position], box_model::position;
    "top" => &[L::Top], box_model::inset, quirk;
    "right" => &[L::Right], box_model::inset, quirk;
    "bottom" => &[L::Bottom], box_model::inset, quirk;
    "left" => &[L::Left], box_model::inset, quirk;
    "inset" => SIDES_INSET, box_model::inset;
    "inset-inline" => &[L::Left, L::Right], box_model::inset;
    "inset-block" => &[L::Top, L::Bottom], box_model::inset;
    "inset-inline-start" => &[L::Left], box_model::inset;
    "inset-inline-end" => &[L::Right], box_model::inset;
    "inset-block-start" => &[L::Top], box_model::inset;
    "inset-block-end" => &[L::Bottom], box_model::inset;
    "z-index" => &[L::ZIndex], box_model::z_index;
    "float" => &[L::Float], box_model::float;
    "clear" => &[L::Clear], box_model::clear;
    "box-sizing" => &[L::BoxSizing], box_model::box_sizing;
    "-webkit-box-sizing" => &[L::BoxSizing], box_model::box_sizing;
    "-moz-box-sizing" => &[L::BoxSizing], box_model::box_sizing;
    // ---- sizes ----
    "width" => &[L::Width], box_model::size, quirk;
    "height" => &[L::Height], box_model::size, quirk;
    "min-width" => &[L::MinWidth], box_model::size, quirk;
    "min-height" => &[L::MinHeight], box_model::size, quirk;
    "max-width" => &[L::MaxWidth], box_model::max_size, quirk;
    "max-height" => &[L::MaxHeight], box_model::max_size, quirk;
    "inline-size" => &[L::Width], box_model::size;
    "block-size" => &[L::Height], box_model::size;
    "min-inline-size" => &[L::MinWidth], box_model::size;
    "min-block-size" => &[L::MinHeight], box_model::size;
    "max-inline-size" => &[L::MaxWidth], box_model::max_size;
    "max-block-size" => &[L::MaxHeight], box_model::max_size;
    // ---- margins & paddings ----
    "margin" => SIDES_MARGIN, box_model::margin, quirk;
    "margin-top" => &[L::MarginTop], box_model::margin, quirk;
    "margin-right" => &[L::MarginRight], box_model::margin, quirk;
    "margin-bottom" => &[L::MarginBottom], box_model::margin, quirk;
    "margin-left" => &[L::MarginLeft], box_model::margin, quirk;
    "margin-inline" => &[L::MarginLeft, L::MarginRight], box_model::margin;
    "margin-block" => &[L::MarginTop, L::MarginBottom], box_model::margin;
    "margin-inline-start" => &[L::MarginLeft], box_model::margin;
    "margin-inline-end" => &[L::MarginRight], box_model::margin;
    "margin-block-start" => &[L::MarginTop], box_model::margin;
    "margin-block-end" => &[L::MarginBottom], box_model::margin;
    "-webkit-margin-start" => &[L::MarginLeft], box_model::margin;
    "-webkit-margin-end" => &[L::MarginRight], box_model::margin;
    "-webkit-margin-before" => &[L::MarginTop], box_model::margin;
    "-webkit-margin-after" => &[L::MarginBottom], box_model::margin;
    "padding" => SIDES_PADDING, box_model::padding, quirk;
    "padding-top" => &[L::PaddingTop], box_model::padding, quirk;
    "padding-right" => &[L::PaddingRight], box_model::padding, quirk;
    "padding-bottom" => &[L::PaddingBottom], box_model::padding, quirk;
    "padding-left" => &[L::PaddingLeft], box_model::padding, quirk;
    "padding-inline" => &[L::PaddingLeft, L::PaddingRight], box_model::padding;
    "padding-block" => &[L::PaddingTop, L::PaddingBottom], box_model::padding;
    "padding-inline-start" => &[L::PaddingLeft], box_model::padding;
    "padding-inline-end" => &[L::PaddingRight], box_model::padding;
    "padding-block-start" => &[L::PaddingTop], box_model::padding;
    "padding-block-end" => &[L::PaddingBottom], box_model::padding;
    "-webkit-padding-start" => &[L::PaddingLeft], box_model::padding;
    "-webkit-padding-end" => &[L::PaddingRight], box_model::padding;
    "-webkit-padding-before" => &[L::PaddingTop], box_model::padding;
    "-webkit-padding-after" => &[L::PaddingBottom], box_model::padding;
    // ---- overflow & misc box ----
    "overflow" => &[L::OverflowX, L::OverflowY], box_model::overflow;
    "overflow-x" => &[L::OverflowX], box_model::overflow;
    "overflow-y" => &[L::OverflowY], box_model::overflow;
    "overflow-inline" => &[L::OverflowX], box_model::overflow;
    "overflow-block" => &[L::OverflowY], box_model::overflow;
    "aspect-ratio" => &[L::AspectRatio], box_model::aspect_ratio;
    "visibility" => &[L::Visibility], box_model::visibility;
    "opacity" => &[L::Opacity], box_model::opacity;
    // ---- borders ----
    "border" => BORDER_ALL, border::border_shorthand;
    "border-top" => BORDER_TOP, border::border_shorthand;
    "border-right" => BORDER_RIGHT, border::border_shorthand;
    "border-bottom" => BORDER_BOTTOM, border::border_shorthand;
    "border-left" => BORDER_LEFT, border::border_shorthand;
    "border-inline" => BORDER_INLINE, border::border_shorthand;
    "border-block" => BORDER_BLOCK, border::border_shorthand;
    "border-inline-start" => BORDER_LEFT, border::border_shorthand;
    "border-inline-end" => BORDER_RIGHT, border::border_shorthand;
    "border-block-start" => BORDER_TOP, border::border_shorthand;
    "border-block-end" => BORDER_BOTTOM, border::border_shorthand;
    "border-width" => BORDER_WIDTHS, border::border_width, quirk;
    "border-style" => BORDER_STYLES, border::border_style;
    "border-color" => BORDER_COLORS, border::border_color;
    "border-top-width" => &[L::BorderTopWidth], border::border_width, quirk;
    "border-right-width" => &[L::BorderRightWidth], border::border_width, quirk;
    "border-bottom-width" => &[L::BorderBottomWidth], border::border_width, quirk;
    "border-left-width" => &[L::BorderLeftWidth], border::border_width, quirk;
    "border-top-style" => &[L::BorderTopStyle], border::border_style;
    "border-right-style" => &[L::BorderRightStyle], border::border_style;
    "border-bottom-style" => &[L::BorderBottomStyle], border::border_style;
    "border-left-style" => &[L::BorderLeftStyle], border::border_style;
    "border-top-color" => &[L::BorderTopColor], border::border_color;
    "border-right-color" => &[L::BorderRightColor], border::border_color;
    "border-bottom-color" => &[L::BorderBottomColor], border::border_color;
    "border-left-color" => &[L::BorderLeftColor], border::border_color;
    "border-inline-width" => &[L::BorderLeftWidth, L::BorderRightWidth], border::border_width;
    "border-inline-style" => &[L::BorderLeftStyle, L::BorderRightStyle], border::border_style;
    "border-inline-color" => &[L::BorderLeftColor, L::BorderRightColor], border::border_color;
    "border-block-width" => &[L::BorderTopWidth, L::BorderBottomWidth], border::border_width;
    "border-block-style" => &[L::BorderTopStyle, L::BorderBottomStyle], border::border_style;
    "border-block-color" => &[L::BorderTopColor, L::BorderBottomColor], border::border_color;
    "border-inline-start-width" => &[L::BorderLeftWidth], border::border_width;
    "border-inline-start-style" => &[L::BorderLeftStyle], border::border_style;
    "border-inline-start-color" => &[L::BorderLeftColor], border::border_color;
    "border-inline-end-width" => &[L::BorderRightWidth], border::border_width;
    "border-inline-end-style" => &[L::BorderRightStyle], border::border_style;
    "border-inline-end-color" => &[L::BorderRightColor], border::border_color;
    "border-block-start-width" => &[L::BorderTopWidth], border::border_width;
    "border-block-start-style" => &[L::BorderTopStyle], border::border_style;
    "border-block-start-color" => &[L::BorderTopColor], border::border_color;
    "border-block-end-width" => &[L::BorderBottomWidth], border::border_width;
    "border-block-end-style" => &[L::BorderBottomStyle], border::border_style;
    "border-block-end-color" => &[L::BorderBottomColor], border::border_color;
    "border-radius" => RADII, border::border_radius;
    "-webkit-border-radius" => RADII, border::border_radius;
    "-moz-border-radius" => RADII, border::border_radius;
    "border-top-left-radius" => &[L::BorderTopLeftRadius], border::corner_radius;
    "border-top-right-radius" => &[L::BorderTopRightRadius], border::corner_radius;
    "border-bottom-right-radius" => &[L::BorderBottomRightRadius], border::corner_radius;
    "border-bottom-left-radius" => &[L::BorderBottomLeftRadius], border::corner_radius;
    "border-start-start-radius" => &[L::BorderTopLeftRadius], border::corner_radius;
    "border-start-end-radius" => &[L::BorderTopRightRadius], border::corner_radius;
    "border-end-end-radius" => &[L::BorderBottomRightRadius], border::corner_radius;
    "border-end-start-radius" => &[L::BorderBottomLeftRadius], border::corner_radius;
    "outline" => &[L::OutlineWidth, L::OutlineStyle, L::OutlineColor], border::outline;
    "outline-width" => &[L::OutlineWidth], border::outline_width;
    "outline-style" => &[L::OutlineStyle], border::outline_style;
    "outline-color" => &[L::OutlineColor], border::outline_color;
    // ---- background ----
    "background" => BACKGROUND, background::background;
    "background-color" => &[L::BackgroundColor], background::background_color;
    "background-image" => &[L::BackgroundImage], background::background_image;
    "background-repeat" => &[L::BackgroundRepeat], background::background_repeat;
    "background-size" => &[L::BackgroundSize], background::background_size;
    "background-position" => &[L::BackgroundPositionX, L::BackgroundPositionY], background::background_position, quirk;
    "background-position-x" => &[L::BackgroundPositionX], background::background_position_axis;
    "background-position-y" => &[L::BackgroundPositionY], background::background_position_axis;
    // ---- fonts ----
    "font" => FONT, font::font;
    "font-family" => &[L::FontFamily], font::font_family;
    "font-size" => &[L::FontSize], font::font_size, quirk;
    "font-weight" => &[L::FontWeight], font::font_weight;
    "font-style" => &[L::FontStyle], font::font_style;
    "line-height" => &[L::LineHeight], font::line_height;
    // ---- text ----
    "color" => &[L::Color], text::color;
    "text-align" => &[L::TextAlign], text::text_align;
    "text-indent" => &[L::TextIndent], text::text_indent, quirk;
    "text-transform" => &[L::TextTransform], text::text_transform;
    "letter-spacing" => &[L::LetterSpacing], text::spacing, quirk;
    "word-spacing" => &[L::WordSpacing], text::spacing, quirk;
    "white-space" => &[L::WhiteSpaceCollapse, L::TextWrapMode], text::white_space;
    "white-space-collapse" => &[L::WhiteSpaceCollapse], text::white_space_collapse;
    "text-wrap-mode" => &[L::TextWrapMode], text::text_wrap;
    "text-wrap" => &[L::TextWrapMode], text::text_wrap;
    "overflow-wrap" => &[L::OverflowWrap], text::overflow_wrap;
    "word-wrap" => &[L::OverflowWrap], text::overflow_wrap;
    "word-break" => &[L::WordBreak], text::word_break;
    "text-decoration" => &[L::TextDecorationLine, L::TextDecorationColor], text::text_decoration;
    "-webkit-text-decoration" => &[L::TextDecorationLine, L::TextDecorationColor], text::text_decoration;
    "text-decoration-line" => &[L::TextDecorationLine], text::text_decoration_line;
    "-webkit-text-decoration-line" => &[L::TextDecorationLine], text::text_decoration_line;
    "text-decoration-color" => &[L::TextDecorationColor], text::text_decoration_color;
    "-webkit-text-decoration-color" => &[L::TextDecorationColor], text::text_decoration_color;
    "vertical-align" => &[L::VerticalAlign], text::vertical_align, quirk;
    "border-collapse" => &[L::BorderCollapse], text::border_collapse;
    "border-spacing" => &[L::BorderSpacing], text::border_spacing, quirk;
    "caption-side" => &[L::CaptionSide], text::caption_side;
    "table-layout" => &[L::TableLayout], text::table_layout;
    // ---- generated content, lists, cursor ----
    "content" => &[L::Content], content::content;
    "cursor" => &[L::Cursor], content::cursor;
    "list-style" => &[L::ListStyleType, L::ListStylePosition, L::ListStyleImage], content::list_style;
    "list-style-type" => &[L::ListStyleType], content::list_style_type;
    "list-style-position" => &[L::ListStylePosition], content::list_style_position;
    "list-style-image" => &[L::ListStyleImage], content::list_style_image;
    // ---- flexbox & grid ----
    "flex" => &[L::FlexGrow, L::FlexShrink, L::FlexBasis], flex::flex;
    "-webkit-flex" => &[L::FlexGrow, L::FlexShrink, L::FlexBasis], flex::flex;
    "-ms-flex" => &[L::FlexGrow, L::FlexShrink, L::FlexBasis], flex::flex;
    "flex-grow" => &[L::FlexGrow], flex::flex_factor;
    "-webkit-flex-grow" => &[L::FlexGrow], flex::flex_factor;
    "flex-shrink" => &[L::FlexShrink], flex::flex_factor;
    "-webkit-flex-shrink" => &[L::FlexShrink], flex::flex_factor;
    "flex-basis" => &[L::FlexBasis], flex::flex_basis;
    "-webkit-flex-basis" => &[L::FlexBasis], flex::flex_basis;
    "flex-direction" => &[L::FlexDirection], flex::flex_direction;
    "-webkit-flex-direction" => &[L::FlexDirection], flex::flex_direction;
    "flex-wrap" => &[L::FlexWrap], flex::flex_wrap;
    "-webkit-flex-wrap" => &[L::FlexWrap], flex::flex_wrap;
    "flex-flow" => &[L::FlexDirection, L::FlexWrap], flex::flex_flow;
    "-webkit-flex-flow" => &[L::FlexDirection, L::FlexWrap], flex::flex_flow;
    "order" => &[L::Order], flex::order;
    "-webkit-order" => &[L::Order], flex::order;
    "justify-content" => &[L::JustifyContent], flex::justify_content;
    "-webkit-justify-content" => &[L::JustifyContent], flex::justify_content;
    "align-content" => &[L::AlignContent], flex::align_content;
    "-webkit-align-content" => &[L::AlignContent], flex::align_content;
    "align-items" => &[L::AlignItems], flex::align_items;
    "-webkit-align-items" => &[L::AlignItems], flex::align_items;
    "align-self" => &[L::AlignSelf], flex::align_self;
    "-webkit-align-self" => &[L::AlignSelf], flex::align_self;
    "place-content" => &[L::AlignContent, L::JustifyContent], flex::place_content;
    "place-items" => &[L::AlignItems], flex::place_items;
    "place-self" => &[L::AlignSelf], flex::place_self;
    "gap" => &[L::RowGap, L::ColumnGap], flex::gap;
    "grid-gap" => &[L::RowGap, L::ColumnGap], flex::gap;
    "row-gap" => &[L::RowGap], flex::gap;
    "grid-row-gap" => &[L::RowGap], flex::gap;
    "column-gap" => &[L::ColumnGap], flex::gap;
    "grid-column-gap" => &[L::ColumnGap], flex::gap;
    "grid-template-columns" => &[L::GridTemplateColumns], grid::grid_template_tracks;
    "grid-template-rows" => &[L::GridTemplateRows], grid::grid_template_tracks;
    "grid-template" => GRID_TEMPLATES, grid::grid_template;
    "grid" => GRID_TEMPLATES, grid::grid;
    "grid-row-start" => &[L::GridRowStart], grid::grid_placement;
    "grid-row-end" => &[L::GridRowEnd], grid::grid_placement;
    "grid-column-start" => &[L::GridColumnStart], grid::grid_placement;
    "grid-column-end" => &[L::GridColumnEnd], grid::grid_placement;
    "grid-row" => &[L::GridRowStart, L::GridRowEnd], grid::grid_placement;
    "grid-column" => &[L::GridColumnStart, L::GridColumnEnd], grid::grid_placement;
    "grid-area" => GRID_AREA, grid::grid_placement;
    // ---- all ----
    "all" => ALL_LONGHANDS, only_css_wide;
};

/// `all` only accepts CSS-wide keywords (handled before the parser is called).
fn only_css_wide(
    _: &[&CV],
    _: &[LonghandId],
    _: &ParseCtx,
    _: &mut Vec<PropertyDeclaration>,
) -> Option<()> {
    None
}

/// Looks up a property by its (lowercase) name.
pub(crate) fn lookup(name: &str) -> Option<&'static PropertyDef> {
    static MAP: OnceLock<HashMap<&'static str, &'static PropertyDef>> = OnceLock::new();
    let map = MAP.get_or_init(|| PROPERTIES.iter().map(|d| (d.name, d)).collect());
    if let Some(def) = map.get(name) {
        return Some(def);
    }
    if name.bytes().any(|b| b.is_ascii_uppercase()) {
        return map.get(name.to_ascii_lowercase().as_str()).copied();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_lowercase() {
        let mut seen = std::collections::HashSet::new();
        for def in PROPERTIES {
            assert!(seen.insert(def.name), "duplicate {}", def.name);
            assert_eq!(def.name, def.name.to_ascii_lowercase());
            assert!(!def.longhands.is_empty());
        }
    }

    #[test]
    fn every_longhand_is_reachable() {
        for id in ALL_LONGHANDS {
            assert!(
                PROPERTIES
                    .iter()
                    .any(|d| d.name != "all" && d.longhands.contains(id)),
                "{id:?} has no property"
            );
        }
    }

    #[test]
    fn known_by_css_supports_table() {
        // Properties xbro-css reports as supported should mostly be parsed here.
        for name in [
            "display",
            "margin",
            "flex",
            "background",
            "font",
            "border-radius",
        ] {
            assert!(lookup(name).is_some());
            assert!(xbro_css::is_supported_property(name));
        }
        assert!(lookup("MARGIN").is_some());
        assert!(lookup("transform").is_none());
    }
}
