//! `<Loop>` component — repeats a subtree across time.
//!
//! Matches Remotion's `<Loop>`.
//!
//! Renders its children repeatedly for `times` iterations (or indefinitely).
//! Inside the children, `use_current_frame()` returns a local frame `[0..duration_in_frames)`.

use crate::sequence::SequenceLayout;
use crate::timeline::context::TimelineContext;
use dioxus::prelude::*;

/// Props for the `<Loop>` component.
#[derive(Props, Clone, PartialEq)]
pub struct LoopProps {
    /// How many frames each loop iteration lasts.
    pub duration_in_frames: u32,

    /// How many times to repeat. None means loop indefinitely (until parent ends).
    #[props(default)]
    pub times: Option<u32>,

    /// Layout mode for the loop container div.
    #[props(default)]
    pub layout: SequenceLayout,

    /// Children to loop.
    pub children: Element,
}

/// Repeats its children across time.
#[component]
pub fn Loop(props: LoopProps) -> Element {
    let parent_signal = use_context::<Signal<TimelineContext>>();
    let parent_ctx = parent_signal.read().clone();
    let parent_frame = parent_ctx.frame;
    let duration = props.duration_in_frames;

    if duration == 0 {
        return rsx! {};
    }

    let max_frames = props
        .times
        .map(|t| t.saturating_mul(duration))
        .unwrap_or(u32::MAX);

    if parent_frame >= max_frames {
        return rsx! {};
    }

    let iteration = parent_frame / duration;
    let start_offset = iteration.saturating_mul(duration);
    let child_ctx = TimelineContext::offset_from(&parent_ctx, start_offset);

    let style = match props.layout {
        SequenceLayout::AbsoluteFill => "position: absolute; top: 0; left: 0; right: 0; bottom: 0;",
        SequenceLayout::None => "",
    };

    rsx! {
        div {
            style: "{style}",
            LoopInner {
                ctx: child_ctx,
                children: props.children,
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct LoopInnerProps {
    ctx: TimelineContext,
    children: Element,
}

#[component]
fn LoopInner(props: LoopInnerProps) -> Element {
    let mut timeline = use_context_provider(|| Signal::new(props.ctx.clone()));
    if *timeline.peek() != props.ctx {
        timeline.set(props.ctx);
    }
    rsx! { {props.children} }
}
