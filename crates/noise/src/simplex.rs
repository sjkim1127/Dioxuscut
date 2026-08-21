//! Stefan Gustavson pure-Rust Simplex 2D, 3D, and 4D procedural noise algorithms.
//!
//! Provides deterministic Simplex noise evaluation compatible with Remotion's `@remotion/noise`.

use crate::seed::{random, NoiseSeed};

const GRAD3: [(f64, f64, f64); 12] = [
    (1.0, 1.0, 0.0),
    (-1.0, 1.0, 0.0),
    (1.0, -1.0, 0.0),
    (-1.0, -1.0, 0.0),
    (1.0, 0.0, 1.0),
    (-1.0, 0.0, 1.0),
    (1.0, 0.0, -1.0),
    (-1.0, 0.0, -1.0),
    (0.0, 1.0, 1.0),
    (0.0, -1.0, 1.0),
    (0.0, 1.0, -1.0),
    (0.0, -1.0, -1.0),
];

const GRAD4: [(f64, f64, f64, f64); 32] = [
    (0.0, 1.0, 1.0, 1.0),
    (0.0, 1.0, 1.0, -1.0),
    (0.0, 1.0, -1.0, 1.0),
    (0.0, 1.0, -1.0, -1.0),
    (0.0, -1.0, 1.0, 1.0),
    (0.0, -1.0, 1.0, -1.0),
    (0.0, -1.0, -1.0, 1.0),
    (0.0, -1.0, -1.0, -1.0),
    (1.0, 0.0, 1.0, 1.0),
    (1.0, 0.0, 1.0, -1.0),
    (1.0, 0.0, -1.0, 1.0),
    (1.0, 0.0, -1.0, -1.0),
    (-1.0, 0.0, 1.0, 1.0),
    (-1.0, 0.0, 1.0, -1.0),
    (-1.0, 0.0, -1.0, 1.0),
    (-1.0, 0.0, -1.0, -1.0),
    (1.0, 1.0, 0.0, 1.0),
    (1.0, 1.0, 0.0, -1.0),
    (1.0, -1.0, 0.0, 1.0),
    (1.0, -1.0, 0.0, -1.0),
    (-1.0, 1.0, 0.0, 1.0),
    (-1.0, 1.0, 0.0, -1.0),
    (-1.0, -1.0, 0.0, 1.0),
    (-1.0, -1.0, 0.0, -1.0),
    (1.0, 1.0, 1.0, 0.0),
    (1.0, 1.0, -1.0, 0.0),
    (1.0, -1.0, 1.0, 0.0),
    (1.0, -1.0, -1.0, 0.0),
    (-1.0, 1.0, 1.0, 0.0),
    (-1.0, 1.0, -1.0, 0.0),
    (-1.0, -1.0, 1.0, 0.0),
    (-1.0, -1.0, -1.0, 0.0),
];

const F2: f64 = 0.5 * (1.7320508075688772 - 1.0); // 0.3660254037844386
const G2: f64 = (3.0 - 1.7320508075688772) / 6.0; // 0.21132486540518713

const F3: f64 = 1.0 / 3.0;
const G3: f64 = 1.0 / 6.0;

const F4: f64 = (2.23606797749979 - 1.0) / 4.0; // 0.30901699437494745
const G4: f64 = (5.0 - 2.23606797749979) / 20.0; // 0.1381966011250105

/// Permutation table state for Stefan Gustavson Simplex noise generator.
#[derive(Clone, Debug)]
pub struct SimplexNoise {
    perm: [u8; 512],
    perm_mod12: [u8; 512],
}

impl SimplexNoise {
    /// Creates a SimplexNoise instance from a PRNG closure.
    pub fn from_prng<F: FnMut() -> f64>(mut prng: F) -> Self {
        let mut p = [0u8; 256];
        for (i, item) in p.iter_mut().enumerate() {
            *item = i as u8;
        }
        for i in 0..255 {
            let r = i + (prng() * (256 - i) as f64).floor() as usize;
            p.swap(i, r);
        }
        let mut perm = [0u8; 512];
        let mut perm_mod12 = [0u8; 512];
        for i in 0..512 {
            let val = p[i & 255];
            perm[i] = val;
            perm_mod12[i] = val % 12;
        }
        Self { perm, perm_mod12 }
    }

    /// Creates a new SimplexNoise generator for 2D noise with the given seed.
    pub fn new_2d(seed: impl Into<NoiseSeed>) -> Self {
        let s = seed.into();
        Self::from_prng(move || random(s.clone()))
    }

