//! `<Circle>` shape component.

use crate::render_svg::RenderSvg;
use dioxus::prelude::*;

/// Props for the `<Circle>` shape component.
#[derive(Props, Clone, PartialEq)]
pub struct CircleProps {
    /// Radius of the circle in pixels.
    #[props(default = 100.0)]
    pub radius: f64,
    /// Fill color.
    #[props(default = "#ffffff".to_string())]
    pub fill: String,
    /// Stroke color.
    #[props(default = "none".to_string())]
    pub stroke: String,
    /// Stroke width.
    #[props(default = 0.0)]
    pub stroke_width: f64,
    /// Opacity.
    #[props(default = 1.0)]
    pub opacity: f64,
    /// Custom CSS styles.
    #[props(default)]
    pub style: String,
}

/// Generates SVG path string and dimensions for a circle of given radius.
pub fn make_circle(radius: f64) -> (String, f64, f64) {
    let r = radius.max(0.0);
    let size = r * 2.0;
    // Circle drawn from top point (r, 0) down to (r, 2r) and back to (r, 0)
    let path = format!("M {r} 0 A {r} {r} 0 1 0 {r} {size} A {r} {r} 0 1 0 {r} 0 Z");
    (path, size, size)
}

/// Renders a procedural SVG Circle.
#[component]
pub fn Circle(props: CircleProps) -> Element {
    let (path, width, height) = make_circle(props.radius);

    rsx! {
        RenderSvg {
            path,
            width,
            height,
            fill: props.fill,
            stroke: props.stroke,
            stroke_width: props.stroke_width,
            opacity: props.opacity,
            style: props.style,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_circle_geometry() {
        let (path, w, h) = make_circle(50.0);
        assert_eq!(w, 100.0);
        assert_eq!(h, 100.0);
        assert!(path.contains("M 50 0"));
        assert!(path.contains("A 50 50"));
    }
}
