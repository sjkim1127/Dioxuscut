//! Fractal Brownian Motion (fBm) multi-octave synthesis and turbulent flow domain warping.

use serde::{Deserialize, Serialize};

use crate::seed::NoiseSeed;
use crate::simplex::SimplexNoise;

/// Configuration options for Fractional Brownian Motion (fBm) noise synthesis.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FbmOptions {
    /// Number of noise octaves / layers to synthesize. Defaults to 4.
    pub octaves: usize,
    /// Frequency multiplier per octave. Defaults to 2.0.
    pub lacunarity: f64,
    /// Amplitude multiplier per octave (persistence / gain). Defaults to 0.5.
    pub persistence: f64,
}

impl Default for FbmOptions {
    fn default() -> Self {
        Self {
            octaves: 4,
            lacunarity: 2.0,
            persistence: 0.5,
        }
    }
}

impl FbmOptions {
    /// Creates a new `FbmOptions` with custom octaves, lacunarity, and persistence.
    pub fn new(octaves: usize, lacunarity: f64, persistence: f64) -> Self {
        Self {
            octaves,
            lacunarity,
            persistence,
        }
    }
}

/// Evaluates multi-octave 2D Fractional Brownian Motion (fBm) in `[-1.0, 1.0]`.
pub fn fbm_2d(seed: impl Into<NoiseSeed>, x: f64, y: f64, options: &FbmOptions) -> f64 {
    if options.octaves == 0 || !x.is_finite() || !y.is_finite() {
        return 0.0;
    }
    let noise = SimplexNoise::new_2d(seed);
    fbm_2d_with_noise(&noise, x, y, options)
}

/// Evaluates multi-octave 2D fBm with a pre-instantiated [`SimplexNoise`] generator.
pub fn fbm_2d_with_noise(noise: &SimplexNoise, x: f64, y: f64, options: &FbmOptions) -> f64 {
    if options.octaves == 0 || !x.is_finite() || !y.is_finite() {
        return 0.0;
    }
    let mut total = 0.0;
    let mut frequency = 1.0;
    let mut amplitude = 1.0;
    let mut max_value = 0.0;

    for _ in 0..options.octaves {
        total += noise.noise_2d(x * frequency, y * frequency) * amplitude;
        max_value += amplitude;
        frequency *= options.lacunarity;
        amplitude *= options.persistence;
    }

    if max_value > 0.0 {
        (total / max_value).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Evaluates multi-octave 3D Fractional Brownian Motion (fBm) in `[-1.0, 1.0]`.
pub fn fbm_3d(seed: impl Into<NoiseSeed>, x: f64, y: f64, z: f64, options: &FbmOptions) -> f64 {
    if options.octaves == 0 || !x.is_finite() || !y.is_finite() || !z.is_finite() {
        return 0.0;
    }
    let noise = SimplexNoise::new_3d(seed);
    let mut total = 0.0;
    let mut frequency = 1.0;
    let mut amplitude = 1.0;
    let mut max_value = 0.0;

    for _ in 0..options.octaves {
        total += noise.noise_3d(x * frequency, y * frequency, z * frequency) * amplitude;
        max_value += amplitude;
        frequency *= options.lacunarity;
        amplitude *= options.persistence;
    }

    if max_value > 0.0 {
        (total / max_value).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Evaluates 2D turbulent noise (sum of absolute value harmonics) in `[0.0, 1.0]`.
pub fn turbulence_2d(seed: impl Into<NoiseSeed>, x: f64, y: f64, octaves: usize) -> f64 {
    if octaves == 0 || !x.is_finite() || !y.is_finite() {
        return 0.0;
    }
    let noise = SimplexNoise::new_2d(seed);
    let mut total = 0.0;
    let mut frequency = 1.0;
    let mut amplitude = 1.0;
    let mut max_value = 0.0;

    for _ in 0..octaves {
        total += noise.noise_2d(x * frequency, y * frequency).abs() * amplitude;
        max_value += amplitude;
        frequency *= 2.0;
        amplitude *= 0.5;
    }

    if max_value > 0.0 {
        (total / max_value).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Applies turbulent flow domain warping to coordinate `(x, y)` with given strength and frequency.
///
/// Returns displaced coordinate `(x + dx, y + dy)`.
pub fn turbulence_warp_2d(
    seed: impl Into<NoiseSeed>,
    x: f64,
    y: f64,
    strength: f64,
    freq: f64,
) -> (f64, f64) {
    if !x.is_finite() || !y.is_finite() || !strength.is_finite() || !freq.is_finite() {
        return (x, y);
    }
    let noise = SimplexNoise::new_2d(seed);
    let dx = noise.noise_2d(x * freq + 5.2, y * freq + 1.3) * strength;
    let dy = noise.noise_2d(x * freq + 1.7, y * freq + 9.2) * strength;

    (x + dx, y + dy)
}

/// Multi-stage domain warping field synthesis (Inigo Quilez formulation).
pub fn domain_warp_2d(
    seed: impl Into<NoiseSeed>,
    x: f64,
    y: f64,
    strength: f64,
    options: &FbmOptions,
) -> f64 {
    let s = seed.into();
    let qx = fbm_2d(s.clone(), x + 5.2, y + 1.3, options);
    let qy = fbm_2d(s.clone(), x + 1.7, y + 9.2, options);

    let rx = fbm_2d(s.clone(), x + 4.0 * qx + 1.7, y + 4.0 * qy + 9.2, options);
    let ry = fbm_2d(s.clone(), x + 4.0 * qx + 8.3, y + 4.0 * qy + 2.8, options);

    fbm_2d(s, x + strength * rx, y + strength * ry, options)
}

/// Warps a slice of 2D points `[(x, y)]` using turbulent flow domain deformation.
pub fn warp_points_2d(
    seed: impl Into<NoiseSeed>,
    points: &[(f64, f64)],
    strength: f64,
    freq: f64,
) -> Vec<(f64, f64)> {
    let s = seed.into();
    let noise = SimplexNoise::new_2d(s);
    points
        .iter()
        .map(|&(x, y)| {
            if !x.is_finite() || !y.is_finite() {
                return (x, y);
            }
            let dx = noise.noise_2d(x * freq + 5.2, y * freq + 1.3) * strength;
            let dy = noise.noise_2d(x * freq + 1.7, y * freq + 9.2) * strength;
            (x + dx, y + dy)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fbm_range_and_octaves() {
        let opts = FbmOptions::default();
        let val = fbm_2d("fbm-seed", 1.0, 2.0, &opts);
        assert!((-1.0..=1.0).contains(&val));

        let val_3d = fbm_3d("fbm-seed", 1.0, 2.0, 3.0, &opts);
        assert!((-1.0..=1.0).contains(&val_3d));

        let zero_octaves = FbmOptions::new(0, 2.0, 0.5);
        assert_eq!(fbm_2d("fbm-seed", 1.0, 2.0, &zero_octaves), 0.0);
    }

    #[test]
    fn test_turbulence_warp_2d() {
        let (wx, wy) = turbulence_warp_2d("warp-seed", 10.0, 20.0, 5.0, 0.1);
        assert!((wx - 10.0).abs() <= 5.0);
        assert!((wy - 20.0).abs() <= 5.0);
    }

    #[test]
    fn test_warp_points_2d() {
        let pts = vec![(0.0, 0.0), (10.0, 10.0), (20.0, 20.0)];
        let warped = warp_points_2d("pts-seed", &pts, 2.0, 0.05);
        assert_eq!(warped.len(), 3);
        assert_ne!(warped[0], pts[0]);
    }
}
