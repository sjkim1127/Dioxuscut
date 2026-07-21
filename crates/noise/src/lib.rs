//! Dioxuscut Noise — Procedural Simplex 2D/3D/4D noise generation and animated organic noise backgrounds.
//!
//! Ported from `@remotion/noise`:
//! - [`noise_2d`]
//! - [`noise_3d`]
//! - [`noise_4d`]
//! - [`hash_seed`]
//! - [`NoiseBackground`]

pub mod seed;
pub mod simplex;
pub mod noise_bg;

pub use seed::{hash_seed, seed_to_float};
pub use simplex::{noise_2d, noise_3d, noise_4d};
pub use noise_bg::{NoiseBackground, NoiseBackgroundProps};
