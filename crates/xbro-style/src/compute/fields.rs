//! Mapping between longhands and [`ComputedStyle`] fields.

use crate::computed::*;
use crate::properties::LonghandId;

/// The `LengthPercentageAuto` field of a longhand.
pub(super) fn lpa_field(
    s: &mut ComputedStyle,
    id: LonghandId,
) -> Option<&mut LengthPercentageAuto> {
    use LonghandId as L;
    Some(match id {
        L::Top => &mut s.top,
        L::Right => &mut s.right,
        L::Bottom => &mut s.bottom,
        L::Left => &mut s.left,
        L::Width => &mut s.width,
        L::Height => &mut s.height,
        L::MinWidth => &mut s.min_width,
        L::MinHeight => &mut s.min_height,
        L::MarginTop => &mut s.margin.top,
        L::MarginRight => &mut s.margin.right,
        L::MarginBottom => &mut s.margin.bottom,
        L::MarginLeft => &mut s.margin.left,
        L::FlexBasis => &mut s.flex_basis,
        _ => return None,
    })
}

/// The `LengthPercentage` field of a longhand.
pub(super) fn lp_field(s: &mut ComputedStyle, id: LonghandId) -> Option<&mut LengthPercentage> {
    use LonghandId as L;
    Some(match id {
        L::PaddingTop => &mut s.padding.top,
        L::PaddingRight => &mut s.padding.right,
        L::PaddingBottom => &mut s.padding.bottom,
        L::PaddingLeft => &mut s.padding.left,
        L::TextIndent => &mut s.text_indent,
        L::RowGap => &mut s.row_gap,
        L::ColumnGap => &mut s.column_gap,
        L::BorderTopLeftRadius => &mut s.border_radius[0],
        L::BorderTopRightRadius => &mut s.border_radius[1],
        L::BorderBottomRightRadius => &mut s.border_radius[2],
        L::BorderBottomLeftRadius => &mut s.border_radius[3],
        L::BackgroundPositionX => &mut s.background_position.0,
        L::BackgroundPositionY => &mut s.background_position.1,
        _ => return None,
    })
}

/// Color fields whose `currentcolor` resolves to the element's own color.
pub(super) fn color_field(s: &mut ComputedStyle, id: LonghandId) -> Option<&mut Color> {
    use LonghandId as L;
    Some(match id {
        L::BorderTopColor => &mut s.border.top.color,
        L::BorderRightColor => &mut s.border.right.color,
        L::BorderBottomColor => &mut s.border.bottom.color,
        L::BorderLeftColor => &mut s.border.left.color,
        L::OutlineColor => &mut s.outline_color,
        L::BackgroundColor => &mut s.background_color,
        _ => return None,
    })
}

/// Index (top, right, bottom, left) of a border width longhand.
pub(super) fn border_width_index(id: LonghandId) -> Option<usize> {
    use LonghandId as L;
    match id {
        L::BorderTopWidth => Some(0),
        L::BorderRightWidth => Some(1),
        L::BorderBottomWidth => Some(2),
        L::BorderLeftWidth => Some(3),
        _ => None,
    }
}

pub(super) fn border_style_field(
    s: &mut ComputedStyle,
    id: LonghandId,
) -> Option<&mut BorderStyle> {
    use LonghandId as L;
    Some(match id {
        L::BorderTopStyle => &mut s.border.top.style,
        L::BorderRightStyle => &mut s.border.right.style,
        L::BorderBottomStyle => &mut s.border.bottom.style,
        L::BorderLeftStyle => &mut s.border.left.style,
        _ => return None,
    })
}

