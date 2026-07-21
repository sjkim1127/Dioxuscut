//! `use_current_frame` hook — returns the current render frame.
//!
//! Equivalent to Remotion's `useCurrentFrame()`.
//!
//! # Usage
//! ```rust,ignore
//! use dioxuscut_core::hooks::use_current_frame;
//!
//! fn MyComponent() -> Element {
//!     let frame = use_current_frame();
//!     rsx! { div { "Frame: {frame}" } }
//! }
//! ```

use crate::timeline::context::TimelineContext;
use dioxus::prelude::*;

/// Returns the current frame number for this component.
///
/// The frame is relative to the component's position in the timeline
/// (i.e., already offset by any enclosing `<Sequence>`).
///
/// Returns `0` if called outside a `<Composition>` context.
pub fn use_current_frame() -> u32 {
    let ctx = use_context::<TimelineContext>();
    ctx.frame
}
