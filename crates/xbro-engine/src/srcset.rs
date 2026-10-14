//! Responsive images: `srcset` / `sizes` parsing and candidate selection.
//!
//! xbro renders at one CSS px per device px for layout purposes, so the
//! selection favours 1x candidates (or, for width descriptors, the smallest
//! image at least as wide as the slot, capped at 1200 px).

use xbro_css::{MediaContext, MediaQueryList};

/// Largest slot width used when picking a width-described candidate.
const MAX_TARGET_WIDTH: f32 = 1200.0;

/// Descriptor of a `srcset` candidate.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum Descriptor {
    /// No descriptor (means `1x`).
    None,
    /// `2x`, `1.5x`.
    Density(f32),
    /// `640w`.
    Width(f32),
}

/// One image candidate of a `srcset` attribute.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Candidate {
    pub url: String,
    pub descriptor: Descriptor,
}

/// Parses a `srcset` attribute (HTML "parse a srcset attribute", simplified:
/// invalid descriptors drop the candidate). URLs may contain commas (`data:`).
pub(crate) fn parse_srcset(input: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    let n = chars.len();
    while i < n && out.len() < 1000 {
        // Skip whitespace and commas.
        while i < n && (chars[i].is_ascii_whitespace() || chars[i] == ',') {
            i += 1;
        }
        if i >= n {
            break;
        }
        let start = i;
        while i < n && !chars[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut url: String = chars[start..i].iter().collect();
        let mut descriptors = String::new();
        if url.ends_with(',') {
            // "url," — no descriptors.
            while url.ends_with(',') {
                url.pop();
            }
        } else {
            // Descriptors run up to the next comma outside parentheses.
            let mut depth = 0usize;
            while i < n {
                let c = chars[i];
                match c {
                    '(' => depth += 1,
                    ')' => depth = depth.saturating_sub(1),
                    ',' if depth == 0 => {
                        i += 1;
                        break;
                    }
                    _ => {}
                }
                descriptors.push(c);
                i += 1;
            }
        }
        if url.is_empty() {
            continue;
        }
        if let Some(descriptor) = parse_descriptors(&descriptors) {
            out.push(Candidate { url, descriptor });
        }
    }
    out
}

fn parse_descriptors(s: &str) -> Option<Descriptor> {
    let mut result = Descriptor::None;
    for token in s.split_ascii_whitespace() {
        let Some((split, _)) = token.char_indices().last() else {
            continue;
        };
        let (number, unit) = token.split_at(split);
        let value: f32 = number.parse().ok()?;
        if !value.is_finite() || value <= 0.0 {
            return None;
        }
        let d = match unit {
            "x" | "X" => Descriptor::Density(value),
            "w" | "W" => Descriptor::Width(value),
            // `h` descriptors are allowed (future-compat) and ignored.
            "h" | "H" => continue,
            _ => return None,
        };
        if result != Descriptor::None {
            return None; // Two descriptors.
        }
        result = d;
    }
    Some(result)
}

/// Splits on commas that are not inside parentheses.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Parses a `sizes` length (`300px`, `50vw`, `2em`, `0`).
fn parse_size_length(token: &str, ctx: &MediaContext) -> Option<f32> {
    let t = token.trim().to_ascii_lowercase();
    if t == "0" {
        return Some(0.0);
    }
    let (num, unit_scale) = if let Some(n) = t.strip_suffix("px") {
        (n, 1.0)
    } else if let Some(n) = t.strip_suffix("vw") {
        (n, ctx.width / 100.0)
    } else if let Some(n) = t.strip_suffix("vh") {
        (n, ctx.height / 100.0)
    } else {
        // `rem` and `em` (the initial font size: 16px).
        let n = t.strip_suffix("rem").or_else(|| t.strip_suffix("em"))?;
        (n, 16.0)
    };
    let v: f32 = num.parse().ok()?;
    let px = v * unit_scale;
    (px.is_finite() && px >= 0.0).then_some(px)
}

/// Evaluates a `sizes` attribute: the slot width in CSS px. Entries whose
/// length cannot be parsed (`calc()`, `auto`) are skipped; with no matching
/// entry the slot is the viewport width (`100vw`).
pub(crate) fn evaluate_sizes(sizes: &str, ctx: &MediaContext) -> Option<f32> {
    for entry in split_top_level_commas(sizes) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // The length is the last top-level token; the rest is the media condition.
        let (condition, length) = match entry.rfind(|c: char| c.is_ascii_whitespace() || c == ')') {
            Some(pos) => {
                let split = if entry[pos..].starts_with(')') {
                    pos + 1
                } else {
                    pos
                };
                (entry[..split].trim(), entry[split..].trim())
            }
            None => ("", entry),
        };
        let Some(px) = parse_size_length(length, ctx) else {
            continue;
        };
        if condition.is_empty() || MediaQueryList::parse_str(condition).matches(ctx) {
            return Some(px);
        }
    }
    None
}

/// The chosen image source and its pixel density (natural size = pixel size / density).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Choice {
    pub url: String,
    pub density: f32,
}

