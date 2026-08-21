//! Deterministic seed hashing and PRNG utilities for procedural noise generation.
//!
//! Provides Remotion-compatible Mulberry32 PRNG, 32-bit Java `hashCode` computation,
//! and [`NoiseSeed`] conversion types.

use std::fmt;

/// Represents a seed value for noise and pseudo-random number generators.
///
/// Can be constructed from strings (`&str`, `String`), integers (`i32`, `i64`, `u32`, `u64`, `usize`),
/// or floating-point numbers (`f32`, `f64`).
#[derive(Clone, Debug, PartialEq)]
pub enum NoiseSeed {
    /// Textual seed.
    Str(String),
    /// Numeric seed.
    Num(f64),
}

impl fmt::Display for NoiseSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoiseSeed::Str(s) => write!(f, "{}", s),
            NoiseSeed::Num(n) => write!(f, "{}", n),
        }
    }
}

impl From<&str> for NoiseSeed {
    fn from(s: &str) -> Self {
        NoiseSeed::Str(s.to_string())
    }
}

impl From<String> for NoiseSeed {
    fn from(s: String) -> Self {
        NoiseSeed::Str(s)
    }
}

impl From<&String> for NoiseSeed {
    fn from(s: &String) -> Self {
        NoiseSeed::Str(s.clone())
    }
}

impl From<f64> for NoiseSeed {
    fn from(n: f64) -> Self {
        NoiseSeed::Num(n)
    }
}

impl From<f32> for NoiseSeed {
    fn from(n: f32) -> Self {
        NoiseSeed::Num(n as f64)
    }
}

impl From<i32> for NoiseSeed {
    fn from(n: i32) -> Self {
        NoiseSeed::Num(n as f64)
    }
}

impl From<i64> for NoiseSeed {
    fn from(n: i64) -> Self {
        NoiseSeed::Num(n as f64)
    }
}

impl From<u32> for NoiseSeed {
    fn from(n: u32) -> Self {
        NoiseSeed::Num(n as f64)
    }
}

impl From<u64> for NoiseSeed {
    fn from(n: u64) -> Self {
        NoiseSeed::Num(n as f64)
    }
}

impl From<usize> for NoiseSeed {
    fn from(n: usize) -> Self {
        NoiseSeed::Num(n as f64)
    }
}

/// Computes 32-bit Java-compatible string hash code using UTF-16 code units.
///
/// Matches Remotion `hashCode(str)` exactly:
/// `hash = ((hash << 5) - hash + charCode) | 0`
pub fn hash_code(s: &str) -> i32 {
    let mut hash: i32 = 0;
    for code_unit in s.encode_utf16() {
        hash = hash.wrapping_mul(31).wrapping_add(code_unit as i32);
    }
    hash
}

/// Remotion Mulberry32 32-bit pseudo-random generator.
///
/// Takes a 32-bit signed or unsigned integer seed and returns a deterministic
/// floating-point number in `[0.0, 1.0)`.
#[inline]
pub fn mulberry32(a: i64) -> f64 {
    let a_u32 = a as u32;
    let t0 = a_u32.wrapping_add(0x6D2B79F5);
    let t1 = (t0 ^ (t0 >> 15)).wrapping_mul(t0 | 1);
    let t2 = t1 ^ t1.wrapping_add((t1 ^ (t1 >> 7)).wrapping_mul(t1 | 61));
    let result = t2 ^ (t2 >> 14);
    (result as f64) / 4294967296.0
}

/// Deterministic pseudo-random number generator matching Remotion's `random(seed)`.
///
/// - For string seeds: `mulberry32(hashCode(seed))`
/// - For numeric seeds: `mulberry32(seed * 10_000_000_000)`
#[inline]
pub fn random(seed: impl Into<NoiseSeed>) -> f64 {
    match seed.into() {
        NoiseSeed::Str(s) => mulberry32(hash_code(&s) as i64),
        NoiseSeed::Num(n) => mulberry32((n * 10_000_000_000.0) as i64),
    }
}

/// Legacy FNV-1a 64-bit string hash (kept for backward compatibility).
pub fn hash_seed(seed: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in seed.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Converts a 64-bit seed into a deterministic pseudo-random float in `[0.0, 1.0)` (kept for backward compatibility).
pub fn seed_to_float(seed: u64) -> f64 {
    let mut x = seed;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    let val = x.wrapping_mul(0x2545F4914F6CDD1D);
    (val as f64) / (u64::MAX as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_code_matches_java_and_remotion() {
        assert_eq!(hash_code("my-seed"), 1462865394);
        assert_eq!(hash_code("hello"), 99162322);
        assert_eq!(hash_code(""), 0);
    }

    #[test]
    fn test_random_values() {
        let r_str = random("my-seed");
        assert!((0.0..1.0).contains(&r_str));

        let r_num = random(1.0);
        assert!((0.0..1.0).contains(&r_num));

        // Determinism
        assert_eq!(random("test-seed"), random("test-seed"));
        assert_eq!(random(42.0), random(42.0));
    }
}
