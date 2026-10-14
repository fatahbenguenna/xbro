//! Shared helpers for the integration tests: deterministic font systems
//! built from the DejaVu (and optionally Liberation/Carlito) font files.
//! Tests skip gracefully when the files are not installed.

#![allow(dead_code)]

use xbro_text::{FontId, FontQuery, FontSystem};

pub const DEJAVU_DIR: &str = "/usr/share/fonts/truetype/dejavu";

pub const SANS: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";
pub const SANS_BOLD: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf";
pub const SERIF: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf";
pub const MONO: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf";
pub const MONO_OBLIQUE: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Oblique.ttf";
pub const LIBERATION_SANS: &str = "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf";
pub const LIBERATION_SERIF: &str =
    "/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf";
pub const CARLITO: &str = "/usr/share/fonts/truetype/crosextra/Carlito-Regular.ttf";
pub const COLOR_EMOJI: &str = "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf";
pub const UNIFONT_UPPER: &str = "/usr/share/fonts/opentype/unifont/unifont_upper.otf";

/// Builds a font system from the given files, in order (FontId(i) is the
/// i-th file). Returns `None` (and logs) when a file is missing.
pub fn system(paths: &[&str]) -> Option<FontSystem> {
    let mut fonts = Vec::new();
    for path in paths {
        match std::fs::read(path) {
            Ok(data) => fonts.push(data),
            Err(_) => {
                eprintln!("skipping: {path} not installed");
                return None;
            }
        }
    }
    Some(FontSystem::from_fonts(fonts))
}

/// The standard test system: Sans (0), Sans Bold (1), Serif (2), Mono (3).
pub fn dejavu() -> Option<FontSystem> {
    system(&[SANS, SANS_BOLD, SERIF, MONO])
}

/// Selects with lowercase family names.
pub fn select(fonts: &FontSystem, families: &[&str], weight: u16, italic: bool) -> FontId {
    let families: Vec<String> = families.iter().map(|f| f.to_string()).collect();
    fonts.select(&FontQuery {
        families: &families,
        weight,
        italic,
    })
}

/// Asserts two floats are within `eps`.
#[track_caller]
pub fn assert_close(actual: f32, expected: f32, eps: f32) {
    assert!(
        (actual - expected).abs() <= eps,
        "expected {expected} ± {eps}, got {actual}"
    );
}

/// Location of the table `tag` in an SFNT file: (record offset, table
/// offset, table length).
pub fn table_record(font: &[u8], tag: &[u8; 4]) -> Option<(usize, usize, usize)> {
    let count = usize::from(u16::from_be_bytes([*font.get(4)?, *font.get(5)?]));
    (0..count).find_map(|i| {
        let record = 12 + i * 16;
        if font.get(record..record + 4)? != tag {
            return None;
        }
        let word = |at: usize| -> Option<usize> {
            let bytes: [u8; 4] = font.get(at..at + 4)?.try_into().ok()?;
            Some(u32::from_be_bytes(bytes) as usize)
        };
        Some((record, word(record + 8)?, word(record + 12)?))
    })
}

/// Overwrites the big-endian `i16` at `field` inside table `tag`.
pub fn patch_i16(font: &mut [u8], tag: &[u8; 4], field: usize, value: i16) {
    let (_, offset, _) = table_record(font, tag).expect("table present");
    font[offset + field..offset + field + 2].copy_from_slice(&value.to_be_bytes());
}

/// Renames the table `from` to `to` in the table directory (the table
/// then counts as missing).
pub fn rename_table(font: &mut [u8], from: &[u8; 4], to: &[u8; 4]) {
    let (record, _, _) = table_record(font, from).expect("table present");
    font[record..record + 4].copy_from_slice(to);
}

/// A font collection (`ttcf`) of `count` faces that all point to the
/// same copy of `font`.
pub fn collection_of_copies(font: &[u8], count: u32) -> Vec<u8> {
    let header = 12 + 4 * count as usize;
    let mut ttc = Vec::with_capacity(header + font.len());
    ttc.extend_from_slice(b"ttcf");
    ttc.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    ttc.extend_from_slice(&count.to_be_bytes());
    for _ in 0..count {
        ttc.extend_from_slice(&(header as u32).to_be_bytes());
    }
    // Table offsets are relative to the start of the file: rebase them.
    let mut face = font.to_vec();
    let tables = usize::from(u16::from_be_bytes([face[4], face[5]]));
    for i in 0..tables {
        let at = 12 + i * 16 + 8;
        let offset = u32::from_be_bytes(face[at..at + 4].try_into().unwrap()) + header as u32;
        face[at..at + 4].copy_from_slice(&offset.to_be_bytes());
    }
    ttc.extend_from_slice(&face);
    ttc
}

/// Small deterministic PRNG (xorshift64*) for fuzz-style tests.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish value in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// Picks one element of a non-empty slice.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}
