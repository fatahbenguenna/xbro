//! Measurement throughput: cache hits must be cheap.
//! The strict bound (1,000,000 chars < 200 ms) only applies to optimised
//! builds: `cargo test -p xbro-text --release --test perf`.

mod common;

use std::time::Instant;

use common::*;
use xbro_text::FontId;

#[test]
fn measuring_a_million_ascii_chars_is_fast() {
    let Some(fonts) = dejavu() else { return };
    let sentence = "The quick brown fox jumps over the lazy dog. AV To Ty, 0123456789! ";
    let mut text = String::with_capacity(1_000_100);
    while text.len() < 1_000_000 {
        text.push_str(sentence);
    }
    text.truncate(1_000_000);

    // Warm the caches.
    let warm = fonts.measure(FontId(0), 16.0, sentence, 0.0, 0.0);
    assert!(warm > 0.0);

    let start = Instant::now();
    let width = fonts.measure(FontId(0), 16.0, &text, 0.0, 0.0);
    let elapsed = start.elapsed();
    eprintln!("measured 1,000,000 chars in {elapsed:?} (width {width})");
    assert!(width > 0.0);
    let limit_ms = if cfg!(debug_assertions) { 2_000 } else { 200 };
    assert!(
        elapsed.as_millis() < limit_ms,
        "measuring took {elapsed:?} (limit {limit_ms} ms)"
    );
}
