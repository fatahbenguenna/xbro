//! Behaviour with no usable font at all.

use xbro_text::{FontId, FontQuery, FontSystem};

fn check_degraded(fonts: &FontSystem) {
    assert!(!fonts.has_fonts());
    assert_eq!(fonts.face_count(), 0);
    let families = vec!["serif".to_string()];
    let id = fonts.select(&FontQuery {
        families: &families,
        weight: 700,
        italic: true,
    });
    assert_eq!(id, FontId(0));
    assert_eq!(fonts.default_font(), FontId(0));
    let m = fonts.metrics(id, 10.0);
    assert_eq!((m.ascent, m.descent, m.x_height), (8.0, 2.0, 5.0));
    assert_eq!(fonts.measure(id, 10.0, "abcd", 0.0, 0.0), 20.0);
    let run = fonts.shape(id, 10.0, "ab", 0.0, 0.0);
    assert_eq!(run.glyphs.len(), 2);
    assert_eq!(run.glyphs[1].x, 5.0);
    assert_eq!(run.width, 10.0);
    assert!(fonts.rasterize(id, 0, 10.0, 0.0).is_none());
    assert_eq!(fonts.family_name(id), "");
}

#[test]
fn empty_font_system_degrades_gracefully() {
    check_degraded(&FontSystem::from_fonts(Vec::new()));
}

#[test]
fn garbage_font_data_is_skipped() {
    let garbage = vec![
        Vec::new(),
        vec![0u8; 100],
        b"ttcf\x00\x01\x00\x00\xff\xff\xff\xff".to_vec(),
        (0..5000u32).map(|i| (i * 31 % 251) as u8).collect(),
    ];
    check_degraded(&FontSystem::from_fonts(garbage));
}
