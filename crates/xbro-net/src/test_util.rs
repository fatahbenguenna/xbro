//! Helpers shared by the unit tests (deterministic pseudo-random inputs).

/// A tiny deterministic xorshift generator, so fuzz-style tests are reproducible.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A value in `0..n` (`n > 0`).
    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Random bytes of a random length below `max_len`.
    pub(crate) fn bytes(&mut self, max_len: usize) -> Vec<u8> {
        let len = self.below(max_len);
        (0..len).map(|_| self.next_u64() as u8).collect()
    }

    /// A random string built from `alphabet` pieces (characters or snippets).
    pub(crate) fn string_from(&mut self, alphabet: &[&str], max_pieces: usize) -> String {
        let pieces = self.below(max_pieces);
        (0..pieces)
            .map(|_| alphabet[self.below(alphabet.len())])
            .collect()
    }
}
