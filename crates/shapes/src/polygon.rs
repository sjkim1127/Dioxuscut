//! `<Polygon>` shape component (regular N-sided polygons).

use crate::render_svg::RenderSvg;
use dioxus::prelude::*;
use std::f64::consts::PI;

/// Props for the `<Polygon>` shape component.
#[derive(Props, Clone, PartialEq)]
pub struct PolygonProps {
    /// Number of sides/vertices (e.g., 6 for hexagon, 8 for octagon).
    #[props(default = 6)]
    pub points: usize,
    /// Outer radius of the polygon in pixels.
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

/// Generates SVG path string and dimensions for a regular N-sided polygon.
pub fn make_polygon(points: usize, radius: f64) -> (String, f64, f64) {
    let pts = points.max(3);
    let r = radius.max(0.0);
    let size = r * 2.0;
    let center_x = r;
    let center_y = r;

    let degree_increment = (PI * 2.0) / (pts as f64);
    let mut path_cmd = String::new();

    for i in 0..pts {
        let angle = degree_increment * (i as f64) - PI / 2.0;
        let x = center_x + r * angle.cos();
        let y = center_y + r * angle.sin();

        if i == 0 {
            path_cmd.push_str(&format!("M {x:.4} {y:.4}"));
        } else {
            path_cmd.push_str(&format!(" L {x:.4} {y:.4}"));
        }
    }
    path_cmd.push_str(" Z");

    (path_cmd, size, size)
}

/// Renders a procedural SVG Polygon.
#[component]
pub fn Polygon(props: PolygonProps) -> Element {
    let (path, width, height) = make_polygon(props.points, props.radius);

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
    fn test_make_hexagon_geometry() {
        let (path, w, h) = make_polygon(6, 100.0);
        assert_eq!(w, 200.0);
        assert_eq!(h, 200.0);
        assert!(path.starts_with("M 100.0000 0.0000"));
        assert!(path.ends_with("Z"));
    }
}
