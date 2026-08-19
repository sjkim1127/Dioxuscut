//! `<Callout>` speech bubble shape component and path generator.

use crate::render_svg::RenderSvg;
use crate::shape_output::ShapeOutput;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// Direction in which the callout pointer/tail points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CalloutDirection {
    /// Pointer extends downwards from the bottom edge.
    #[default]
    Down,
    /// Pointer extends upwards from the top edge.
    Up,
    /// Pointer extends leftwards from the left edge.
    Left,
    /// Pointer extends rightwards from the right edge.
    Right,
}

/// Props for the `<Callout>` shape component.
#[derive(Props, Clone, PartialEq)]
pub struct CalloutProps {
    /// Bounding width of the callout body rectangle in pixels.
    #[props(default = 200.0)]
    pub width: f64,
    /// Bounding height of the callout body rectangle in pixels.
    #[props(default = 120.0)]
    pub height: f64,
    /// Length of the pointer/tail in pixels.
    #[props(default = 30.0)]
    pub pointer_length: f64,
    /// Direction that the pointer extends towards.
    #[props(default = CalloutDirection::Down)]
    pub pointer_direction: CalloutDirection,
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

/// Generates parametric SVG path data and metadata for a speech bubble callout.
pub fn make_callout(
    width: f64,
    height: f64,
    pointer_length: f64,
    pointer_direction: CalloutDirection,
) -> ShapeOutput {
    let w = width.max(0.0);
    let h = height.max(0.0);
    let ptr_len = pointer_length.max(0.0);

    if w == 0.0 || h == 0.0 {
        let (total_w, total_h) = match pointer_direction {
            CalloutDirection::Down | CalloutDirection::Up => (w, h + ptr_len),
            CalloutDirection::Left | CalloutDirection::Right => (w + ptr_len, h),
        };
        return ShapeOutput::new(
            String::new(),
            total_w,
            total_h,
            format!("{} {}", total_w / 2.0, total_h / 2.0),
        );
    }

    match pointer_direction {
        CalloutDirection::Down => {
            let base_w = (w * 0.2).clamp(10.0, 60.0).min(w);
            let total_w = w;
            let total_h = h + ptr_len;
            let p_start = (w + base_w) / 2.0;
            let p_tip_x = w / 2.0;
            let p_tip_y = h + ptr_len;
            let p_end = (w - base_w) / 2.0;

            let path = format!(
                "M 0 0 L {w} 0 L {w} {h} L {p_start} {h} L {p_tip_x} {p_tip_y} L {p_end} {h} L 0 {h} Z"
            );
            ShapeOutput::new(path, total_w, total_h, format!("{} {}", w / 2.0, h / 2.0))
        }
        CalloutDirection::Up => {
            let base_w = (w * 0.2).clamp(10.0, 60.0).min(w);
            let total_w = w;
            let total_h = h + ptr_len;
            let p_start = (w - base_w) / 2.0;
            let p_tip_x = w / 2.0;
            let p_tip_y = 0.0;
            let p_end = (w + base_w) / 2.0;

            let path = format!(
                "M 0 {ptr_len} L {p_start} {ptr_len} L {p_tip_x} {p_tip_y} L {p_end} {ptr_len} L {w} {ptr_len} L {w} {total_h} L 0 {total_h} Z"
            );
            ShapeOutput::new(
                path,
                total_w,
                total_h,
                format!("{} {}", w / 2.0, ptr_len + h / 2.0),
            )
        }
        CalloutDirection::Right => {
            let base_h = (h * 0.2).clamp(10.0, 60.0).min(h);
            let total_w = w + ptr_len;
            let total_h = h;
            let p_start = (h - base_h) / 2.0;
            let p_tip_x = w + ptr_len;
            let p_tip_y = h / 2.0;
            let p_end = (h + base_h) / 2.0;

            let path = format!(
                "M 0 0 L {w} 0 L {w} {p_start} L {p_tip_x} {p_tip_y} L {w} {p_end} L {w} {h} L 0 {h} Z"
            );
            ShapeOutput::new(path, total_w, total_h, format!("{} {}", w / 2.0, h / 2.0))
        }
        CalloutDirection::Left => {
            let base_h = (h * 0.2).clamp(10.0, 60.0).min(h);
            let total_w = w + ptr_len;
            let total_h = h;
            let p_start = (h + base_h) / 2.0;
            let p_tip_x = 0.0;
            let p_tip_y = h / 2.0;
            let p_end = (h - base_h) / 2.0;

            let path = format!(
                "M {ptr_len} 0 L {total_w} 0 L {total_w} {h} L {ptr_len} {h} L {ptr_len} {p_start} L {p_tip_x} {p_tip_y} L {ptr_len} {p_end} Z"
            );
            ShapeOutput::new(
                path,
                total_w,
                total_h,
                format!("{} {}", ptr_len + w / 2.0, h / 2.0),
            )
        }
    }
}

/// Renders a procedural SVG Callout.
#[component]
pub fn Callout(props: CalloutProps) -> Element {
    let shape = make_callout(
        props.width,
        props.height,
        props.pointer_length,
        props.pointer_direction,
    );

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
    fn test_make_callout_down() {
        let callout = make_callout(200.0, 100.0, 30.0, CalloutDirection::Down);
        assert_eq!(callout.width, 200.0);
        assert_eq!(callout.height, 130.0);
        assert_eq!(callout.transform_origin, "100 50");
        assert!(callout.path.starts_with("M 0 0"));
        assert!(callout.path.contains("100 130"));
        assert!(callout.path.ends_with("Z"));
    }

    #[test]
    fn test_make_callout_up() {
        let callout = make_callout(200.0, 100.0, 30.0, CalloutDirection::Up);
        assert_eq!(callout.width, 200.0);
        assert_eq!(callout.height, 130.0);
        assert_eq!(callout.transform_origin, "100 80");
        assert!(callout.path.starts_with("M 0 30"));
        assert!(callout.path.contains("100 0"));
    }

    #[test]
    fn test_make_callout_right() {
        let callout = make_callout(200.0, 100.0, 30.0, CalloutDirection::Right);
        assert_eq!(callout.width, 230.0);
        assert_eq!(callout.height, 100.0);
        assert_eq!(callout.transform_origin, "100 50");
        assert!(callout.path.contains("230 50"));
    }

    #[test]
    fn test_make_callout_left() {
        let callout = make_callout(200.0, 100.0, 30.0, CalloutDirection::Left);
        assert_eq!(callout.width, 230.0);
        assert_eq!(callout.height, 100.0);
        assert_eq!(callout.transform_origin, "130 50");
        assert!(callout.path.starts_with("M 30 0"));
        assert!(callout.path.contains("0 50"));
    }

    #[test]
    fn test_callout_direction_default() {
        assert_eq!(CalloutDirection::default(), CalloutDirection::Down);
    }

    #[test]
    fn test_make_callout_zero_pointer_length() {
        let callout = make_callout(100.0, 80.0, 0.0, CalloutDirection::Down);
        assert_eq!(callout.width, 100.0);
        assert_eq!(callout.height, 80.0);
    }

    #[test]
    fn test_make_callout_zero_dimensions() {
        let callout = make_callout(0.0, 0.0, 20.0, CalloutDirection::Left);
        assert_eq!(callout.path, "");
        assert_eq!(callout.width, 20.0);
        assert_eq!(callout.height, 0.0);
    }
}