/// Copies the computed value of `id` from `src` to `dst`. Longhands stored
/// outside `ComputedStyle` (border/outline widths before the style
/// adjustment, outline style, white-space and break parts) are handled by
/// the caller.
pub(super) fn copy_field(id: LonghandId, dst: &mut ComputedStyle, src: &ComputedStyle) {
    use LonghandId as L;
    match id {
        L::Display => dst.display = src.display,
        L::Position => dst.position = src.position,
        L::Top => dst.top = src.top,
        L::Right => dst.right = src.right,
        L::Bottom => dst.bottom = src.bottom,
        L::Left => dst.left = src.left,
        L::ZIndex => dst.z_index = src.z_index,
        L::Float => dst.float = src.float,
        L::Clear => dst.clear = src.clear,
        L::BoxSizing => dst.box_sizing = src.box_sizing,
        L::Width => dst.width = src.width,
        L::Height => dst.height = src.height,
        L::MinWidth => dst.min_width = src.min_width,
        L::MinHeight => dst.min_height = src.min_height,
        L::MaxWidth => dst.max_width = src.max_width,
        L::MaxHeight => dst.max_height = src.max_height,
        L::MarginTop => dst.margin.top = src.margin.top,
        L::MarginRight => dst.margin.right = src.margin.right,
        L::MarginBottom => dst.margin.bottom = src.margin.bottom,
        L::MarginLeft => dst.margin.left = src.margin.left,
        L::PaddingTop => dst.padding.top = src.padding.top,
        L::PaddingRight => dst.padding.right = src.padding.right,
        L::PaddingBottom => dst.padding.bottom = src.padding.bottom,
        L::PaddingLeft => dst.padding.left = src.padding.left,
        L::BorderTopWidth => dst.border.top.width = src.border.top.width,
        L::BorderRightWidth => dst.border.right.width = src.border.right.width,
        L::BorderBottomWidth => dst.border.bottom.width = src.border.bottom.width,
        L::BorderLeftWidth => dst.border.left.width = src.border.left.width,
        L::BorderTopStyle => dst.border.top.style = src.border.top.style,
        L::BorderRightStyle => dst.border.right.style = src.border.right.style,
        L::BorderBottomStyle => dst.border.bottom.style = src.border.bottom.style,
        L::BorderLeftStyle => dst.border.left.style = src.border.left.style,
        L::BorderTopColor => dst.border.top.color = src.border.top.color,
        L::BorderRightColor => dst.border.right.color = src.border.right.color,
        L::BorderBottomColor => dst.border.bottom.color = src.border.bottom.color,
        L::BorderLeftColor => dst.border.left.color = src.border.left.color,
        L::BorderTopLeftRadius => dst.border_radius[0] = src.border_radius[0],
        L::BorderTopRightRadius => dst.border_radius[1] = src.border_radius[1],
        L::BorderBottomRightRadius => dst.border_radius[2] = src.border_radius[2],
        L::BorderBottomLeftRadius => dst.border_radius[3] = src.border_radius[3],
        L::OverflowX => dst.overflow_x = src.overflow_x,
        L::OverflowY => dst.overflow_y = src.overflow_y,
        L::AspectRatio => dst.aspect_ratio = src.aspect_ratio,
        L::Visibility => dst.visibility = src.visibility,
        L::Opacity => dst.opacity = src.opacity,
        L::BackgroundColor => dst.background_color = src.background_color,
        L::BackgroundImage => dst.background_image = src.background_image.clone(),
        L::BackgroundRepeat => dst.background_repeat = src.background_repeat,
        L::BackgroundSize => dst.background_size = src.background_size,
        L::BackgroundPositionX => dst.background_position.0 = src.background_position.0,
        L::BackgroundPositionY => dst.background_position.1 = src.background_position.1,
        L::OutlineWidth => dst.outline_width = src.outline_width,
        L::OutlineStyle => {}
        L::OutlineColor => dst.outline_color = src.outline_color,
        L::Cursor => dst.cursor = src.cursor,
        L::Color => dst.color = src.color,
        L::FontFamily => dst.font_family = src.font_family.clone(),
        L::FontSize => dst.font_size = src.font_size,
        L::FontWeight => dst.font_weight = src.font_weight,
        L::FontStyle => dst.font_style = src.font_style,
        L::LineHeight => dst.line_height = src.line_height,
        L::TextAlign => dst.text_align = src.text_align,
        L::TextIndent => dst.text_indent = src.text_indent,
        L::TextTransform => dst.text_transform = src.text_transform,
        L::LetterSpacing => dst.letter_spacing = src.letter_spacing,
        L::WordSpacing => dst.word_spacing = src.word_spacing,
        L::WhiteSpaceCollapse | L::TextWrapMode | L::OverflowWrap | L::WordBreak => {}
        L::ListStyleType => dst.list_style_type = src.list_style_type,
        L::ListStylePosition => dst.list_style_position = src.list_style_position,
        L::ListStyleImage => dst.list_style_image = src.list_style_image.clone(),
        L::BorderCollapse => dst.border_collapse = src.border_collapse,
        L::BorderSpacing => dst.border_spacing = src.border_spacing,
        L::TextDecorationLine => dst.text_decoration_line = src.text_decoration_line,
        L::TextDecorationColor => dst.text_decoration_color = src.text_decoration_color,
        L::VerticalAlign => dst.vertical_align = src.vertical_align,
        L::FlexDirection => dst.flex_direction = src.flex_direction,
        L::FlexWrap => dst.flex_wrap = src.flex_wrap,
        L::JustifyContent => dst.justify_content = src.justify_content,
        L::AlignItems => dst.align_items = src.align_items,
        L::AlignSelf => dst.align_self = src.align_self,
        L::AlignContent => dst.align_content = src.align_content,
        L::FlexGrow => dst.flex_grow = src.flex_grow,
        L::FlexShrink => dst.flex_shrink = src.flex_shrink,
        L::FlexBasis => dst.flex_basis = src.flex_basis,
        L::Order => dst.order = src.order,
        L::RowGap => dst.row_gap = src.row_gap,
        L::ColumnGap => dst.column_gap = src.column_gap,
        L::GridColumns => dst.grid_columns = src.grid_columns,
        L::Content => dst.content = src.content.clone(),
    }
}
