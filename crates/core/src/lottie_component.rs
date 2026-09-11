//! `<Lottie>` component for headless and web Lottie animation playback.
//!
//! Matches Remotion's `@remotion/lottie` `<Lottie>` component.

use dioxus::prelude::*;
use dioxuscut_rasterizer::LoopBehavior;

/// Props for the `<Lottie>` component.
#[derive(Props, Clone, PartialEq)]
pub struct LottieProps {
    /// File path or asset URI to the Lottie JSON file.
    pub src: String,
    /// Rendered width in pixels.
    #[props(default = 200.0)]
    pub width: f64,
    /// Rendered height in pixels.
    #[props(default = 200.0)]
    pub height: f64,
    /// Playback rate multiplier (1.0 = normal speed).
    #[props(default = 1.0)]
    pub playback_rate: f64,
    /// Animation looping behavior.
    #[props(default = LoopBehavior::Loop)]
    pub loop_behavior: LoopBehavior,
    /// Additional CSS styles.
    #[props(default)]
    pub style: String,
    /// Additional CSS classes.
    #[props(default)]
    pub class: String,
}

/// Renders a Lottie vector animation on the timeline.
#[component]
pub fn Lottie(props: LottieProps) -> Element {
    let combined_style = format!(
        "width: {}px; height: {}px; display: inline-block; overflow: hidden; {}",
        props.width, props.height, props.style
    );

    rsx! {
        div {
            class: "{props.class}",
            style: "{combined_style}",
            "data-dioxuscut-lottie": "{props.src}",
            "data-playback-rate": "{props.playback_rate}",
        }
    }
}
