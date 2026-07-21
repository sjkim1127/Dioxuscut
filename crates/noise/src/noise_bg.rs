//! Dioxus component for dynamic organic noise background rendering.

use crate::simplex::noise_3d;
use dioxus::prelude::*;
use dioxuscut_core::hooks::use_current_frame;

/// Props for the `<NoiseBackground>` component.
#[derive(Props, Clone, PartialEq)]
pub struct NoiseBackgroundProps {
    /// Seed identifier for noise generator.
    #[props(default = "default-noise-seed".to_string())]
    pub seed: String,
    /// Base background color (e.g. `"#0b0d19"`).
    #[props(default = "#0b0d19".to_string())]
    pub base_color: String,
    /// Secondary accent color for noise wave (e.g. `"#6c63ff"`).
    #[props(default = "#6c63ff".to_string())]
    pub accent_color: String,
    /// Noise frequency scale.
    #[props(default = 0.05)]
    pub speed: f64,
    /// Additional CSS style.
    #[props(default)]
    pub style: String,
}

/// Dioxus component for rendering animated procedural organic noise backgrounds.
#[component]
pub fn NoiseBackground(props: NoiseBackgroundProps) -> Element {
    let frame = use_current_frame();
    let time = frame as f64 * props.speed;

    // Generate dynamic noise offset
    let n1 = noise_3d(&props.seed, 0.5, 0.5, time);
    let n2 = noise_3d(&props.seed, 1.5, 1.5, time * 0.8);

    let opacity = ((n1 + 1.0) / 2.0 * 0.5 + 0.25).clamp(0.0, 1.0);
    let blur_radius = (20.0 + n2 * 10.0).max(5.0);

    let container_style = format!(
        "position: absolute; top: 0; left: 0; width: 100%; height: 100%; \
         background-color: {}; overflow: hidden; {};",
        props.base_color, props.style
    );

    let wave_style = format!(
        "position: absolute; width: 140%; height: 140%; top: -20%; left: -20%; \
         background: radial-gradient(circle at {}% {}%, {} 0%, transparent 70%); \
         opacity: {:.4}; filter: blur({:.1}px);",
        50.0 + n1 * 30.0,
        50.0 + n2 * 30.0,
        props.accent_color,
        opacity,
        blur_radius
    );

    rsx! {
        div {
            style: "{container_style}",
            div {
                style: "{wave_style}",
            }
        }
    }
}