    /// Creates a new SimplexNoise generator for 3D noise with the given seed.
    pub fn new_3d(seed: impl Into<NoiseSeed>) -> Self {
        let s = seed.into();
        let r = random(s);
        Self::from_prng(move || random(r))
    }

    /// Creates a new SimplexNoise generator for 4D noise with the given seed.
    pub fn new_4d(seed: impl Into<NoiseSeed>) -> Self {
        let s = seed.into();
        let r = random(s);
        Self::from_prng(move || random(r))
    }

    /// Creates a standard SimplexNoise generator seeded with default 2D seed hashing.
    pub fn new(seed: impl Into<NoiseSeed>) -> Self {
        Self::new_2d(seed)
    }

    /// Evaluates 2D Simplex noise at `(x, y)` in `[-1.0, 1.0]`.
    pub fn noise_2d(&self, x: f64, y: f64) -> f64 {
        if !x.is_finite() || !y.is_finite() || x.abs() > 1e15 || y.abs() > 1e15 {
            return 0.0;
        }

        let s = (x + y) * F2;
        let i = (x + s).floor();
        let j = (y + s).floor();

        let t = (i + j) * G2;
        let x0_orig = i - t;
        let y0_orig = j - t;

        let x0 = x - x0_orig;
        let y0 = y - y0_orig;

        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };

        let x1 = x0 - i1 as f64 + G2;
        let y1 = y0 - j1 as f64 + G2;

        let x2 = x0 - 1.0 + 2.0 * G2;
        let y2 = y0 - 1.0 + 2.0 * G2;

        let ii = (i.rem_euclid(256.0) as usize) & 255;
        let jj = (j.rem_euclid(256.0) as usize) & 255;

        let gi0 = self.perm_mod12[ii + self.perm[jj] as usize] as usize;
        let gi1 = self.perm_mod12[ii + i1 + self.perm[jj + j1] as usize] as usize;
        let gi2 = self.perm_mod12[ii + 1 + self.perm[jj + 1] as usize] as usize;

        let mut t0 = 0.5 - x0 * x0 - y0 * y0;
        let n0 = if t0 < 0.0 {
            0.0
        } else {
            t0 *= t0;
            t0 * t0 * (GRAD3[gi0].0 * x0 + GRAD3[gi0].1 * y0)
        };

        let mut t1 = 0.5 - x1 * x1 - y1 * y1;
        let n1 = if t1 < 0.0 {
            0.0
        } else {
            t1 *= t1;
            t1 * t1 * (GRAD3[gi1].0 * x1 + GRAD3[gi1].1 * y1)
        };

        let mut t2 = 0.5 - x2 * x2 - y2 * y2;
        let n2 = if t2 < 0.0 {
            0.0
        } else {
            t2 *= t2;
            t2 * t2 * (GRAD3[gi2].0 * x2 + GRAD3[gi2].1 * y2)
        };

