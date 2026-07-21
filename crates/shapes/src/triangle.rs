//! `<Triangle>` shape component.

use dioxus::prelude::*;
use crate::render_svg::RenderSvg;

/// Props for the `<Triangle>` shape component.
#[derive(Props, Clone, PartialEq)]
pub struct TriangleProps {
    /// Side length of the equilateral triangle in pixels.
    #[props(default = 100.0)]
    pub length: f64,
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

/// Generates SVG path string and dimensions for an equilateral triangle.
pub fn make_triangle(length: f64) -> (String, f64, f64) {
    let l = length.max(0.0);
    let h = l * (3.0_f64.sqrt() / 2.0);
    let half_l = l / 2.0;

    let path = format!("M {half_l:.4} 0 L {l:.4} {h:.4} L 0 {h:.4} Z");
    (path, l, h)
}

/// Renders a procedural SVG Triangle.
#[component]
pub fn Triangle(props: TriangleProps) -> Element {
    let (path, width, height) = make_triangle(props.length);

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
    fn test_make_triangle_geometry() {
        let (path, w, h) = make_triangle(100.0);
        assert_eq!(w, 100.0);
        assert!((h - 86.6025).abs() < 0.01);
        assert!(path.contains("M 50.0000 0"));
        assert!(path.contains("Z"));
    }
}
