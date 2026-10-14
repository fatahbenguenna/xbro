//! Minimal, bounds-checked access to the raw SFNT (TrueType/OpenType) table
//! directory, used for the few pieces of information ab_glyph does not expose:
//! the `post` table underline metrics, the presence of outline / `kern`
//! tables, and the raw `cmap` table (see [`crate::cmap`]).
//!
//! Every read is checked; malformed data yields `None`, never a panic.

/// Reads a big-endian `u16` at `offset`.
#[inline]
pub(crate) fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    let bytes: [u8; 2] = data.get(offset..end)?.try_into().ok()?;
    Some(u16::from_be_bytes(bytes))
}

/// Reads a big-endian `i16` at `offset`.
#[inline]
pub(crate) fn read_i16(data: &[u8], offset: usize) -> Option<i16> {
    read_u16(data, offset).map(|v| v as i16)
}

/// Reads a big-endian `u32` at `offset`.
#[inline]
pub(crate) fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let bytes: [u8; 4] = data.get(offset..end)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

/// Cheap sanity check of font file data before handing it to fontdb: a
/// collection (`ttcf`) header must announce a face count whose offset array
/// fits in the data (fontdb pre-allocates one slot per announced face, so a
/// forged count of billions would abort the process).
pub(crate) fn plausible_font_data(data: &[u8]) -> bool {
    if data.len() < 12 {
        return false;
    }
    if data.get(0..4) != Some(b"ttcf".as_slice()) {
        return true;
    }
    match read_u32(data, 8) {
        Some(count) => {
            let needed = (count as u64).saturating_mul(4).saturating_add(12);
            count > 0 && needed <= data.len() as u64
        }
        None => false,
    }
}

/// The table directory of one face of a font file (plain or collection).
#[derive(Copy, Clone, Debug)]
pub(crate) struct FaceTables<'a> {
    data: &'a [u8],
    /// Offset of the face's table directory (offset table) in `data`.
    directory: usize,
    num_tables: usize,
}

impl<'a> FaceTables<'a> {
    /// Locates the table directory of face `index`. For a plain font file the
    /// index is ignored; for a `ttcf` collection it selects the face.
    pub(crate) fn parse(data: &'a [u8], index: u32) -> Option<Self> {
        let directory = if data.get(0..4)? == b"ttcf" {
            let num_fonts = read_u32(data, 8)?;
            if index >= num_fonts {
                return None;
            }
            let entry = 12usize.checked_add((index as usize).checked_mul(4)?)?;
            read_u32(data, entry)? as usize
        } else {
            0
        };
        let num_tables = usize::from(read_u16(data, directory.checked_add(4)?)?);
        Some(Self {
            data,
            directory,
            num_tables,
        })
    }

    /// Returns the bytes of the table tagged `tag`, if present and in bounds.
    pub(crate) fn table(&self, tag: &[u8; 4]) -> Option<&'a [u8]> {
        let records = self.directory.checked_add(12)?;
        for i in 0..self.num_tables {
            let record = records.checked_add(i.checked_mul(16)?)?;
            let record_tag = self.data.get(record..record.checked_add(4)?)?;
            if record_tag == tag {
                let offset = read_u32(self.data, record.checked_add(8)?)? as usize;
                let length = read_u32(self.data, record.checked_add(12)?)? as usize;
                return self.data.get(offset..offset.checked_add(length)?);
            }
        }
        None
    }

    /// True when the face has a table tagged `tag`.
    pub(crate) fn has_table(&self, tag: &[u8; 4]) -> bool {
        self.table(tag).is_some()
    }

    /// True when the face has vector outlines (`glyf`, `CFF ` or `CFF2`),
    /// i.e. it is not a bitmap-only / colour-bitmap-only font that ab_glyph
    /// cannot outline (e.g. Noto Color Emoji).
    pub(crate) fn has_outlines(&self) -> bool {
        self.has_table(b"glyf") || self.has_table(b"CFF ") || self.has_table(b"CFF2")
    }

    /// `post` table underline position and thickness in font units
    /// (position: positive = above the baseline, as stored in the font).
    pub(crate) fn underline(&self) -> Option<(i16, i16)> {
        let post = self.table(b"post")?;
        Some((read_i16(post, 8)?, read_i16(post, 10)?))
    }
}

/// Facts about a face gathered from its raw tables.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct SfntInfo {
    /// `post` underline (position, thickness) in font units, when present.
    pub underline: Option<(i16, i16)>,
    /// The face has vector outlines.
    pub has_outlines: bool,
    /// The face has a legacy `kern` table (the only kerning ab_glyph applies).
    pub has_kern: bool,
}

