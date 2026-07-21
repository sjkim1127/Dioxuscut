//! Timeline context — provides frame position and video config to all
//! descendant Dioxus components via context injection.
//!
//! Equivalent to Remotion's `TimelineContext` + `SequenceContext`.

use crate::types::VideoConfig;

/// The current playback position in the composition, relative to the
/// **component's own origin** (i.e., already offset by any parent `Sequence`).
///
/// This is the value returned by `use_current_frame()`.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineContext {
    /// Frame number relative to the component root (0-indexed).
    pub frame: u32,
    /// Absolute frame in the root composition (for internal use).
    pub(crate) absolute_frame: u32,
}

impl TimelineContext {
    /// Create a new context starting at `frame`.
    pub fn new(frame: u32) -> Self {
        Self {
            frame,
            absolute_frame: frame,
        }
    }

    /// Create a child context offset by `from_frame` (for `Sequence`).
    pub fn with_offset(absolute_frame: u32, offset: u32) -> Self {
        let frame = absolute_frame.saturating_sub(offset);
        Self {
            frame,
            absolute_frame,
        }
    }
}

/// Context that carries the `VideoConfig` (width, height, fps, duration).
///
/// This is the value returned by `use_video_config()`.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoConfigContext(pub VideoConfig);
