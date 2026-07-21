//! Deterministic seed hashing utilities for noise generation.

/// Hashes a string seed into a deterministic 64-bit integer using FNV-1a.
pub fn hash_seed(seed: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in seed.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Converts a 64-bit seed into a deterministic pseudo-random float in `[0.0, 1.0)`.
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
    fn test_seed_hashing_consistency() {
        let h1 = hash_seed("remotion-seed-123");
        let h2 = hash_seed("remotion-seed-123");
        let h3 = hash_seed("remotion-seed-456");

        assert_eq!(h1, h2);
        assert_ne!(h1, h3);

        let f1 = seed_to_float(h1);
        assert!((0.0..1.0).contains(&f1));
    }
}
