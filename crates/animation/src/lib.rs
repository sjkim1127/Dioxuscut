//! # dioxuscut-animation
//!
//! Remotion-compatible animation primitives for Dioxuscut.
//!
//! Provides:
//! - [`interpolate`] — maps a value from one range to another (Remotion `interpolate()`)
//! - [`spring`]       — physics-based spring animation (Remotion `spring()`)
//! - [`easing`]       — common easing functions (Remotion `Easing`)
//! - [`interpolate_colors`] — color interpolation
//! - [`transform`]    — CSS transform builder (`makeTransform`, `interpolateStyles`)

pub mod easing;
pub mod interpolate;
pub mod interpolate_colors;
pub mod spring;
pub mod transform;

pub use easing::{bezier, EasingFn};
pub use interpolate::{interpolate, ExtrapolateType, InterpolateOptions};
pub use interpolate_colors::interpolate_colors;
pub use spring::{spring, SpringConfig};
pub use transform::{
    interpolate_styles, make_transform, matrix, matrix3d, perspective, rotate, rotate3d, rotate_x,
    rotate_y, rotate_z, scale, scale3d, scale_x, scale_y, scale_z, skew, skew_x, skew_y, translate,
    translate3d, translate_x, translate_y, translate_z, StyleMap, StyleValue, TransformOp,
};
