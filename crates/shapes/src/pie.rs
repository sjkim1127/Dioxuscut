//! `<Pie>` shape component (pie chart / arc slices).

use dioxus::prelude::*;
use crate::render_svg::RenderSvg;
use std::f64::consts::PI;

/// Props for the `<Pie>` shape component.
#[derive(Props, Clone, PartialEq)]
pub struct PieProps {
    /// Radius of the pie chart in pixels.
    #[props(default = 100.0)]
    pub radius: f64,
    /// Progress ratio (0.0 = 0%, 0.5 = 50%, 1.0 = 100%).
    #[props(default = 1.0)]
    pub progress: f64,
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

/// Generates SVG path string and dimensions for a pie slice.
pub fn make_pie(radius: f64, progress: f64) -> (String, f64, f64) {
    let r = radius.max(0.0);
    let size = r * 2.0;
    let clamped_p = progress.clamp(0.0, 1.0);

    if clamped_p <= 0.0 {
        return (String::new(), size, size);
    }

    if clamped_p >= 1.0 {
        let path = format!("M {r} 0 A {r} {r} 0 1 0 {r} {size} A {r} {r} 0 1 0 {r} 0 Z");
        return (path, size, size);
    }

    let start_angle = -PI / 2.0;
    let end_angle = start_angle + clamped_p * (PI * 2.0);

    let x = r + r * end_angle.cos();
    let y = r + r * end_angle.sin();

    let large_arc_flag = if clamped_p > 0.5 { 1 } else { 0 };

    let path = format!("M {r} {r} L {r} 0 A {r} {r} 0 {large_arc_flag} 1 {x:.4} {y:.4} Z");
    (path, size, size)
}

/// Renders a procedural SVG Pie.
#[component]
pub fn Pie(props: PieProps) -> Element {
    let (path, width, height) = make_pie(props.radius, props.progress);

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
    fn test_make_pie_quarter() {
        let (path, w, h) = make_pie(100.0, 0.25);
        assert_eq!(w, 200.0);
        assert_eq!(h, 200.0);
        assert!(path.contains("M 100 100 L 100 0 A 100 100 0 0 1 200.0000 100.0000 Z"));
    }

    #[test]
    fn test_make_pie_half() {
        let (path, _, _) = make_pie(100.0, 0.5);
        assert!(path.contains("M 100 100 L 100 0 A 100 100 0 0 1 100.0000 200.0000 Z"));
    }
}