        70.0 * (n0 + n1 + n2)
    }

    /// Evaluates 3D Simplex noise at `(x, y, z)` in `[-1.0, 1.0]`.
    pub fn noise_3d(&self, x: f64, y: f64, z: f64) -> f64 {
        if !x.is_finite()
            || !y.is_finite()
            || !z.is_finite()
            || x.abs() > 1e15
            || y.abs() > 1e15
            || z.abs() > 1e15
        {
            return 0.0;
        }

        let s = (x + y + z) * F3;
        let i = (x + s).floor();
        let j = (y + s).floor();
        let k = (z + s).floor();

        let t = (i + j + k) * G3;
        let x0_orig = i - t;
        let y0_orig = j - t;
        let z0_orig = k - t;

        let x0 = x - x0_orig;
        let y0 = y - y0_orig;
        let z0 = z - z0_orig;

        let (i1, j1, k1, i2, j2, k2) = if x0 >= y0 {
            if y0 >= z0 {
                (1, 0, 0, 1, 1, 0)
            } else if x0 >= z0 {
                (1, 0, 0, 1, 0, 1)
            } else {
                (0, 0, 1, 1, 0, 1)
            }
        } else if y0 < z0 {
            (0, 0, 1, 0, 1, 1)
        } else if x0 < z0 {
            (0, 1, 0, 0, 1, 1)
        } else {
            (0, 1, 0, 1, 1, 0)
        };

        let x1 = x0 - i1 as f64 + G3;
        let y1 = y0 - j1 as f64 + G3;
        let z1 = z0 - k1 as f64 + G3;

        let x2 = x0 - i2 as f64 + 2.0 * G3;
        let y2 = y0 - j2 as f64 + 2.0 * G3;
        let z2 = z0 - k2 as f64 + 2.0 * G3;

        let x3 = x0 - 1.0 + 3.0 * G3;
        let y3 = y0 - 1.0 + 3.0 * G3;
        let z3 = z0 - 1.0 + 3.0 * G3;

        let ii = (i.rem_euclid(256.0) as usize) & 255;
        let jj = (j.rem_euclid(256.0) as usize) & 255;
        let kk = (k.rem_euclid(256.0) as usize) & 255;

        let gi0 = self.perm_mod12[ii + self.perm[jj + self.perm[kk] as usize] as usize] as usize;
        let gi1 = self.perm_mod12
            [ii + i1 + self.perm[jj + j1 + self.perm[kk + k1] as usize] as usize]
            as usize;
        let gi2 = self.perm_mod12
            [ii + i2 + self.perm[jj + j2 + self.perm[kk + k2] as usize] as usize]
            as usize;
        let gi3 = self.perm_mod12[ii + 1 + self.perm[jj + 1 + self.perm[kk + 1] as usize] as usize]
            as usize;

        let mut t0 = 0.6 - x0 * x0 - y0 * y0 - z0 * z0;
        let n0 = if t0 < 0.0 {
            0.0
        } else {
            t0 *= t0;
            t0 * t0 * (GRAD3[gi0].0 * x0 + GRAD3[gi0].1 * y0 + GRAD3[gi0].2 * z0)
        };

        let mut t1 = 0.6 - x1 * x1 - y1 * y1 - z1 * z1;
        let n1 = if t1 < 0.0 {
            0.0
        } else {
            t1 *= t1;
            t1 * t1 * (GRAD3[gi1].0 * x1 + GRAD3[gi1].1 * y1 + GRAD3[gi1].2 * z1)
        };

        let mut t2 = 0.6 - x2 * x2 - y2 * y2 - z2 * z2;
        let n2 = if t2 < 0.0 {
            0.0
        } else {
            t2 *= t2;
            t2 * t2 * (GRAD3[gi2].0 * x2 + GRAD3[gi2].1 * y2 + GRAD3[gi2].2 * z2)
        };

        let mut t3 = 0.6 - x3 * x3 - y3 * y3 - z3 * z3;
        let n3 = if t3 < 0.0 {
            0.0
        } else {
            t3 *= t3;
            t3 * t3 * (GRAD3[gi3].0 * x3 + GRAD3[gi3].1 * y3 + GRAD3[gi3].2 * z3)
        };

        32.0 * (n0 + n1 + n2 + n3)
    }

    /// Evaluates 4D Simplex noise at `(x, y, z, w)` in `[-1.0, 1.0]`.
    pub fn noise_4d(&self, x: f64, y: f64, z: f64, w: f64) -> f64 {
        if !x.is_finite()
            || !y.is_finite()
            || !z.is_finite()
            || !w.is_finite()
            || x.abs() > 1e15
            || y.abs() > 1e15
            || z.abs() > 1e15
            || w.abs() > 1e15
        {
            return 0.0;
        }

        let s = (x + y + z + w) * F4;
        let i = (x + s).floor();
        let j = (y + s).floor();
        let k = (z + s).floor();
        let l = (w + s).floor();

        let t = (i + j + k + l) * G4;
        let x0_orig = i - t;
        let y0_orig = j - t;
        let z0_orig = k - t;
        let w0_orig = l - t;

        let x0 = x - x0_orig;
        let y0 = y - y0_orig;
        let z0 = z - z0_orig;
        let w0 = w - w0_orig;

        let mut rank_x = 0usize;
        let mut rank_y = 0usize;
        let mut rank_z = 0usize;
        let mut rank_w = 0usize;

        if x0 > y0 {
            rank_x += 1;
        } else {
            rank_y += 1;
        }
        if x0 > z0 {
            rank_x += 1;
        } else {
            rank_z += 1;
        }
        if x0 > w0 {
            rank_x += 1;
        } else {
            rank_w += 1;
        }
        if y0 > z0 {
            rank_y += 1;
        } else {
            rank_z += 1;
        }
        if y0 > w0 {
            rank_y += 1;
        } else {
            rank_w += 1;
        }
        if z0 > w0 {
            rank_z += 1;
        } else {
            rank_w += 1;
        }

        let i1 = if rank_x >= 3 { 1 } else { 0 };
        let j1 = if rank_y >= 3 { 1 } else { 0 };
        let k1 = if rank_z >= 3 { 1 } else { 0 };
        let l1 = if rank_w >= 3 { 1 } else { 0 };

        let i2 = if rank_x >= 2 { 1 } else { 0 };
        let j2 = if rank_y >= 2 { 1 } else { 0 };
        let k2 = if rank_z >= 2 { 1 } else { 0 };
        let l2 = if rank_w >= 2 { 1 } else { 0 };

        let i3 = if rank_x >= 1 { 1 } else { 0 };
        let j3 = if rank_y >= 1 { 1 } else { 0 };
        let k3 = if rank_z >= 1 { 1 } else { 0 };
        let l3 = if rank_w >= 1 { 1 } else { 0 };

        let x1 = x0 - i1 as f64 + G4;
        let y1 = y0 - j1 as f64 + G4;
        let z1 = z0 - k1 as f64 + G4;
        let w1 = w0 - l1 as f64 + G4;

        let x2 = x0 - i2 as f64 + 2.0 * G4;
        let y2 = y0 - j2 as f64 + 2.0 * G4;
        let z2 = z0 - k2 as f64 + 2.0 * G4;
        let w2 = w0 - l2 as f64 + 2.0 * G4;

        let x3 = x0 - i3 as f64 + 3.0 * G4;
        let y3 = y0 - j3 as f64 + 3.0 * G4;
        let z3 = z0 - k3 as f64 + 3.0 * G4;
        let w3 = w0 - l3 as f64 + 3.0 * G4;

        let x4 = x0 - 1.0 + 4.0 * G4;
        let y4 = y0 - 1.0 + 4.0 * G4;
        let z4 = z0 - 1.0 + 4.0 * G4;
        let w4 = w0 - 1.0 + 4.0 * G4;

        let ii = (i.rem_euclid(256.0) as usize) & 255;
        let jj = (j.rem_euclid(256.0) as usize) & 255;
        let kk = (k.rem_euclid(256.0) as usize) & 255;
        let ll = (l.rem_euclid(256.0) as usize) & 255;

        let gi0 = (self.perm
            [ii + self.perm[jj + self.perm[kk + self.perm[ll] as usize] as usize] as usize]
            % 32) as usize;
        let gi1 = (self.perm[ii
            + i1
            + self.perm[jj + j1 + self.perm[kk + k1 + self.perm[ll + l1] as usize] as usize]
                as usize]
            % 32) as usize;
        let gi2 = (self.perm[ii
            + i2
            + self.perm[jj + j2 + self.perm[kk + k2 + self.perm[ll + l2] as usize] as usize]
                as usize]
            % 32) as usize;
        let gi3 = (self.perm[ii
            + i3
            + self.perm[jj + j3 + self.perm[kk + k3 + self.perm[ll + l3] as usize] as usize]
                as usize]
            % 32) as usize;
        let gi4 = (self.perm[ii
            + 1
            + self.perm[jj + 1 + self.perm[kk + 1 + self.perm[ll + 1] as usize] as usize] as usize]
            % 32) as usize;

        let mut t0 = 0.6 - x0 * x0 - y0 * y0 - z0 * z0 - w0 * w0;
        let n0 = if t0 < 0.0 {
            0.0
        } else {
            t0 *= t0;
            t0 * t0
                * (GRAD4[gi0].0 * x0 + GRAD4[gi0].1 * y0 + GRAD4[gi0].2 * z0 + GRAD4[gi0].3 * w0)
        };

        let mut t1 = 0.6 - x1 * x1 - y1 * y1 - z1 * z1 - w1 * w1;
        let n1 = if t1 < 0.0 {
            0.0
        } else {
            t1 *= t1;
            t1 * t1
                * (GRAD4[gi1].0 * x1 + GRAD4[gi1].1 * y1 + GRAD4[gi1].2 * z1 + GRAD4[gi1].3 * w1)
        };

        let mut t2 = 0.6 - x2 * x2 - y2 * y2 - z2 * z2 - w2 * w2;
        let n2 = if t2 < 0.0 {
            0.0
        } else {
            t2 *= t2;
            t2 * t2
                * (GRAD4[gi2].0 * x2 + GRAD4[gi2].1 * y2 + GRAD4[gi2].2 * z2 + GRAD4[gi2].3 * w2)
        };

        let mut t3 = 0.6 - x3 * x3 - y3 * y3 - z3 * z3 - w3 * w3;
        let n3 = if t3 < 0.0 {
            0.0
        } else {
            t3 *= t3;
            t3 * t3
                * (GRAD4[gi3].0 * x3 + GRAD4[gi3].1 * y3 + GRAD4[gi3].2 * z3 + GRAD4[gi3].3 * w3)
        };

        let mut t4 = 0.6 - x4 * x4 - y4 * y4 - z4 * z4 - w4 * w4;
        let n4 = if t4 < 0.0 {
            0.0
        } else {
            t4 *= t4;
            t4 * t4
                * (GRAD4[gi4].0 * x4 + GRAD4[gi4].1 * y4 + GRAD4[gi4].2 * z4 + GRAD4[gi4].3 * w4)
        };

        27.0 * (n0 + n1 + n2 + n3 + n4)
    }

    /// Alias for [`Self::noise_2d`].
    pub fn noise2d(&self, x: f64, y: f64) -> f64 {
        self.noise_2d(x, y)
    }

    /// Alias for [`Self::noise_3d`].
    pub fn noise3d(&self, x: f64, y: f64, z: f64) -> f64 {
        self.noise_3d(x, y, z)
    }

    /// Alias for [`Self::noise_4d`].
    pub fn noise4d(&self, x: f64, y: f64, z: f64, w: f64) -> f64 {
        self.noise_4d(x, y, z, w)
    }
}