/// Picks a candidate from `srcset` (with `sizes`), `src` being the implicit
/// `1x` candidate when `srcset` has no width descriptors and no `1x` entry.
pub(crate) fn choose(
    srcset: &str,
    sizes: Option<&str>,
    src: Option<&str>,
    ctx: &MediaContext,
) -> Option<Choice> {
    let candidates = parse_srcset(srcset);
    let widths: Vec<(&Candidate, f32)> = candidates
        .iter()
        .filter_map(|c| match c.descriptor {
            Descriptor::Width(w) => Some((c, w)),
            _ => None,
        })
        .collect();
    if !widths.is_empty() {
        let slot = sizes.and_then(|s| evaluate_sizes(s, ctx));
        let viewport = if ctx.width.is_finite() && ctx.width > 0.0 {
            ctx.width
        } else {
            MAX_TARGET_WIDTH
        };
        let target = slot.unwrap_or(viewport).clamp(1.0, MAX_TARGET_WIDTH);
        let chosen = widths
            .iter()
            .filter(|(_, w)| *w >= target)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .or_else(|| widths.iter().max_by(|a, b| a.1.total_cmp(&b.1)))?;
        // Without a usable `sizes`, keep the image's own pixel size (forgiving).
        let density = match slot {
            Some(s) if s > 0.0 => (chosen.1 / s).clamp(0.1, 10.0),
            _ => 1.0,
        };
        return Some(Choice {
            url: chosen.0.url.clone(),
            density,
        });
    }
    let densities: Vec<(&Candidate, f32)> = candidates
        .iter()
        .map(|c| match c.descriptor {
            Descriptor::Density(d) => (c, d),
            _ => (c, 1.0),
        })
        .collect();
    if let Some((c, _)) = densities.iter().find(|(_, d)| (*d - 1.0).abs() < 1e-3) {
        return Some(Choice {
            url: c.url.clone(),
            density: 1.0,
        });
    }
    if let Some(src) = src.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(Choice {
            url: src.to_string(),
            density: 1.0,
        });
    }
    let chosen = densities
        .iter()
        .filter(|(_, d)| *d >= 1.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .or_else(|| densities.iter().max_by(|a, b| a.1.total_cmp(&b.1)))?;
    Some(Choice {
        url: chosen.0.url.clone(),
        density: chosen.1.clamp(0.1, 10.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(width: f32) -> MediaContext {
        MediaContext {
            width,
            height: 800.0,
            ..MediaContext::default()
        }
    }

    #[test]
    fn parses_candidates() {
        let c = parse_srcset(
            " a.png 1x, b.png 2x,c.png,  d.png 640w 480h , data:image/png;base64,AA== 3x",
        );
        assert_eq!(c.len(), 5);
        assert_eq!(c[0].descriptor, Descriptor::Density(1.0));
        assert_eq!(c[1].url, "b.png");
        assert_eq!(c[2].url, "c.png");
        assert_eq!(c[2].descriptor, Descriptor::None);
        assert_eq!(c[3].descriptor, Descriptor::Width(640.0));
        assert_eq!(c[4].url, "data:image/png;base64,AA==");
        // Invalid descriptors drop the candidate.
        assert!(parse_srcset("a.png 1q, b.png -2x, c.png 1x 2x").is_empty());
        assert!(parse_srcset("").is_empty());
        assert!(parse_srcset("a.png 1é, b.png 日, c.png 2日x").is_empty());
        assert!(parse_srcset(" , ,").is_empty());
    }

    #[test]
    fn sizes() {
        let c = ctx(500.0);
        assert_eq!(
            evaluate_sizes("(max-width: 600px) 100vw, 300px", &c),
            Some(500.0)
        );
        assert_eq!(
            evaluate_sizes("(max-width: 400px) 100vw, 300px", &c),
            Some(300.0)
        );
        assert_eq!(evaluate_sizes("calc(100vw - 2em), 50vw", &c), Some(250.0));
        assert_eq!(evaluate_sizes("auto", &c), None);
        assert_eq!(evaluate_sizes("", &c), None);
    }

    #[test]
    fn chooses_candidates() {
        let c = ctx(1024.0);
        let pick = |s, sizes, src| choose(s, sizes, src, &c);
        assert_eq!(pick("a.png 1x, b.png 2x", None, None).unwrap().url, "a.png");
        assert_eq!(pick("b.png 2x", None, Some("a.png")).unwrap().url, "a.png");
        let only2x = pick("b.png 2x", None, None).unwrap();
        assert_eq!((only2x.url.as_str(), only2x.density), ("b.png", 2.0));
        let w = pick(
            "s.png 320w, m.png 800w, l.png 1600w, xl.png 3000w",
            None,
            None,
        )
        .unwrap();
        assert_eq!((w.url.as_str(), w.density), ("l.png", 1.0));
        let w = pick("s.png 320w, m.png 800w", None, None).unwrap();
        assert_eq!(w.url, "m.png");
        let w = pick("s.png 320w, m.png 800w", Some("300px"), None).unwrap();
        assert_eq!(w.url, "s.png");
        assert!((w.density - 320.0 / 300.0).abs() < 1e-4);
        assert_eq!(pick("", None, Some(" x.png ")).unwrap().url, "x.png");
        assert_eq!(pick("", None, None), None);
    }
}
