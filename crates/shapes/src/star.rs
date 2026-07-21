//! `<Star>` shape component.

use dioxus::prelude::*;
use crate::render_svg::RenderSvg;
use std::f64::consts::PI;

/// Props for the `<Star>` shape component.
#[derive(Props, Clone, PartialEq)]
pub struct StarProps {
    /// Number of star points (e.g., 5 for classic star).
    #[props(default = 5)]
    pub points: usize,
    /// Inner radius of the star points.
    #[props(default = 40.0)]
    pub inner_radius: f64,
    /// Outer radius of the star points.
    #[props(default = 100.0)]
    pub outer_radius: f64,
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

/// Generates SVG path string and dimensions for a star.
pub fn make_star(points: usize, inner_radius: f64, outer_radius: f64) -> (String, f64, f64) {
    let pts = points.max(3);
    let size = outer_radius * 2.0;
    let center_x = outer_radius;
    let center_y = outer_radius;

    let degree_increment = (PI * 2.0) / (pts as f64 * 2.0);
    let mut path_cmd = String::new();

    for i in 0..(pts * 2) {
        let r = if i % 2 == 0 { outer_radius } else { inner_radius };
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

/// Renders a procedural SVG Star.
#[component]
pub fn Star(props: StarProps) -> Element {
    let (path, width, height) = make_star(props.points, props.inner_radius, props.outer_radius);

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
    fn test_make_star_geometry() {
        let (path, w, h) = make_star(5, 40.0, 100.0);
        assert_eq!(w, 200.0);
        assert_eq!(h, 200.0);
        assert!(path.starts_with("M 100.0000 0.0000"));
        assert!(path.ends_with("Z"));
    }
}
