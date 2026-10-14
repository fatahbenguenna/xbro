//! HTML "rules for parsing a legacy colour value" (HTML §2.3.6).

use super::Color;

fn hex_value(c: char) -> u8 {
    c.to_digit(16).map_or(0, |d| d as u8)
}

pub(super) fn parse_legacy(input: &str) -> Option<Color> {
    // Only the empty string is an error; a whitespace-only value is stripped
    // to "" and then padded to "000" below (black), as the spec says.
    if input.is_empty() {
        return None;
    }
    // Strip leading and trailing ASCII whitespace.
    let input = input.trim_matches(|c: char| c.is_ascii_whitespace());
    if input.eq_ignore_ascii_case("transparent") {
        return None;
    }
    if let Some(color) = Color::named(input) {
        return Some(color);
    }
    // `#rgb`. Later steps only look at the first 128 code points, so
    // longer inputs are not collected in full.
    let chars: Vec<char> = input.chars().take(129).collect();
    if let ['#', r, g, b] = chars.as_slice() {
        if r.is_ascii_hexdigit() && g.is_ascii_hexdigit() && b.is_ascii_hexdigit() {
            return Some(Color::rgb(
                hex_value(*r) * 17,
                hex_value(*g) * 17,
                hex_value(*b) * 17,
            ));
        }
    }
    // Code points above U+FFFF count as two characters ("00").
    let mut digits: Vec<char> = Vec::with_capacity(chars.len().min(130));
    for c in chars {
        if digits.len() >= 128 {
            break;
        }
        if c as u32 > 0xFFFF {
            digits.push('0');
            digits.push('0');
        } else {
            digits.push(c);
        }
    }
    digits.truncate(128);
    if digits.first() == Some(&'#') {
        digits.remove(0);
    }
    for d in digits.iter_mut() {
        if !d.is_ascii_hexdigit() {
            *d = '0';
        }
    }
    while digits.is_empty() || digits.len() % 3 != 0 {
        digits.push('0');
    }
    let mut length = digits.len() / 3;
    let mut components: Vec<&[char]> = digits.chunks(length).collect();
    if length > 8 {
        let skip = length - 8;
        for component in components.iter_mut() {
            *component = component.get(skip..).unwrap_or(&[]);
        }
        length = 8;
    }
    while length > 2 && components.iter().all(|c| c.first() == Some(&'0')) {
        for component in components.iter_mut() {
            *component = component.get(1..).unwrap_or(&[]);
        }
        length -= 1;
    }
    let channel = |component: &[char]| -> u8 {
        let pair = component.get(..length.min(2)).unwrap_or(&[]);
        pair.iter().fold(0u8, |acc, c| {
            acc.wrapping_mul(16).wrapping_add(hex_value(*c))
        })
    };
    Some(Color::rgb(
        channel(components.first()?),
        channel(components.get(1)?),
        channel(components.get(2)?),
    ))
}
