//! Dioxus preview adapter for the same native scenes used during export.

use dioxus::prelude::*;
use dioxuscut_composition::{Composition, CompositionError, NativeCompositionContext};
use dioxuscut_core::use_current_frame;
use dioxuscut_rasterizer::{Color, GradientStop, ImageFit, Scene, SceneNode, Transform2D};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static NEXT_SCENE_VIEW_ID: AtomicU64 = AtomicU64::new(1);

/// Cloneable handle for passing a native composition through Dioxus props.
#[derive(Clone)]
pub struct CompositionHandle(Arc<dyn Composition>);

impl CompositionHandle {
    pub fn new<C>(composition: C) -> Self
    where
        C: Composition + 'static,
    {
        Self(Arc::new(composition))
    }

    pub fn id(&self) -> &str {
        self.0.id()
    }

    /// Prepare and render one frame through the shared export contract.
    pub fn render_frame(
        &self,
        frame: u32,
        input_props: &Value,
        context: NativeCompositionContext,
    ) -> Result<Scene, CompositionError> {
        self.0.prepare(input_props, context)?.render(frame)
    }
}

impl PartialEq for CompositionHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct NativeCompositionPreviewProps {
    pub composition: CompositionHandle,
    #[props(default = Value::Object(Default::default()))]
    pub input_props: Value,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_in_frames: u32,
}

