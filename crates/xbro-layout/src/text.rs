//! Character-level helpers for inline layout: white-space classes,
//! `text-transform`, soft wrap opportunities and list/marker strings.

use xbro_style::TextTransform;

/// Collapsible white space characters (CSS Text 3 "document white space").
#[inline]
pub(crate) fn is_collapsible_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C')
}

/// CJK ideographs, kana, Hangul and full-width forms: soft wrap
/// opportunities exist before and after each of them.
#[inline]
pub(crate) fn is_cjk(c: char) -> bool {
    let u = c as u32;
    matches!(
        u,
        0x2E80..=0x2FFF        // radicals, Kangxi
            | 0x3040..=0x30FF  // hiragana, katakana
            | 0x3100..=0x312F  // bopomofo
            | 0x3130..=0x318F  // Hangul compatibility jamo
            | 0x31A0..=0x31FF
            | 0x3400..=0x4DBF  // CJK extension A
            | 0x4E00..=0x9FFF  // CJK unified ideographs
            | 0xAC00..=0xD7AF  // Hangul syllables
            | 0xF900..=0xFAFF  // compatibility ideographs
            | 0xFF01..=0xFF60  // full-width forms
            | 0x20000..=0x3134F
    )
}

/// CJK punctuation that must not start a line (a subset of the
/// "kinsoku" rules): no break opportunity before it.
#[inline]
pub(crate) fn is_cjk_closing(c: char) -> bool {
    matches!(
        c,
        '\u{3001}'
            | '\u{3002}'
            | '\u{FF0C}'
            | '\u{FF0E}'
            | '\u{FF1A}'
            | '\u{FF1B}'
            | '\u{FF01}'
            | '\u{FF1F}'
            | '\u{300D}'
            | '\u{300F}'
            | '\u{FF09}'
            | '\u{3009}'
            | '\u{300B}'
            | '\u{3011}'
            | '\u{30FC}'
    )
}

/// Dashes after which a line may break (when between letters for `-`).
#[inline]
pub(crate) fn is_break_after_dash(c: char) -> bool {
    matches!(c, '-' | '\u{2010}' | '\u{2013}' | '\u{2014}')
}

/// Appends `c` to `out` transformed per `text-transform`. `word_start` tells
/// whether `c` begins a word (for `capitalize`).
#[inline]
pub(crate) fn push_transformed(out: &mut String, c: char, tt: TextTransform, word_start: bool) {
    match tt {
        TextTransform::None => out.push(c),
        TextTransform::Uppercase => out.extend(c.to_uppercase()),
        TextTransform::Lowercase => out.extend(c.to_lowercase()),
        TextTransform::Capitalize => {
            if word_start {
                out.extend(c.to_uppercase());
            } else {
                out.push(c);
            }
        }
    }
}

/// Characters that keep a word going for `capitalize` (letters, digits,
/// apostrophes inside words).
#[inline]
pub(crate) fn continues_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '\'' | '\u{2019}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        assert!(is_collapsible_space('\n'));
        assert!(!is_collapsible_space('\u{A0}'));
        assert!(is_cjk('日'));
        assert!(is_cjk('한'));
        assert!(!is_cjk('a'));
        assert!(is_cjk_closing('。'));
        assert!(is_break_after_dash('-'));
    }

    #[test]
    fn transforms() {
        let mut s = String::new();
        push_transformed(&mut s, 'ß', TextTransform::Uppercase, false);
        push_transformed(&mut s, 'A', TextTransform::Lowercase, false);
        push_transformed(&mut s, 'b', TextTransform::Capitalize, true);
        push_transformed(&mut s, 'c', TextTransform::Capitalize, false);
        assert_eq!(s, "SSaBc");
    }
}
