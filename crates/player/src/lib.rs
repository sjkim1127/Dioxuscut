//! # dioxuscut-player
//!
//! Interactive `<Player>` component — renders a composition with playback
//! controls (play/pause/seek/scrub).
//!
//! Equivalent to the `@remotion/player` package.

pub mod controls;
pub mod player;

pub use controls::Controls;
pub use player::{Player, PlayerProps};