/// Renders the current Player frame using the same `Composition -> Scene` path as export.
#[component]
pub fn NativeCompositionPreview(props: NativeCompositionPreviewProps) -> Element {
    let frame = use_current_frame();
    let context = NativeCompositionContext {
        width: props.width,
        height: props.height,
        fps: props.fps,
        duration_in_frames: props.duration_in_frames,
    };

    match props
        .composition
        .render_frame(frame, &props.input_props, context)
    {
        Ok(scene) => rsx! {
            SceneView {
                scene,
                width: props.width,
                height: props.height,
            }
        },
        Err(error) => rsx! {
            div {
                class: "dioxuscut-native-preview-error",
                style: "box-sizing: border-box; width: 100%; height: 100%; padding: 24px; background: #220d16; color: #fecdd3; font-family: monospace; white-space: pre-wrap;",
                "Failed to preview composition '{props.composition.id()}' at frame {frame}: {error}"
            }
        },
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SceneViewProps {
    pub scene: Scene,
    pub width: u32,
    pub height: u32,
}

/// Converts the shared native scene graph to SVG for Dioxus web/desktop preview.
#[component]
pub fn SceneView(props: SceneViewProps) -> Element {
    let instance_id = use_hook(|| NEXT_SCENE_VIEW_ID.fetch_add(1, Ordering::Relaxed));
    let view_box = format!("0 0 {} {}", props.width, props.height);

    rsx! {
        svg {
            class: "dioxuscut-scene-view",
            view_box,
            width: "100%",
            height: "100%",
            style: "display: block; overflow: hidden;",
            xmlns: "http://www.w3.org/2000/svg",
            for (index, node) in props.scene.nodes.into_iter().enumerate() {
                SceneNodeView {
                    key: "root-{index}",
                    node,
                    node_path: format!("scene-{instance_id}-root-{index}"),
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct SceneNodeViewProps {
    node: SceneNode,
    node_path: String,
}

#[component]
fn SceneNodeView(props: SceneNodeViewProps) -> Element {
    match props.node {
        SceneNode::Rect {
            x,
            y,
            w,
            h,
            fill,
            stroke,
            stroke_width,
            corner_radius,
        } => {
            let fill = color_css(fill);
            let stroke = optional_color_css(stroke);
            rsx! {
                rect {
                    x,
                    y,
                    width: w,
                    height: h,
                    rx: corner_radius,
                    ry: corner_radius,
                    fill,
                    stroke,
                    stroke_width,
                }
            }
        }
        SceneNode::Circle {
            cx,
            cy,
            r,
            fill,
            stroke,
            stroke_width,
        } => {
            let fill = color_css(fill);
            let stroke = optional_color_css(stroke);
            rsx! {
                circle { cx, cy, r, fill, stroke, stroke_width }
            }
        }
        SceneNode::Path {
            d,
            fill,
            stroke,
            stroke_width,
            opacity,
        } => {
            let fill = optional_color_css(fill);
            let stroke = optional_color_css(stroke);
            rsx! {
                path { d, fill, stroke, stroke_width, opacity }
            }
        }
        SceneNode::Text {
            x,
            y,
            content,
            font_size,
            color,
            font_weight,
        } => {
            let fill = color_css(color);
            rsx! {
                text {
                    x,
                    y,
                    fill,
                    font_size,
                    font_weight,
                    dominant_baseline: "alphabetic",
                    "{content}"
                }
            }
        }
        SceneNode::Image {
            src,
            x,
            y,
            w,
            h,
            fit,
            opacity,
        } => {
            let preserve_aspect_ratio = image_preserve_aspect_ratio(fit);
            rsx! {
                image {
                    x,
                    y,
                    width: w,
                    height: h,
                    href: src,
                    opacity,
                    preserve_aspect_ratio,
                }
            }
        }
        SceneNode::Video {
            src,
            time,
            x,
            y,
            w,
            h,
            fit,
            opacity,
        } => {
            let src = format!("{src}#t={time:.6}");
            let style = format!(
                "display:block;width:100%;height:100%;object-fit:{};opacity:{};",
                media_object_fit(fit),
                opacity.clamp(0.0, 1.0)
            );
            rsx! {
                foreignObject { x, y, width: w, height: h,
                    video {
                        src,
                        style,
                        muted: true,
                        preload: "auto",
                    }
                }
            }
        }
        SceneNode::Audio { .. } => rsx! {},
        SceneNode::LinearGradient {
            x,
            y,
            w,
            h,
            angle_deg,
            stops,
        } => {
            let gradient_id = format!("dioxuscut-linear-{}", props.node_path);
            let center_x = x + w / 2.0;
            let center_y = y + h / 2.0;
            let gradient_transform = format!("rotate({angle_deg} {center_x} {center_y})");
            let fill = format!("url(#{gradient_id})");
            rsx! {
                defs {
                    linearGradient {
                        id: gradient_id,
                        gradient_units: "userSpaceOnUse",
                        x1: x,
                        y1: center_y,
                        x2: x + w,
                        y2: center_y,
                        gradient_transform,
                        for (index, stop) in stops.into_iter().enumerate() {
                            GradientStopView { key: "stop-{index}", stop }
                        }
                    }
                }
                rect { x, y, width: w, height: h, fill }
            }
        }
        SceneNode::RadialGradient { cx, cy, r, stops } => {
            let gradient_id = format!("dioxuscut-radial-{}", props.node_path);
            let fill = format!("url(#{gradient_id})");
            rsx! {
                defs {
                    radialGradient {
                        id: gradient_id,
                        gradient_units: "userSpaceOnUse",
                        cx,
                        cy,
                        r,
                        for (index, stop) in stops.into_iter().enumerate() {
                            GradientStopView { key: "stop-{index}", stop }
                        }
                    }
                }
                circle { cx, cy, r, fill }
            }
        }
        SceneNode::Group {
            transform,
            opacity,
            children,
        } => {
            let transform = transform_css(transform);
            rsx! {
                g {
                    transform,
                    opacity,
                    for (index, node) in children.into_iter().enumerate() {
                        SceneNodeView {
                            key: "child-{index}",
                            node,
                            node_path: format!("{}-{index}", props.node_path),
                        }
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct GradientStopViewProps {
    stop: GradientStop,
}

#[component]
fn GradientStopView(props: GradientStopViewProps) -> Element {
    let offset = format!("{}%", props.stop.position.clamp(0.0, 1.0) * 100.0);
    let stop_color = color_css(props.stop.color);
    rsx! { stop { offset, stop_color } }
}

fn color_css(color: Color) -> String {
    format!(
        "rgba({}, {}, {}, {:.6})",
        color.r,
        color.g,
        color.b,
        color.a as f64 / 255.0
    )
}

fn optional_color_css(color: Option<Color>) -> String {
    color.map(color_css).unwrap_or_else(|| "none".to_string())
}

fn transform_css(transform: Transform2D) -> String {
    format!(
        "translate({} {}) scale({} {}) rotate({})",
        transform.tx, transform.ty, transform.scale_x, transform.scale_y, transform.rotate_deg
    )
}

fn image_preserve_aspect_ratio(fit: ImageFit) -> &'static str {
    match fit {
        ImageFit::Cover => "xMidYMid slice",
        ImageFit::Contain | ImageFit::ScaleDown => "xMidYMid meet",
        ImageFit::Fill => "none",
        ImageFit::None => "xMidYMid meet",
    }
}

fn media_object_fit(fit: ImageFit) -> &'static str {
    match fit {
        ImageFit::Cover => "cover",
        ImageFit::Contain => "contain",
        ImageFit::Fill => "fill",
        ImageFit::None => "none",
        ImageFit::ScaleDown => "scale-down",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxuscut_composition::HelloWorldComposition;

    #[test]
    fn composition_handle_renders_the_shared_scene() {
        let composition = CompositionHandle::new(HelloWorldComposition);
        let context = NativeCompositionContext {
            width: 320,
            height: 180,
            fps: 30.0,
            duration_in_frames: 30,
        };
        let scene = composition
            .render_frame(10, &serde_json::json!({"title": "Shared preview"}), context)
            .unwrap();

        assert!(scene.nodes.iter().any(|node| matches!(
            node,
            SceneNode::Text { content, .. } if content == "Shared preview"
        )));
    }

    #[test]
    fn composition_handles_compare_by_identity() {
        let first = CompositionHandle::new(HelloWorldComposition);
        let clone = first.clone();
        let second = CompositionHandle::new(HelloWorldComposition);

        assert!(first == clone);
        assert!(first != second);
    }

    #[test]
    fn image_fit_maps_to_svg_aspect_ratio() {
        assert_eq!(
            image_preserve_aspect_ratio(ImageFit::Cover),
            "xMidYMid slice"
        );
        assert_eq!(
            image_preserve_aspect_ratio(ImageFit::Contain),
            "xMidYMid meet"
        );
        assert_eq!(image_preserve_aspect_ratio(ImageFit::Fill), "none");
        assert_eq!(media_object_fit(ImageFit::ScaleDown), "scale-down");
    }
}