/// Generates 2D Simplex noise value in `[-1.0, 1.0]` for a given seed and `(x, y)` coordinates.
///
/// Ported directly from Remotion's `noise2D(seed, x, y)`.
#[inline]
pub fn noise2d(seed: impl Into<NoiseSeed>, x: f64, y: f64) -> f64 {
    SimplexNoise::new_2d(seed).noise_2d(x, y)
}

/// Generates 3D Simplex noise value in `[-1.0, 1.0]` for a given seed and `(x, y, z)` coordinates.
///
/// Ported directly from Remotion's `noise3D(seed, x, y, z)`.
#[inline]
pub fn noise3d(seed: impl Into<NoiseSeed>, x: f64, y: f64, z: f64) -> f64 {
    SimplexNoise::new_3d(seed).noise_3d(x, y, z)
}

/// Generates 4D Simplex noise value in `[-1.0, 1.0]` for a given seed and `(x, y, z, w)` coordinates.
///
/// Ported directly from Remotion's `noise4D(seed, x, y, z, w)`.
#[inline]
pub fn noise4d(seed: impl Into<NoiseSeed>, x: f64, y: f64, z: f64, w: f64) -> f64 {
    SimplexNoise::new_4d(seed).noise_4d(x, y, z, w)
}

/// Alias for [`noise2d`].
#[inline]
pub fn noise_2d(seed: impl Into<NoiseSeed>, x: f64, y: f64) -> f64 {
    noise2d(seed, x, y)
}

