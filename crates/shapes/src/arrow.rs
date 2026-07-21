//! `<Arrow>` shape component.

use crate::render_svg::RenderSvg;
use dioxus::prelude::*;

/// Props for the `<Arrow>` shape component.
#[derive(Props, Clone, PartialEq)]
pub struct ArrowProps {
    /// Total length of the arrow in pixels.
    #[props(default = 200.0)]
    pub length: f64,
    /// Thickness of the arrow stem in pixels.
    #[props(default = 20.0)]
    pub thickness: f64,
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

/// Generates SVG path string and dimensions for a right-pointing arrow.
pub fn make_arrow(length: f64, thickness: f64) -> (String, f64, f64) {
    let l = length.max(20.0);
    let t = thickness.max(4.0);
    let head_l = (t * 2.0).min(l * 0.4);
    let head_w = t * 2.5;
    let height = head_w;
    let center_y = height / 2.0;

    let stem_top = center_y - t / 2.0;
    let stem_bot = center_y + t / 2.0;
    let head_x = l - head_l;

    let path = format!(
        "M 0 {stem_top:.4} L {head_x:.4} {stem_top:.4} L {head_x:.4} 0 L {l:.4} {center_y:.4} L {head_x:.4} {height:.4} L {head_x:.4} {stem_bot:.4} L 0 {stem_bot:.4} Z"
    );

    (path, l, height)
}

/// Renders a procedural SVG Arrow.
#[component]
pub fn Arrow(props: ArrowProps) -> Element {
    let (path, width, height) = make_arrow(props.length, props.thickness);

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
    fn test_make_arrow_geometry() {
        let (path, w, h) = make_arrow(200.0, 20.0);
        assert_eq!(w, 200.0);
        assert_eq!(h, 50.0);
        assert!(path.starts_with("M 0 15.0000"));
        assert!(path.contains("L 200.0000 25.0000"));
    }
}
