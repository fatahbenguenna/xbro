//! Font synthesis: synthetic oblique and bold, as browsers apply when a
//! family has no slanted or bold face (CSS `font-synthesis: weight style`).
//!
//! A synthesised face is addressed by its own [`FontId`]: the id of the
//! installed face with flag bits set in its two high bits. Every lookup by
//! id (faces, metrics, family names) ignores the flags, while the glyph and
//! raster caches key on the whole id, so a synthesised face never shares
//! cached advances or bitmaps with the plain one.
//!
//! * Oblique shears the outlines by ≈14° around the baseline (`x += 0.25 y`
//!   in font units) when rasterising; advances are unchanged.
//! * Bold dilates the coverage mask horizontally by `size / 24` px and adds
//!   as much to the advance of every glyph that has one, so measuring,
//!   shaping and drawing agree.

use crate::face::FaceEntry;
use crate::{FontId, Synthesis};

/// Flag bit of a [`FontId`] selecting synthetic oblique.
const OBLIQUE_BIT: u32 = 1 << 31;

/// Flag bit of a [`FontId`] selecting synthetic bold.
const BOLD_BIT: u32 = 1 << 30;

/// Bits of a [`FontId`] holding the face index.
const FACE_MASK: u32 = BOLD_BIT - 1;

/// Largest number of faces addressable alongside the flag bits.
pub(crate) const MAX_FACES: usize = FACE_MASK as usize + 1;

/// Horizontal shear of synthetic oblique: `x += y × OBLIQUE_SKEW` (y up),
/// i.e. a slant of atan(0.25) ≈ 14°.
pub(crate) const OBLIQUE_SKEW: f32 = 0.25;

/// Synthetic bold strength as a fraction of the em: the horizontal
/// dilation of the glyph ink and the extra advance of every glyph.
pub(crate) const BOLD_EM: f32 = 1.0 / 24.0;

/// Smallest requested weight that makes a light face (≤ 500) emboldened.
const BOLD_REQUEST: u16 = 600;

/// Heaviest face weight that still gets synthetic bold on a bold request.
const LIGHT_FACE: u16 = 500;

/// Weight assumed for a request satisfied with synthetic bold.
const SYNTHETIC_BOLD_WEIGHT: u16 = 700;

impl FontId {
    /// The installed face this id draws with: `self` without its
    /// [`Synthesis`] (ids returned by [`crate::FontSystem::select`] may
    /// denote "face + synthetic oblique and/or bold").
    #[inline]
    pub fn face(self) -> FontId {
        FontId(self.0 & FACE_MASK)
    }

    /// The synthetic styles this id applies on top of its [`FontId::face`].
    #[inline]
    pub fn synthesis(self) -> Synthesis {
        Synthesis {
            oblique: self.0 & OBLIQUE_BIT != 0,
            bold: self.0 & BOLD_BIT != 0,
        }
    }

    /// The face of `self` with exactly the synthetic styles `synthesis`.
    #[inline]
    pub fn with_synthesis(self, synthesis: Synthesis) -> FontId {
        let mut id = self.0 & FACE_MASK;
        if synthesis.oblique {
            id |= OBLIQUE_BIT;
        }
        if synthesis.bold {
            id |= BOLD_BIT;
        }
        FontId(id)
    }
}

impl Synthesis {
    /// True when no style is synthesised.
    #[inline]
    pub fn is_none(self) -> bool {
        !(self.oblique || self.bold)
    }
}

/// The synthesis a face needs to render a request: oblique when a slant is
/// requested and the face is upright, bold when a weight of 600 or more is
/// requested and the face weighs 500 or less (real italic and bold faces
/// are chosen by matching beforehand, so they are preferred).
pub(crate) fn needed(face: &FaceEntry, weight: u16, italic: bool) -> Synthesis {
    Synthesis {
        oblique: italic && !face.is_slanted(),
        bold: weight >= BOLD_REQUEST && face.weight <= LIGHT_FACE,
    }
}

/// The (weight, slant) a font id stands for: its face's, with synthetic
/// bold counting as 700 and synthetic oblique as slanted.
pub(crate) fn effective_style(face: &FaceEntry, id: FontId) -> (u16, bool) {
    let synthesis = id.synthesis();
    let weight = if synthesis.bold {
        face.weight.max(SYNTHETIC_BOLD_WEIGHT)
    } else {
        face.weight
    };
    (weight, synthesis.oblique || face.is_slanted())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        let face = FontId(12345);
        assert!(face.synthesis().is_none());
        assert_eq!(face.face(), face);
        for (oblique, bold) in [(false, false), (true, false), (false, true), (true, true)] {
            let synthesis = Synthesis { oblique, bold };
            let id = face.with_synthesis(synthesis);
            assert_eq!(id.face(), face);
            assert_eq!(id.synthesis(), synthesis);
            assert_eq!(id == face, synthesis.is_none());
            // Replacing the synthesis does not accumulate flags.
            assert_eq!(id.with_synthesis(Synthesis::default()), face);
        }
        let both = face.with_synthesis(Synthesis {
            oblique: true,
            bold: true,
        });
        assert_ne!(
            both,
            face.with_synthesis(Synthesis {
                oblique: true,
                bold: false
            })
        );
        // The largest face index keeps its flags apart.
        let last = FontId(FACE_MASK);
        assert_eq!(last.synthesis(), Synthesis::default());
        assert_eq!(FontId(u32::MAX).face(), last);
        assert_eq!(
            FontId(u32::MAX).synthesis(),
            Synthesis {
                oblique: true,
                bold: true
            }
        );
    }
}
