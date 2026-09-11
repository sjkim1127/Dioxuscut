//! `<Ellipse>` shape component.

use crate::render_svg::RenderSvg;
use crate::shape_output::ShapeOutput;
use dioxus::prelude::*;

/// Props for the `<Ellipse>` shape component.
#[derive(Props, Clone, PartialEq)]
pub struct EllipseProps {
    /// Horizontal radius in pixels.
    #[props(default = 100.0)]
    pub rx: f64,
    /// Vertical radius in pixels.
    #[props(default = 60.0)]
    pub ry: f64,
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

/// Generates SVG path and bounding dimensions for an ellipse of radii `rx` and `ry`.
pub fn make_ellipse(rx: f64, ry: f64) -> ShapeOutput {
    let rx = rx.max(0.0);
    let ry = ry.max(0.0);
    let width = rx * 2.0;
    let height = ry * 2.0;
    let path = format!("M {rx} 0 A {rx} {ry} 0 1 0 {rx} {height} A {rx} {ry} 0 1 0 {rx} 0 Z");
    ShapeOutput::new(path, width, height, format!("{rx} {ry}"))
}

/// Renders a procedural SVG Ellipse.
#[component]
pub fn Ellipse(props: EllipseProps) -> Element {
    let shape = make_ellipse(props.rx, props.ry);

    rsx! {
        RenderSvg {
            path: shape.path,
            width: shape.width,
            height: shape.height,
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
    fn test_make_ellipse_dimensions() {
        let shape = make_ellipse(80.0, 40.0);
        assert_eq!(shape.width, 160.0);
        assert_eq!(shape.height, 80.0);
        assert!(shape.path.contains("M 80 0"));
        assert!(shape.path.contains("A 80 40"));
        assert_eq!(shape.transform_origin, "80 40");
    }
}
