//! `<Fade>` transition — crossfade between transparent and opaque.
//!
//! Equivalent to Remotion `<TransitionSeries>` with `fade()` presentation.
//!
//! # Example
//! ```rust,ignore
//! use dioxuscut_transitions::Fade;
//!
//! fn MyScene() -> Element {
//!     rsx! {
//!         // Fade in over 20 frames from frame 0
//!         Fade { enter_duration: 20, from_frame: 0,
//!             SceneContent {}
//!         }
//!     }
//! }
//! ```

use dioxus::prelude::*;
use dioxuscut_animation::interpolate::{interpolate, ExtrapolateType, InterpolateOptions};
use dioxuscut_core::hooks::use_current_frame;

/// Props for the `<Fade>` component.
#[derive(Props, Clone, PartialEq)]
pub struct FadeProps {
    /// Duration (in frames) of the fade-in. `0` = instant.
    #[props(default = 20)]
    pub enter_duration: u32,
    /// Duration (in frames) of the fade-out. `0` = no fade-out.
    #[props(default = 0)]
    pub exit_duration: u32,
    /// Total duration of this segment — required when `exit_duration > 0`.
    #[props(default)]
    pub duration_in_frames: Option<u32>,
    /// Children to fade.
    pub children: Element,
}

/// Crossfade wrapper — fades children in and optionally out.
#[component]
pub fn Fade(props: FadeProps) -> Element {
    let frame = use_current_frame() as f64;
    let enter = props.enter_duration as f64;

    // Opacity based on enter fade
    let enter_opacity = if enter > 0.0 {
        interpolate(
            frame,
            &[0.0, enter],
            &[0.0, 1.0],
            InterpolateOptions {
                extrapolate_left: ExtrapolateType::Clamp,
                extrapolate_right: ExtrapolateType::Clamp,
                ..Default::default()
            },
        )
    } else {
        1.0
    };

    // Opacity based on exit fade
    let exit_opacity =
        if let (Some(total), exit) = (props.duration_in_frames, props.exit_duration as f64) {
            if exit > 0.0 {
                let total_f = total as f64;
                interpolate(
                    frame,
                    &[total_f - exit, total_f],
                    &[1.0, 0.0],
                    InterpolateOptions {
                        extrapolate_left: ExtrapolateType::Clamp,
                        extrapolate_right: ExtrapolateType::Clamp,
                        ..Default::default()
                    },
                )
            } else {
                1.0
            }
        } else {
            1.0
        };

    let opacity = enter_opacity.min(exit_opacity);

    rsx! {
        div {
            style: "opacity: {opacity:.4}; position: absolute; top: 0; left: 0; right: 0; bottom: 0;",
            {props.children}
        }
    }
}
