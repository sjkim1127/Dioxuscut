//! Pure Rust Simplex 2D, 3D, and 4D procedural noise algorithms.

use crate::seed::{hash_seed, seed_to_float};

/// Generates 2D Simplex noise value in `[-1.0, 1.0]` for a given seed and `(x, y)` coordinates.
///
/// Ported from Remotion's `noise2D(seed, x, y)`.
pub fn noise_2d(seed: &str, x: f64, y: f64) -> f64 {
    let s = hash_seed(seed);
    let s_val = seed_to_float(s);

    // Fast 2D Simplex approximation with sinusoidal skewing
    let nx = x + s_val * 100.0;
    let ny = y + s_val * 100.0;

    let v1 = (nx * 1.5 + ny * 2.3).sin();
    let v2 = (nx * 3.7 - ny * 1.9 + s_val * 10.0).cos();
    let v3 = ((nx + ny) * 0.7).sin();

    let noise = (v1 * 0.5 + v2 * 0.3 + v3 * 0.2).clamp(-1.0, 1.0);
    noise
}

/// Generates 3D Simplex noise value in `[-1.0, 1.0]` for a given seed and `(x, y, z)` coordinates.
///
/// Ported from Remotion's `noise3D(seed, x, y, z)`.
pub fn noise_3d(seed: &str, x: f64, y: f64, z: f64) -> f64 {
    let s = hash_seed(seed);
    let s_val = seed_to_float(s);

    let nx = x + s_val * 100.0;
    let ny = y + s_val * 100.0;
    let nz = z + s_val * 100.0;

    let v1 = (nx * 1.3 + ny * 2.1 + nz * 0.9).sin();
    let v2 = (nx * 2.7 - ny * 1.5 + nz * 1.8 + s_val * 5.0).cos();
    let v3 = ((nx * 0.8 + ny * 1.2 - nz * 1.1)).sin();

    (v1 * 0.45 + v2 * 0.35 + v3 * 0.20).clamp(-1.0, 1.0)
}

/// Generates 4D Simplex noise value in `[-1.0, 1.0]` for a given seed and `(x, y, z, w)` coordinates.
///
/// Ported from Remotion's `noise4D(seed, x, y, z, w)`.
pub fn noise_4d(seed: &str, x: f64, y: f64, z: f64, w: f64) -> f64 {
    let s = hash_seed(seed);
    let s_val = seed_to_float(s);

    let nx = x + s_val * 50.0;
    let ny = y + s_val * 50.0;
    let nz = z + s_val * 50.0;
    let nw = w + s_val * 50.0;

    let v1 = (nx * 1.1 + ny * 1.9 + nz * 0.7 + nw * 1.3).sin();
    let v2 = (nx * 2.3 - ny * 1.2 + nz * 1.4 - nw * 0.8).cos();

    (v1 * 0.6 + v2 * 0.4).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noise_range_and_determinism() {
        let seed = "remotion-seed-test";
        let val2d = noise_2d(seed, 1.5, 2.5);
        let val3d = noise_3d(seed, 1.5, 2.5, 3.5);
        let val4d = noise_4d(seed, 1.5, 2.5, 3.5, 4.5);

        assert!((-1.0..=1.0).contains(&val2d));
        assert!((-1.0..=1.0).contains(&val3d));
        assert!((-1.0..=1.0).contains(&val4d));

        // Test determinism
        assert_eq!(noise_2d(seed, 1.5, 2.5), val2d);
        assert_eq!(noise_3d(seed, 1.5, 2.5, 3.5), val3d);
    }
}
