//! SVG wrapper component for procedural shapes.

use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct RenderSvgProps {
    /// SVG path `d` instruction string.
    pub path: String,
    /// Bounding box width in pixels.
    pub width: f64,
    /// Bounding box height in pixels.
    pub height: f64,
    /// Fill color string (e.g., `"#ff0055"`, `"rgba(255,0,0,0.5)"`, `"none"`).
    #[props(default = "#ffffff".to_string())]
    pub fill: String,
    /// Stroke color string.
    #[props(default = "none".to_string())]
    pub stroke: String,
    /// Stroke width in pixels.
    #[props(default = 0.0)]
    pub stroke_width: f64,
    /// Opacity (0.0 to 1.0).
    #[props(default = 1.0)]
    pub opacity: f64,
    /// Additional CSS style string.
    #[props(default)]
    pub style: String,
}

/// Generic SVG shape renderer.
#[component]
pub fn RenderSvg(props: RenderSvgProps) -> Element {
    let view_box = format!("0 0 {} {}", props.width, props.height);
    let combined_style = format!(
        "width: {}px; height: {}px; overflow: visible; opacity: {}; {}",
        props.width, props.height, props.opacity, props.style
    );

    rsx! {
        svg {
            view_box: "{view_box}",
            style: "{combined_style}",
            path {
                d: "{props.path}",
                fill: "{props.fill}",
                stroke: "{props.stroke}",
                stroke_width: "{props.stroke_width}",
            }
        }
    }
}
