//! # dioxuscut-transitions
//!
//! Transition components — wrap children in animated reveal/exit effects.
//!
//! Equivalent to `@remotion/transitions`.

pub mod fade;
pub mod slide;

pub use fade::{Fade, FadeProps};
pub use slide::{Slide, SlideProps, SlideDirection};
