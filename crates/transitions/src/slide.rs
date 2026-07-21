//! `<Slide>` transition — slides content in/out from a given direction.

use dioxus::prelude::*;
use dioxuscut_animation::interpolate::{interpolate, ExtrapolateType, InterpolateOptions};
use dioxuscut_core::hooks::use_current_frame;

/// Direction the content slides in from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SlideDirection {
    #[default]
    FromRight,
    FromLeft,
    FromTop,
    FromBottom,
}

/// Props for `<Slide>`.
#[derive(Props, Clone, PartialEq)]
pub struct SlideProps {
    /// Duration of the slide-in (frames).
    #[props(default = 20)]
    pub enter_duration: u32,
    /// Direction the content enters from.
    #[props(default)]
    pub direction: SlideDirection,
    /// Children to slide in.
    pub children: Element,
}

/// Slides children in from the specified direction.
#[component]
pub fn Slide(props: SlideProps) -> Element {
    let frame = use_current_frame() as f64;
    let enter = props.enter_duration as f64;

    let t = if enter > 0.0 {
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

    // Translate by 100% in the enter direction, lerp to 0%
    let translate_style = match props.direction {
        SlideDirection::FromRight => format!("translateX({:.2}%)", (1.0 - t) * 100.0),
        SlideDirection::FromLeft => format!("translateX({:.2}%)", (t - 1.0) * 100.0),
        SlideDirection::FromBottom => format!("translateY({:.2}%)", (1.0 - t) * 100.0),
        SlideDirection::FromTop => format!("translateY({:.2}%)", (t - 1.0) * 100.0),
    };

    rsx! {
        div {
            style: "
                position: absolute; top: 0; left: 0; right: 0; bottom: 0;
                transform: {translate_style};
            ",
            {props.children}
        }
    }
}
