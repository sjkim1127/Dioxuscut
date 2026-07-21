//! # dioxuscut-animation
//!
//! Remotion-compatible animation primitives for Dioxuscut.
//!
//! Provides:
//! - [`interpolate`] — maps a value from one range to another (Remotion `interpolate()`)
//! - [`spring`]       — physics-based spring animation (Remotion `spring()`)
//! - [`easing`]       — common easing functions (Remotion `Easing`)
//! - [`interpolate_colors`] — color interpolation

pub mod easing;
pub mod interpolate;
pub mod interpolate_colors;
pub mod spring;

pub use easing::{bezier, EasingFn};
pub use interpolate::{interpolate, ExtrapolateType, InterpolateOptions};
pub use interpolate_colors::interpolate_colors;
pub use spring::{spring, SpringConfig};
