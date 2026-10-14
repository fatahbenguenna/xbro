//! A small, fast, std-only hasher for the hot caches (glyph and raster caches).
//!
//! It accumulates words like FxHash and finishes with the MurmurHash3 64-bit
//! finaliser, so that every input bit influences the low bits used by the
//! hash table for bucket selection (plain Fx leaves high input bits out of
//! the low output bits, which pages could exploit with chosen characters).

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Multiplicative constant (from rustc-hash 2).
const K: u64 = 0xf135_7aea_2e62_a9c5;

/// Fast non-cryptographic hasher for small fixed-size keys.
#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct FastHasher {
    hash: u64,
}

impl FastHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = self.hash.wrapping_add(word).wrapping_mul(K);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut buf = [0u8; 8];
            for (dst, src) in buf.iter_mut().zip(chunk) {
                *dst = *src;
            }
            self.add(u64::from_le_bytes(buf));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        // MurmurHash3 fmix64.
        let mut h = self.hash;
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        h ^= h >> 33;
        h
    }
}

/// `BuildHasher` for [`FastHasher`].
pub(crate) type FastBuildHasher = BuildHasherDefault<FastHasher>;

/// `HashMap` using [`FastHasher`].
pub(crate) type FastMap<K, V> = HashMap<K, V, FastBuildHasher>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{BuildHasher, Hash};

    fn hash_of<T: Hash>(value: T) -> u64 {
        FastBuildHasher::default().hash_one(value)
    }

    #[test]
    fn distinct_keys_hash_differently() {
        assert_ne!(hash_of((1u32, 'a')), hash_of((1u32, 'b')));
        assert_ne!(hash_of((1u32, 'a')), hash_of((2u32, 'a')));
        assert_eq!(hash_of((7u32, 'z')), hash_of((7u32, 'z')));
    }

    #[test]
    fn low_bits_depend_on_high_input_bits() {
        // Characters that differ only in high bits must not all share low bits.
        let low: std::collections::HashSet<u64> = (0..64u32)
            .map(|i| hash_of((0u32, i << 16)) & 0xff)
            .collect();
        assert!(low.len() > 16, "poor dispersion: {}", low.len());
    }

    #[test]
    fn byte_writes_handle_partial_chunks() {
        assert_ne!(hash_of("abc"), hash_of("abd"));
        assert_ne!(hash_of("abcdefghi"), hash_of("abcdefghj"));
        assert_eq!(hash_of(""), hash_of(""));
    }

    #[test]
    fn map_works() {
        let mut map: FastMap<(u32, char), u32> = FastMap::default();
        for (i, c) in ('a'..='z').enumerate() {
            map.insert((1, c), i as u32);
        }
        assert_eq!(map.get(&(1, 'c')), Some(&2));
        assert_eq!(map.get(&(2, 'c')), None);
    }
}
