//! Character classification for shaping.

/// How a character takes part in shaping.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum CharClass {
    /// Rendered with a glyph (possibly from a fallback face).
    Normal,
    /// Horizontal tab: the glyph and advance of a space.
    Tab,
    /// Produces no glyph and no advance (control characters, default
    /// ignorable code points, line/paragraph separators).
    Ignorable,
}

/// Classifies `c` (fast path for printable ASCII).
#[inline]
pub(crate) fn classify(c: char) -> CharClass {
    let u = c as u32;
    if (0x20..0x7F).contains(&u) {
        return CharClass::Normal;
    }
    if c == '\t' {
        return CharClass::Tab;
    }
    if c.is_control() || is_ignorable(u) {
        return CharClass::Ignorable;
    }
    CharClass::Normal
}

/// Default ignorable code points (Unicode `Default_Ignorable_Code_Point`)
/// plus U+2028/U+2029: invisible, zero-width.
pub(crate) fn is_ignorable(u: u32) -> bool {
    matches!(
        u,
        0x00AD          // soft hyphen
        | 0x034F        // combining grapheme joiner
        | 0x061C        // Arabic letter mark
        | 0x115F..=0x1160 // Hangul fillers
        | 0x17B4..=0x17B5
        | 0x180B..=0x180F // Mongolian variation selectors, vowel separator
        | 0x200B..=0x200F // ZWSP, ZWNJ, ZWJ, LRM, RLM
        | 0x2028..=0x2029 // line / paragraph separators
        | 0x202A..=0x202E // bidi embeddings and overrides
        | 0x2060..=0x206F // word joiner, invisible operators, bidi isolates
        | 0x3164        // Hangul filler
        | 0xFE00..=0xFE0F // variation selectors
        | 0xFEFF        // zero width no-break space / BOM
        | 0xFFA0        // halfwidth Hangul filler
        | 0xFFF0..=0xFFF8
        | 0x1BCA0..=0x1BCA3 // shorthand format controls
        | 0x1D173..=0x1D17A // musical symbol formatting
        | 0xE0000..=0xE0FFF // tags, variation selectors supplement
    )
}

/// True for characters that receive `word-spacing` (U+0020, U+00A0).
#[inline]
pub(crate) fn is_word_separator(c: char) -> bool {
    c == ' ' || c == '\u{A0}'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii() {
        assert_eq!(classify('a'), CharClass::Normal);
        assert_eq!(classify(' '), CharClass::Normal);
        assert_eq!(classify('~'), CharClass::Normal);
        assert_eq!(classify('\t'), CharClass::Tab);
        for c in ['\0', '\n', '\r', '\u{1B}', '\u{7F}', '\u{85}', '\u{9F}'] {
            assert_eq!(classify(c), CharClass::Ignorable, "{:?}", c);
        }
    }

    #[test]
    fn zero_width_characters() {
        for c in [
            '\u{AD}',
            '\u{200B}',
            '\u{200C}',
            '\u{200D}',
            '\u{200E}',
            '\u{200F}',
            '\u{2028}',
            '\u{2029}',
            '\u{FEFF}',
            '\u{FE00}',
            '\u{FE0F}',
            '\u{34F}',
            '\u{2060}',
            '\u{2064}',
            '\u{E0001}',
        ] {
            assert_eq!(classify(c), CharClass::Ignorable, "{:?}", c);
        }
    }

    #[test]
    fn visible_characters() {
        for c in [
            '\u{A0}', 'é', '中', '\u{2003}', '\u{3000}', '😀', '\u{2010}',
        ] {
            assert_eq!(classify(c), CharClass::Normal, "{:?}", c);
        }
    }

    #[test]
    fn word_separators() {
        assert!(is_word_separator(' '));
        assert!(is_word_separator('\u{A0}'));
        assert!(!is_word_separator('\t'));
        assert!(!is_word_separator('\u{3000}'));
    }
}
