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

    /// Create a child context offset from its parent's local timeline.
    ///
    /// The root absolute frame is preserved so nested sequences do not
    /// accidentally reinterpret a relative `from` value as a root offset.
    pub fn offset_from(parent: &Self, offset: u32) -> Self {
        let frame = parent.frame.saturating_sub(offset);
        Self {
            frame,
            absolute_frame: parent.absolute_frame,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_offsets_use_the_parent_local_frame() {
        let root = TimelineContext::new(45);
        let parent = TimelineContext::offset_from(&root, 30);
        let child = TimelineContext::offset_from(&parent, 10);

        assert_eq!(parent.frame, 15);
        assert_eq!(child.frame, 5);
        assert_eq!(child.absolute_frame, 45);
    }
}

/// Context that carries the `VideoConfig` (width, height, fps, duration).
///
/// This is the value returned by `use_video_config()`.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoConfigContext(pub VideoConfig);