/// Inspects face `index` of `data`. Unparsable data gives the default
/// (no underline, assumed outlines, no kerning).
pub(crate) fn inspect(data: &[u8], index: u32) -> SfntInfo {
    match FaceTables::parse(data, index) {
        Some(tables) => SfntInfo {
            underline: tables.underline(),
            has_outlines: tables.has_outlines(),
            has_kern: tables.has_table(b"kern"),
        },
        None => SfntInfo {
            underline: None,
            has_outlines: true,
            has_kern: false,
        },
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a minimal SFNT file with the given tables (no checksums).
    pub(crate) fn build_sfnt(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        out.extend_from_slice(&[0u8; 6]);
        let mut offset = 12 + 16 * tables.len();
        let mut bodies = Vec::new();
        for (tag, body) in tables {
            out.extend_from_slice(&tag[..]);
            out.extend_from_slice(&0u32.to_be_bytes());
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            offset += body.len();
            bodies.extend_from_slice(body);
        }
        out.extend_from_slice(&bodies);
        out
    }

    fn post_table(position: i16, thickness: i16) -> Vec<u8> {
        let mut post = vec![0u8; 32];
        post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
        post[8..10].copy_from_slice(&position.to_be_bytes());
        post[10..12].copy_from_slice(&thickness.to_be_bytes());
        post
    }

    #[test]
    fn reads_tables_and_underline() {
        let font = build_sfnt(&[(b"post", post_table(-130, 90)), (b"glyf", vec![0; 4])]);
        let tables = FaceTables::parse(&font, 0).expect("directory");
        assert_eq!(tables.underline(), Some((-130, 90)));
        assert!(tables.has_outlines());
        assert!(!tables.has_table(b"kern"));
        let info = inspect(&font, 0);
        assert_eq!(info.underline, Some((-130, 90)));
        assert!(info.has_outlines);
        assert!(!info.has_kern);
    }

    #[test]
    fn bitmap_only_font_has_no_outlines() {
        let font = build_sfnt(&[(b"CBDT", vec![0; 8]), (b"CBLC", vec![0; 8])]);
        let tables = FaceTables::parse(&font, 0).expect("directory");
        assert!(!tables.has_outlines());
        assert_eq!(tables.underline(), None);
    }

    #[test]
    fn collections_select_the_face() {
        let face = build_sfnt(&[(b"kern", vec![0; 4])]);
        // ttcf header with two faces: face 0 is garbage offset, face 1 valid.
        let mut ttc = Vec::new();
        ttc.extend_from_slice(b"ttcf");
        ttc.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        ttc.extend_from_slice(&2u32.to_be_bytes());
        ttc.extend_from_slice(&0xFFFF_FFF0u32.to_be_bytes());
        let base = ttc.len() + 4;
        ttc.extend_from_slice(&(base as u32).to_be_bytes());
        // Table offsets inside `face` are relative to the file start: rebase.
        let mut rebased = face.clone();
        let table_offset = read_u32(&face, 12 + 8).unwrap() as usize + base;
        rebased[20..24].copy_from_slice(&(table_offset as u32).to_be_bytes());
        ttc.extend_from_slice(&rebased);
        assert!(FaceTables::parse(&ttc, 0).is_none());
        let tables = FaceTables::parse(&ttc, 1).expect("face 1");
        assert!(tables.has_table(b"kern"));
        assert!(FaceTables::parse(&ttc, 2).is_none());
    }

    #[test]
    fn plausibility_check() {
        assert!(!plausible_font_data(&[]));
        assert!(!plausible_font_data(&[0; 11]));
        assert!(plausible_font_data(&build_sfnt(&[])));
        let mut ttc = b"ttcf\x00\x01\x00\x00".to_vec();
        ttc.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(!plausible_font_data(&ttc));
        let mut ttc = b"ttcf\x00\x01\x00\x00".to_vec();
        ttc.extend_from_slice(&2u32.to_be_bytes());
        ttc.extend_from_slice(&[0; 8]);
        assert!(plausible_font_data(&ttc));
        ttc[11] = 0;
        assert!(!plausible_font_data(&ttc), "zero faces");
    }

    #[test]
    fn malformed_data_never_panics() {
        for len in 0..64 {
            let data = vec![0xFFu8; len];
            let _ = inspect(&data, 0);
            let _ = inspect(&data, 7);
        }
        let mut huge_count = build_sfnt(&[]);
        huge_count[4..6].copy_from_slice(&u16::MAX.to_be_bytes());
        let tables = FaceTables::parse(&huge_count, 0).expect("header");
        assert!(tables.table(b"post").is_none());
        // Table record pointing out of bounds.
        let mut font = build_sfnt(&[(b"post", post_table(-1, 1))]);
        font[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(FaceTables::parse(&font, 0).unwrap().underline(), None);
        assert_eq!(read_u16(&[1], 0), None);
        assert_eq!(read_u32(&[1, 2, 3], 0), None);
        assert_eq!(read_u16(&[1, 2], usize::MAX), None);
    }
}