/// Alias for [`noise3d`].
#[inline]
pub fn noise_3d(seed: impl Into<NoiseSeed>, x: f64, y: f64, z: f64) -> f64 {
    noise3d(seed, x, y, z)
}

/// Alias for [`noise4d`].
#[inline]
pub fn noise_4d(seed: impl Into<NoiseSeed>, x: f64, y: f64, z: f64, w: f64) -> f64 {
    noise4d(seed, x, y, z, w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remotion_parity_values() {
        let n_zero = noise2d(1, 0.0, 0.0);
        assert_eq!(n_zero, 0.0);

        let n2d = noise2d("my-seed", 0.5, 0.5);
        assert!(
            (n2d - 0.3071565136272162).abs() < 1e-10,
            "Expected 0.3071565136272162, got {}",
            n2d
        );

        let n3d = noise3d("my-seed", 0.7, 0.5, 0.5);
        assert!(
            (n3d - 0.6402128434567901).abs() < 1e-10,
            "Expected 0.6402128434567901, got {}",
            n3d
        );

        let n4d = noise4d("my-seed", 0.7, 0.5, 0.5, 0.9);
        assert!(
            (n4d - 0.2714290963058814).abs() < 1e-10,
            "Expected 0.2714290963058814, got {}",
            n4d
        );
    }
}
