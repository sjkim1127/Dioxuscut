//! Native Scene adapter for all procedural shape generators.

use crate::{make_arrow, make_circle, make_pie, make_polygon, make_rect, make_star, make_triangle};
use dioxuscut_composition::{CompositionError, SceneEmitter, SceneFrameContext};
use dioxuscut_rasterizer::{Color, Scene, SceneNode, Transform2D};
use serde_json::Value;

/// A procedural shape primitive shared with native preview and export.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneShape {
    pub path: String,
    pub width: f64,
    pub height: f64,
    pub x: f32,
    pub y: f32,
    pub fill: String,
    pub stroke: String,
    pub stroke_width: f32,
    pub opacity: f32,
}

impl SceneShape {
    pub fn new(path: impl Into<String>, width: f64, height: f64) -> Self {
        Self {
            path: path.into(),
            width,
            height,
            x: 0.0,
            y: 0.0,
            fill: "#ffffff".into(),
            stroke: "none".into(),
            stroke_width: 0.0,
            opacity: 1.0,
        }
    }

    pub fn arrow(length: f64, thickness: f64) -> Self {
        let (path, width, height) = make_arrow(length, thickness);
        Self::new(path, width, height)
    }

    pub fn circle(radius: f64) -> Self {
        let (path, width, height) = make_circle(radius);
        Self::new(path, width, height)
    }

    pub fn pie(radius: f64, progress: f64) -> Self {
        let (path, width, height) = make_pie(radius, progress);
        Self::new(path, width, height)
    }

    pub fn polygon(points: usize, radius: f64) -> Self {
        let (path, width, height) = make_polygon(points, radius);
        Self::new(path, width, height)
    }

    pub fn rect(width: f64, height: f64, corner_radius: f64) -> Self {
        let (path, width, height) = make_rect(width, height, corner_radius);
        Self::new(path, width, height)
    }

    pub fn star(points: usize, inner_radius: f64, outer_radius: f64) -> Self {
        let (path, width, height) = make_star(points, inner_radius, outer_radius);
        Self::new(path, width, height)
    }

    pub fn triangle(length: f64) -> Self {
        let (path, width, height) = make_triangle(length);
        Self::new(path, width, height)
    }

    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub fn with_fill(mut self, fill: impl Into<String>) -> Self {
        self.fill = fill.into();
        self
    }

    pub fn with_stroke(mut self, stroke: impl Into<String>, width: f32) -> Self {
        self.stroke = stroke.into();
        self.stroke_width = width.max(0.0);
        self
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }
}

impl SceneEmitter for SceneShape {
    fn emit(
        &self,
        context: SceneFrameContext,
        _props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        if self.path.trim().is_empty() {
            return Ok(());
        }
        let node = SceneNode::Path {
            d: self.path.clone(),
            fill: parse_optional_color(&self.fill, context)?,
            stroke: parse_optional_color(&self.stroke, context)?,
            stroke_width: self.stroke_width,
            opacity: self.opacity,
        };
        if self.x == 0.0 && self.y == 0.0 {
            scene.push(node);
        } else {
            scene.push(SceneNode::Group {
                transform: Transform2D {
                    tx: self.x,
                    ty: self.y,
                    ..Default::default()
                },
                opacity: 1.0,
                children: vec![node],
            });
        }
        Ok(())
    }
}

fn parse_optional_color(
    value: &str,
    context: SceneFrameContext,
) -> Result<Option<Color>, CompositionError> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("transparent") {
        return Ok(None);
    }
    Color::from_css(value).map(Some).ok_or_else(|| {
        CompositionError::render(
            context.global_frame,
            format!("unsupported native shape color '{value}'"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxuscut_composition::{
        NativeComposition, NativeCompositionContext, SceneEmitterComposition,
    };
    use dioxuscut_rasterizer::{FrameConfig, RasterizerBackend, TinySkiaBackend};

    fn context() -> NativeCompositionContext {
        NativeCompositionContext {
            width: 120,
            height: 120,
            fps: 30.0,
            duration_in_frames: 1,
        }
    }

    #[test]
    fn every_shape_generator_emits_a_native_path() {
        let shapes = [
            SceneShape::arrow(80.0, 10.0),
            SceneShape::circle(30.0),
            SceneShape::pie(30.0, 0.25),
            SceneShape::polygon(6, 30.0),
            SceneShape::rect(60.0, 40.0, 8.0),
            SceneShape::star(5, 12.0, 30.0),
            SceneShape::triangle(60.0),
        ];
        for shape in shapes {
            let composition = SceneEmitterComposition::new("shape", shape);
            let scene = composition.render(0, &Value::Null, context()).unwrap();
            assert!(matches!(scene.nodes[0], SceneNode::Path { .. }));
        }
    }

    #[test]
    fn rounded_shape_arc_renders_after_translation() {
        let shape = SceneShape::rect(60.0, 40.0, 10.0)
            .at(20.0, 30.0)
            .with_fill("rgb(255, 0, 0)");
        let composition = SceneEmitterComposition::new("shape", shape);
        let scene = composition.render(0, &Value::Null, context()).unwrap();
        let image = TinySkiaBackend::headless()
            .render_frame(&scene, &FrameConfig::new(120, 120, 0, 30.0))
            .unwrap();

        assert!(image.get_pixel(50, 50)[0] > 240);
        assert_eq!(image.get_pixel(20, 30)[3], 0);
    }

    #[test]
    fn invalid_css_colors_fail_with_the_global_frame() {
        let composition = SceneEmitterComposition::new(
            "shape",
            SceneShape::circle(20.0).with_fill("hsl(10 20% 30%)"),
        );
        let error = composition.render(0, &Value::Null, context()).unwrap_err();
        assert!(error.to_string().contains("unsupported native shape color"));
    }
}
