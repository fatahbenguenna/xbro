//! Simple shaping (one glyph per character, left to right, with kerning)
//! and allocation-free measurement sharing the same code path.

use crate::chars::{classify, is_word_separator, CharClass};
use crate::hash::FastMap;
use crate::metrics::{sanitize_size, sanitize_spacing};
use crate::sync::{read, write};
use crate::{FontId, FontSystem, ShapedGlyph};

/// Maximum number of cached (font, char) entries before a reset.
const MAX_GLYPH_ENTRIES: usize = 1 << 18;

/// How one character of one requested font is rendered.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct CharEntry {
    /// Face providing the glyph (the requested face or a fallback), with
    /// the synthetic styles to draw it with.
    pub font: FontId,
    pub glyph: u16,
    /// Advance as a fraction of the em (synthetic bold included).
    pub advance_em: f32,
    /// The face has a `kern` table.
    pub kern: bool,
}

/// Entry used when no font is available at all: 0.5em per character.
const DEGRADED_ENTRY: CharEntry = CharEntry {
    font: FontId(0),
    glyph: 0,
    advance_em: 0.5,
    kern: false,
};

/// Cache: (requested font, char) → entry.
pub(crate) type GlyphMap = FastMap<(FontId, char), CharEntry>;

/// Maximum number of cached kerning pairs before a reset.
const MAX_KERN_ENTRIES: usize = 1 << 16;

/// Cache: (face, first glyph, second glyph) → kerning as a fraction of the
/// em. Looking a pair up in a large `kern` table (a binary search per
/// subtable) cost two thirds of the shaping time of ordinary text.
pub(crate) type KernMap = FastMap<(FontId, u16, u16), f32>;

/// A glyph waiting for the next one (its advance may still get kerning).
struct Pending {
    glyph: ShapedGlyph,
    entry: CharEntry,
}

impl FontSystem {
    /// Lays out `text` and calls `emit` for every glyph in order; returns the
    /// total advance, which is exactly the sum of the emitted advances.
    /// `shape` collects the glyphs and `measure` ignores them, so both always
    /// agree bit for bit.
    pub(crate) fn layout_run(
        &self,
        font: FontId,
        size: f32,
        text: &str,
        letter_spacing: f32,
        word_spacing: f32,
        mut emit: impl FnMut(ShapedGlyph),
    ) -> f32 {
        let size = sanitize_size(size);
        let letter_spacing = sanitize_spacing(letter_spacing);
        let word_spacing = sanitize_spacing(word_spacing);
        // No usable face at all (empty database, or nothing parses).
        let degraded = !self.has_fonts();
        let font = if degraded {
            FontId(0)
        } else {
            self.valid_font(font)
        };

        // Accumulated in f64 so positions don't drift on very long runs
        // (f32 loses whole pixels after ~20k glyphs).
        let mut pen = 0.0f64;
        let mut pending: Option<Pending> = None;
        // Lock order: kerning cache, then glyph cache (never the reverse).
        let kerns = read(&self.kern_cache);
        let mut new_kerns: Vec<((FontId, u16, u16), f32)> = Vec::new();
        let mut cache = read(&self.glyph_cache);
        for (cluster, c) in text.char_indices() {
            let key = match classify(c) {
                CharClass::Ignorable => continue,
                CharClass::Tab => ' ',
                CharClass::Normal => c,
            };
            let entry = if degraded {
                DEGRADED_ENTRY
            } else {
                match cache.get(&(font, key)).copied() {
                    Some(entry) => entry,
                    None => {
                        drop(cache);
                        let entry = self.resolve_and_cache(font, key);
                        cache = read(&self.glyph_cache);
                        entry
                    }
                }
            };

            if let Some(mut prev) = pending.take() {
                if entry.kern && prev.entry.font == entry.font {
                    let key = (entry.font, prev.entry.glyph, entry.glyph);
                    let kern = match kerns.get(&key) {
                        Some(&kern) => Some(kern),
                        None => self
                            .collection
                            .face(entry.font)
                            .and_then(|face| face.loaded_if_ready())
                            .map(|face| {
                                let kern = face.kern_em(prev.entry.glyph, entry.glyph);
                                new_kerns.push((key, kern));
                                kern
                            }),
                    };
                    if let Some(kern) = kern {
                        prev.glyph.advance += kern * size;
                    }
                }
                pen += f64::from(prev.glyph.advance);
                emit(prev.glyph);
            }

            let mut advance = entry.advance_em * size + letter_spacing;
            if is_word_separator(c) {
                advance += word_spacing;
            }
            pending = Some(Pending {
                glyph: ShapedGlyph {
                    font: entry.font,
                    glyph_id: entry.glyph,
                    x: pen as f32,
                    advance,
                    cluster,
                },
                entry,
            });
        }
        drop(cache);
        drop(kerns);
        if !new_kerns.is_empty() {
            let mut map = write(&self.kern_cache);
            if map.len() + new_kerns.len() > MAX_KERN_ENTRIES {
                map.clear();
            }
            map.extend(new_kerns.into_iter().take(MAX_KERN_ENTRIES));
        }
        if let Some(last) = pending {
            pen += f64::from(last.glyph.advance);
            emit(last.glyph);
        }
        pen as f32
    }

    /// Resolves `c` for `font` and caches the result when it is final.
    fn resolve_and_cache(&self, font: FontId, c: char) -> CharEntry {
        let (entry, cacheable) = self.resolve_char(font, c);
        if cacheable {
            let mut map = write(&self.glyph_cache);
            if map.len() >= MAX_GLYPH_ENTRIES {
                map.clear();
            }
            map.insert((font, c), entry);
        }
        entry
    }

    /// Finds the face and glyph rendering `c` when `font` is requested:
    /// the font itself, U+00A0 → its space glyph, a fallback face, and
    /// finally the font's `.notdef` glyph. The flag tells whether the result
    /// may be cached (false when the fallback scan was cut short).
    fn resolve_char(&self, font: FontId, c: char) -> (CharEntry, bool) {
        let primary = self.collection.loaded(font);
        if let Some(face) = primary {
            let glyph = face.glyph(c);
            if glyph != 0 {
                return (entry_for(font, face, glyph), true);
            }
            if c == '\u{A0}' && face.space_glyph != 0 {
                return (entry_for(font, face, face.space_glyph), true);
            }
        }

        let (fallback, cacheable) = self.fallback_face(c, font);
        if let Some(id) = fallback {
            if let Some(face) = self.collection.loaded(id) {
                let glyph = face.glyph(c);
                if glyph != 0 {
                    return (entry_for(id, face, glyph), cacheable);
                }
            }
        }

        if c == '\u{A0}' {
            // No face has a no-break space: render it as a space.
            let (entry, space_cacheable) = self.resolve_char(font, ' ');
            return (entry, cacheable && space_cacheable);
        }

        let entry = match primary {
            Some(face) => entry_for(font, face, 0),
            None => CharEntry {
                font,
                ..DEGRADED_ENTRY
            },
        };
        (entry, cacheable)
    }
}

/// Entry for `glyph` of the loaded `face` (id `font`, which may carry
/// synthetic styles: synthetic bold widens every non-zero advance).
fn entry_for(font: FontId, face: &crate::face::LoadedFace, glyph: u16) -> CharEntry {
    let mut advance_em = face.advance_em(glyph);
    if font.synthesis().bold && advance_em > 0.0 {
        advance_em += crate::synthesis::BOLD_EM;
    }
    CharEntry {
        font,
        glyph,
        advance_em,
        kern: face.has_kern,
    }
}
