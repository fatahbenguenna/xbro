//! Longhand property identifiers.
//!
//! Longhands mirror the fields of [`ComputedStyle`](crate::ComputedStyle)
//! (physical sides only: logical properties are mapped to physical ones for
//! horizontal left-to-right text at parse time), plus a few internal
//! longhands whose computed values are combined into one field
//! (`white-space-collapse` + `text-wrap-mode` → `white_space`,
//! `overflow-wrap` + `word-break` → `break_anywhere`, `outline-style`).

macro_rules! longhands {
    ($($name:ident),* $(,)?) => {
        /// A longhand property.
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u8)]
        pub(crate) enum LonghandId {
            $($name),*
        }

        /// Every longhand, in declaration order (`ALL[id as usize] == id`).
        pub(crate) const ALL_LONGHANDS: &[LonghandId] = &[$(LonghandId::$name),*];
    };
}

longhands!(
    Display,
    Position,
    Top,
    Right,
    Bottom,
    Left,
    ZIndex,
    Float,
    Clear,
    BoxSizing,
    Width,
    Height,
    MinWidth,
    MinHeight,
    MaxWidth,
    MaxHeight,
    MarginTop,
    MarginRight,
    MarginBottom,
    MarginLeft,
    PaddingTop,
    PaddingRight,
    PaddingBottom,
    PaddingLeft,
    BorderTopWidth,
    BorderRightWidth,
    BorderBottomWidth,
    BorderLeftWidth,
    BorderTopStyle,
    BorderRightStyle,
    BorderBottomStyle,
    BorderLeftStyle,
    BorderTopColor,
    BorderRightColor,
    BorderBottomColor,
    BorderLeftColor,
    BorderTopLeftRadius,
    BorderTopRightRadius,
    BorderBottomRightRadius,
    BorderBottomLeftRadius,
    OverflowX,
    OverflowY,
    AspectRatio,
    Visibility,
    Opacity,
    BackgroundColor,
    BackgroundImage,
    BackgroundRepeat,
    BackgroundSize,
    BackgroundPositionX,
    BackgroundPositionY,
    OutlineWidth,
    OutlineStyle,
    OutlineColor,
    Cursor,
    Color,
    FontFamily,
    FontSize,
    FontWeight,
    FontStyle,
    LineHeight,
    TextAlign,
    TextIndent,
    TextTransform,
    LetterSpacing,
    WordSpacing,
    WhiteSpaceCollapse,
    TextWrapMode,
    OverflowWrap,
    WordBreak,
    ListStyleType,
    ListStylePosition,
    ListStyleImage,
    BorderCollapse,
    BorderSpacing,
    TextDecorationLine,
    TextDecorationColor,
    VerticalAlign,
    FlexDirection,
    FlexWrap,
    JustifyContent,
    AlignItems,
    AlignSelf,
    AlignContent,
    FlexGrow,
    FlexShrink,
    FlexBasis,
    Order,
    RowGap,
    ColumnGap,
    GridColumns,
    Content,
);

/// Number of longhands.
pub(crate) const LONGHAND_COUNT: usize = ALL_LONGHANDS.len();

impl LonghandId {
    pub fn index(self) -> usize {
        self as usize
    }

    /// Inherited properties: exactly those copied by
    /// [`ComputedStyle::inherit_from`](crate::ComputedStyle::inherit_from).
    pub fn is_inherited(self) -> bool {
        use LonghandId::*;
        matches!(
            self,
            Color
                | FontFamily
                | FontSize
                | FontWeight
                | FontStyle
                | LineHeight
                | TextAlign
                | TextIndent
                | TextTransform
                | LetterSpacing
                | WordSpacing
                | WhiteSpaceCollapse
                | TextWrapMode
                | OverflowWrap
                | WordBreak
                | ListStyleType
                | ListStylePosition
                | ListStyleImage
                | BorderCollapse
                | BorderSpacing
                | Visibility
                | Cursor
        )
    }

    /// Sizes whose computed value may not be negative (`calc()` results are clamped).
    pub fn is_non_negative(self) -> bool {
        use LonghandId::*;
        matches!(
            self,
            Width
                | Height
                | MinWidth
                | MinHeight
                | MaxWidth
                | MaxHeight
                | PaddingTop
                | PaddingRight
                | PaddingBottom
                | PaddingLeft
                | BorderTopLeftRadius
                | BorderTopRightRadius
                | BorderBottomRightRadius
                | BorderBottomLeftRadius
                | RowGap
                | ColumnGap
                | FlexBasis
                | BackgroundSize
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_match_table() {
        for (i, id) in ALL_LONGHANDS.iter().enumerate() {
            assert_eq!(id.index(), i);
        }
        const { assert!(LONGHAND_COUNT < 256) };
    }

    #[test]
    fn inherited_set_matches_inherit_from() {
        use crate::computed::*;
        // Changing an inherited field on the parent must show up in
        // `inherit_from`, and vice versa for non-inherited ones.
        let parent = ComputedStyle {
            color: Color::rgb(1, 2, 3),
            width: LengthPercentageAuto::px(5.0),
            visibility: Visibility::Hidden,
            cursor: Cursor::Pointer,
            ..ComputedStyle::default()
        };
        let child = ComputedStyle::inherit_from(&parent);
        assert_eq!(child.color, parent.color);
        assert_eq!(child.visibility, Visibility::Hidden);
        assert_eq!(child.cursor, Cursor::Pointer);
        assert!(child.width.is_auto());
        assert!(LonghandId::Color.is_inherited());
        assert!(LonghandId::Visibility.is_inherited());
        assert!(LonghandId::Cursor.is_inherited());
        assert!(!LonghandId::Width.is_inherited());
        assert!(!LonghandId::TextDecorationLine.is_inherited());
        assert!(!LonghandId::VerticalAlign.is_inherited());
    }
}
