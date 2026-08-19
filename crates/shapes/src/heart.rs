//! `<Heart>` shape component and path generator.

use crate::render_svg::RenderSvg;
use crate::shape_output::ShapeOutput;
use dioxus::prelude::*;

/// Props for the `<Heart>` shape component.
#[derive(Props, Clone, PartialEq)]
pub struct HeartProps {
    /// Bounding box width in pixels.
    #[props(default = 100.0)]
    pub width: f64,
    /// Bounding box height in pixels.
    #[props(default = 100.0)]
    pub height: f64,
    /// Fill color.
    #[props(default = "#ffffff".to_string())]
    pub fill: String,
    /// Stroke color.
    #[props(default = "none".to_string())]
    pub stroke: String,
    /// Stroke width in pixels.
    #[props(default = 0.0)]
    pub stroke_width: f64,
    /// Opacity (0.0 to 1.0).
    #[props(default = 1.0)]
    pub opacity: f64,
    /// Custom CSS styles.
    #[props(default)]
    pub style: String,
}

/// Generates parametric SVG path data and metadata for a heart shape.
pub fn make_heart(width: f64, height: f64) -> ShapeOutput {
    let w = width.max(0.0);
    let h = height.max(0.0);

    if w == 0.0 || h == 0.0 {
        return ShapeOutput::new(String::new(), w, h, format!("{} {}", w / 2.0, h / 2.0));
    }

    let bottom_cp_x = (23.0 / 110.0) * w;
    let bottom_cp_y = (69.0 / 100.0) * h;
    let bottom_left_cp_y = (60.0 / 100.0) * h;
    let top_left_cp = (13.0 / 100.0) * h;
    let top_bezier_w = (29.0 / 110.0) * w;
    let top_right_cp_x = (15.0 / 110.0) * w;
    let inner_cp_x = (5.0 / 110.0) * w;
    let inner_cp_y = (7.0 / 100.0) * h;
    let depth = (17.0 / 100.0) * h;

    let half_w = w / 2.0;
    let quarter_w = w / 4.0;
    let three_quarter_w = 3.0 * w / 4.0;
    let quarter_h = h / 4.0;

    let path = format!(
        "M {half_w} {h} \
         C {} {bottom_cp_y} 0 {bottom_left_cp_y} 0 {quarter_h} \
         C 0 {top_left_cp} {} 0 {quarter_w} 0 \
         C {} 0 {} {inner_cp_y} {half_w} {depth} \
         C {} {inner_cp_y} {} 0 {three_quarter_w} 0 \
         C {} 0 {w} {top_left_cp} {w} {quarter_h} \
         C {w} {bottom_left_cp_y} {} {bottom_cp_y} {half_w} {h} Z",
        half_w - bottom_cp_x,
        quarter_w - top_bezier_w / 2.0,
        quarter_w + top_bezier_w / 2.0,
        half_w - inner_cp_x,
        half_w + inner_cp_x,
        half_w + top_right_cp_x,
        three_quarter_w + top_bezier_w / 2.0,
        half_w + bottom_cp_x,
    );

    ShapeOutput::new(path, w, h, format!("{} {}", half_w, h / 2.0))
}

/// Renders a procedural SVG Heart.
#[component]
pub fn Heart(props: HeartProps) -> Element {
    let shape = make_heart(props.width, props.height);

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
    fn test_make_heart_geometry() {
        let heart = make_heart(110.0, 100.0);
        assert_eq!(heart.width, 110.0);
        assert_eq!(heart.height, 100.0);
        assert_eq!(heart.transform_origin, "55 50");
        assert!(heart.path.starts_with("M 55 100"));
        assert!(heart.path.ends_with("Z"));
        assert!(heart.path.contains("C 32 69 0 60 0 25"));
    }

    #[test]
    fn test_make_heart_zero_dimension() {
        let heart = make_heart(0.0, 50.0);
        assert_eq!(heart.path, "");
        assert_eq!(heart.width, 0.0);
        assert_eq!(heart.height, 50.0);
    }

    #[test]
    fn test_make_heart_negative_clamping() {
        let heart = make_heart(-20.0, -30.0);
        assert_eq!(heart.path, "");
        assert_eq!(heart.width, 0.0);
        assert_eq!(heart.height, 0.0);
    }

    #[test]
    fn test_make_heart_custom_aspect_ratio() {
        let heart = make_heart(220.0, 100.0);
        assert_eq!(heart.width, 220.0);
        assert_eq!(heart.height, 100.0);
        assert_eq!(heart.transform_origin, "110 50");
        assert!(heart.path.starts_with("M 110 100"));
    }
}
