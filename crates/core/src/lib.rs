//! # dioxuscut-core
//!
//! Core Dioxus components and hooks for Dioxuscut.
//!
//! Provides a Rust/Dioxus equivalent of `remotion/core`:
//!
//! ## Components
//! - [`Composition`] — top-level video definition
//! - [`Sequence`]    — time-sliced sub-composition
//! - [`AbsoluteFill`] — full-size absolute overlay
//! - [`Freeze`]       — pause a subtree at a specific frame
//!
//! ## Hooks
//! - [`use_current_frame`] — returns the current render frame
//! - [`use_video_config`]  — returns [`VideoConfig`] for the composition
//!
//! ## Re-exports
//! - Animation primitives from [`dioxuscut_animation`]

pub mod absolute_fill;
pub mod composition;
pub mod freeze;
pub mod hooks;
pub mod scene;
pub mod sequence;
pub mod timeline;
pub mod types;

// ── Public re-exports ─────────────────────────────────────────────────────────
pub use absolute_fill::AbsoluteFill;
pub use composition::{Composition, CompositionProps};
pub use freeze::Freeze;
pub use hooks::{use_current_frame, use_video_config};
pub use scene::SceneAbsoluteFill;
pub use sequence::{Sequence, SequenceProps};
pub use timeline::context::{TimelineContext, VideoConfigContext};
pub use types::VideoConfig;

// Re-export animation crate for convenience
pub use dioxuscut_animation as animation;
