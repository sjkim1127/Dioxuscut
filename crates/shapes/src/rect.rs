//! `<Rect>` shape component.

use crate::render_svg::RenderSvg;
use dioxus::prelude::*;

/// Props for the `<Rect>` shape component.
#[derive(Props, Clone, PartialEq)]
pub struct RectProps {
    /// Width of the rectangle in pixels.
    #[props(default = 200.0)]
    pub width: f64,
    /// Height of the rectangle in pixels.
    #[props(default = 100.0)]
    pub height: f64,
    /// Corner radius for rounded corners (CSS border-radius equivalent).
    #[props(default = 0.0)]
    pub corner_radius: f64,
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

/// Generates SVG path string and dimensions for a rectangle.
pub fn make_rect(width: f64, height: f64, corner_radius: f64) -> (String, f64, f64) {
    let w = width.max(0.0);
    let h = height.max(0.0);
    let max_r = (w / 2.0).min(h / 2.0);
    let r = corner_radius.clamp(0.0, max_r);

    let path = if r <= 0.0 {
        format!("M 0 0 L {w} 0 L {w} {h} L 0 {h} Z")
    } else {
        format!(
            "M {r} 0 L {} 0 A {r} {r} 0 0 1 {w} {r} L {w} {} A {r} {r} 0 0 1 {} {h} L {r} {h} A {r} {r} 0 0 1 0 {} L 0 {r} A {r} {r} 0 0 1 {r} 0 Z",
            w - r,
            h - r,
            w - r,
            h - r
        )
    };

    (path, w, h)
}

/// Renders a procedural SVG Rectangle.
#[component]
pub fn Rect(props: RectProps) -> Element {
    let (path, width, height) = make_rect(props.width, props.height, props.corner_radius);

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
    fn test_make_rect_flat() {
        let (path, w, h) = make_rect(200.0, 100.0, 0.0);
        assert_eq!(w, 200.0);
        assert_eq!(h, 100.0);
        assert_eq!(path, "M 0 0 L 200 0 L 200 100 L 0 100 Z");
    }

    #[test]
    fn test_make_rect_rounded() {
        let (path, w, h) = make_rect(200.0, 100.0, 10.0);
        assert_eq!(w, 200.0);
        assert_eq!(h, 100.0);
        assert!(path.contains("M 10 0 L 190 0 A 10 10"));
    }
}
