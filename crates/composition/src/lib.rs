//! Shared native composition contract and built-in composition registry.

pub mod safe_area;
mod scene_emitter;

pub use safe_area::{
    fit_text, get_safe_area_insets, measure_text_approx, Platform, SafeAreaInsets,
};
pub use scene_emitter::{
    FlipDirection, LinearWipeDirection, SceneEmitter, SceneEmitterComposition, SceneFrameContext,
    SceneFreeze, SceneGroup, SceneLayer, SceneLinearGradient, SceneLoop, SceneRect, SceneSequence,
    SceneSeries, SceneSeriesEntry, SceneStack, SceneText, SceneTextBlock, SceneTrail,
    SceneTrailOpacity, SceneTransitionSeries, TransitionKind, TransitionTiming,
};

use dioxuscut_rasterizer::{
    BlendMode, Color, GradientStop, MaskMode, Scene, SceneFilter, SceneNode, SceneShadow,
    Transform2D,
};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

/// Immutable render parameters supplied to every native composition frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeCompositionContext {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_in_frames: u32,
}

impl NativeCompositionContext {
    /// Normalized timeline progress in the inclusive range `0.0..=1.0`.
    pub fn progress(self, frame: u32) -> f32 {
        let last_frame = self.duration_in_frames.saturating_sub(1).max(1);
        (frame.min(last_frame) as f32 / last_frame as f32).clamp(0.0, 1.0)
    }
}

/// Errors produced while preparing or rendering a composition.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CompositionError {
    #[error("Failed to prepare composition: {0}")]
    Prepare(String),
    #[error("Failed to render frame {frame}: {reason}")]
    Render { frame: u32, reason: String },
}

impl CompositionError {
    pub fn render(frame: u32, reason: impl Into<String>) -> Self {
        Self::Render {
            frame,
            reason: reason.into(),
        }
    }
}

/// A composition instance prepared once for a complete render job.
///
/// Implementations may cache parsed input, compiled scripts, and other
/// immutable state here. `render` can be called concurrently for different
/// frames.
pub trait PreparedComposition: Send + Sync {
    fn render(&self, frame: u32) -> Result<Scene, CompositionError>;
}

/// General composition contract used by the registry.
pub trait Composition: Send + Sync {
    fn id(&self) -> &str;

    fn prepare(
        &self,
        props: &Value,
        context: NativeCompositionContext,
    ) -> Result<Box<dyn PreparedComposition + '_>, CompositionError>;
}

/// A browser-free Rust composition that produces one rasterizer scene per frame.
///
/// Applications can implement this trait, register implementations in a
/// [`CompositionRegistry`], and call `execute_render_command_with_registry`.
pub trait NativeComposition: Send + Sync {
    fn id(&self) -> &str;

    fn render(
        &self,
        frame: u32,
        props: &Value,
        context: NativeCompositionContext,
    ) -> Result<Scene, CompositionError>;
}

struct PreparedNativeComposition<'a, C> {
    composition: &'a C,
    props: Value,
    context: NativeCompositionContext,
}

impl<C> PreparedComposition for PreparedNativeComposition<'_, C>
where
    C: NativeComposition,
{
    fn render(&self, frame: u32) -> Result<Scene, CompositionError> {
        self.composition.render(frame, &self.props, self.context)
    }
}

impl<C> Composition for C
where
    C: NativeComposition,
{
    fn id(&self) -> &str {
        NativeComposition::id(self)
    }

    fn prepare(
        &self,
        props: &Value,
        context: NativeCompositionContext,
    ) -> Result<Box<dyn PreparedComposition + '_>, CompositionError> {
        Ok(Box::new(PreparedNativeComposition {
            composition: self,
            props: props.clone(),
            context,
        }))
    }
}

/// Errors produced while building or querying a composition registry.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CompositionRegistryError {
    #[error("Composition '{0}' is already registered")]
    Duplicate(String),
    #[error("Unknown composition '{requested}'. Available compositions: {available}")]
    Unknown {
        requested: String,
        available: String,
    },
}

/// Deterministic registry used by preview and export clients to resolve composition IDs.
#[derive(Default)]
pub struct CompositionRegistry {
    compositions: BTreeMap<String, Box<dyn Composition>>,
}

impl CompositionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<C>(&mut self, composition: C) -> Result<(), CompositionRegistryError>
    where
        C: Composition + 'static,
    {
        let id = composition.id().to_string();
        if self.compositions.contains_key(&id) {
            return Err(CompositionRegistryError::Duplicate(id));
        }
        self.compositions.insert(id, Box::new(composition));
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<&dyn Composition, CompositionRegistryError> {
        self.compositions.get(id).map(Box::as_ref).ok_or_else(|| {
            CompositionRegistryError::Unknown {
                requested: id.to_string(),
                available: self.ids().join(", "),
            }
        })
    }

    pub fn ids(&self) -> Vec<&str> {
        self.compositions.keys().map(String::as_str).collect()
    }
}

/// Registry shipped by the standalone `dioxuscut` binary.
pub fn built_in_registry() -> CompositionRegistry {
    let mut registry = CompositionRegistry::new();
    registry
        .register(HelloWorldComposition)
        .expect("built-in composition IDs must be unique");
    registry
        .register(ShapesAndFiltersComposition)
        .expect("built-in composition IDs must be unique");
    registry
        .register(CyberpunkGridComposition)
        .expect("built-in composition IDs must be unique");
    registry
        .register(ComplexGradientsComposition)
        .expect("built-in composition IDs must be unique");
    registry
}

/// Built-in native composition used by the quickstart and acceptance tests.
pub struct HelloWorldComposition;

impl NativeComposition for HelloWorldComposition {
    fn id(&self) -> &str {
        "HelloWorld"
    }

    fn render(
        &self,
        frame: u32,
        props: &Value,
        context: NativeCompositionContext,
    ) -> Result<Scene, CompositionError> {
        let width = context.width as f32;
        let height = context.height as f32;
        let t = context.progress(frame);

        let bg_start = color_prop(props, "background_start", Color::rgb(15, 23, 42));
        let bg_end = color_prop(props, "background_end", Color::rgb(30, 27, 75));
        let accent = color_prop(props, "accent_color", Color::rgb(108, 99, 255));
        let title = string_prop(props, "title", "Hello Dioxuscut");
        let subtitle = string_prop(props, "subtitle", "Declarative programmatic video in Rust");

        let mut scene = Scene::new();
        scene.push(SceneNode::LinearGradient {
            x: 0.0,
            y: 0.0,
            w: width,
            h: height,
            angle_deg: 135.0 + t * 90.0,
            stops: vec![
                GradientStop {
                    position: 0.0,
                    color: bg_start,
                },
                GradientStop {
                    position: 1.0,
                    color: bg_end,
                },
            ],
        });

        let center_x = width * 0.5;
        let center_y = height * 0.5;
        let shortest_side = width.min(height);
        let r1 = shortest_side * 0.2 + (t * std::f32::consts::TAU).sin() * 20.0;
        scene.push(SceneNode::Circle {
            cx: center_x,
            cy: center_y,
            r: r1,
            fill: accent.with_opacity(0.12),
            stroke: Some(accent),
            stroke_width: 2.0,
        });

        let r2 = shortest_side * 0.3 + (t * std::f32::consts::PI).cos() * 30.0;
        scene.push(SceneNode::Circle {
            cx: center_x,
            cy: center_y,
            r: r2,
            fill: Color::TRANSPARENT,
            stroke: Some(Color::rgba(0, 242, 254, 180)),
            stroke_width: 1.5,
        });

        let rect_size = 80.0 + (t * std::f32::consts::TAU).sin() * 15.0;
        scene.push(SceneNode::Rect {
            x: width * 0.15,
            y: height * 0.2,
            w: rect_size,
            h: rect_size,
            fill: Color::rgba(0, 242, 254, 40),
            stroke: Some(Color::rgb(0, 242, 254)),
            stroke_width: 2.0,
            corner_radius: 12.0,
        });
        scene.push(SceneNode::Rect {
            x: width * 0.78,
            y: height * 0.65,
            w: rect_size * 1.2,
            h: rect_size * 1.2,
            fill: Color::rgba(255, 230, 0, 30),
            stroke: Some(Color::rgb(255, 230, 0)),
            stroke_width: 2.0,
            corner_radius: 16.0,
        });

        scene.push(SceneNode::Rect {
            x: 0.0,
            y: height - 6.0,
            w: width * t,
            h: 6.0,
            fill: Color::rgb(0, 242, 254),
            stroke: None,
            stroke_width: 0.0,
            corner_radius: 0.0,
        });

        let font_size = (width * 0.045).max(28.0);
        let text_x = width * 0.12;
        let text_y = height * 0.45;
        scene.push(SceneNode::Text {
            x: text_x,
            y: text_y,
            content: title,
            font_size,
            color: Color::WHITE,
            font_weight: 700,
            font_sources: Vec::new(),
        });
        scene.push(SceneNode::Text {
            x: text_x,
            y: text_y + font_size * 0.8,
            content: subtitle,
            font_size: font_size * 0.45,
            color: Color::rgb(0, 242, 254),
            font_weight: 400,
            font_sources: Vec::new(),
        });
        Ok(scene)
    }
}

/// Built-in composition exercising geometric shapes, SVG paths, shadows, and image filters.
pub struct ShapesAndFiltersComposition;

impl NativeComposition for ShapesAndFiltersComposition {
    fn id(&self) -> &str {
        "ShapesAndFilters"
    }

    fn render(
        &self,
        frame: u32,
        _props: &Value,
        context: NativeCompositionContext,
    ) -> Result<Scene, CompositionError> {
        let width = context.width as f32;
        let height = context.height as f32;
        let t = context.progress(frame);

        let mut scene = Scene::new();

        // 1. Dark canvas background
        scene.push(SceneNode::Rect {
            x: 0.0,
            y: 0.0,
            w: width,
            h: height,
            fill: Color::rgb(11, 15, 25),
            stroke: None,
            stroke_width: 0.0,
            corner_radius: 0.0,
        });

        // 2. Layer with drop shadow and blur filter
        let center_x = width * 0.35;
        let center_y = height * 0.5;

        scene.push(SceneNode::Layer {
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            clip: None,
            mask: None,
            mask_mode: MaskMode::Alpha,
            filters: vec![SceneFilter::Blur {
                sigma: 2.0 + t * 4.0,
            }],
            shadow: Some(SceneShadow {
                color: Color::rgba(255, 0, 128, 160),
                blur_sigma: 16.0,
                offset_x: 0.0,
                offset_y: 4.0,
            }),
            children: vec![SceneNode::Rect {
                x: center_x - 100.0,
                y: center_y - 100.0,
                w: 200.0,
                h: 200.0,
                fill: Color::rgba(255, 0, 128, 200),
                stroke: Some(Color::rgb(255, 255, 255)),
                stroke_width: 3.0,
                corner_radius: 24.0 + (t * std::f32::consts::PI).sin() * 20.0,
            }],
        });

        // 3. SVG Star path with animated transform
        let star_path =
            "M 0 -50 L 14 -15 L 50 -15 L 21 7 L 32 43 L 0 22 L -32 43 L -21 7 L -50 -15 L -14 -15 Z"
                .to_string();
        scene.push(SceneNode::Group {
            transform: Transform2D::translate(width * 0.65, height * 0.5)
                .with_rotate(t * 180.0)
                .with_scale_uniform(1.2 + (t * std::f32::consts::PI).sin() * 0.3),
            opacity: 0.9,
            children: vec![SceneNode::Path {
                d: star_path,
                fill: Some(Color::rgb(0, 240, 255)),
                stroke: Some(Color::rgb(255, 255, 255)),
                stroke_width: 2.0,
                opacity: 1.0,
            }],
        });

        // 4. Concentric circles
        scene.push(SceneNode::Circle {
            cx: width * 0.5,
            cy: height * 0.85,
            r: 40.0 + t * 20.0,
            fill: Color::rgba(0, 255, 128, 60),
            stroke: Some(Color::rgb(0, 255, 128)),
            stroke_width: 2.5,
        });

        // 5. Fullscreen vignette filter overlay
        scene.push(SceneNode::Layer {
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            clip: None,
            mask: None,
            mask_mode: MaskMode::Alpha,
            filters: vec![SceneFilter::Vignette {
                offset: 0.85,
                darkness: 0.75,
                roundness: 0.5,
            }],
            shadow: None,
            children: vec![],
        });

        Ok(scene)
    }
}

/// Built-in composition exercising procedural perspective grid lines and chromatic aberration.
pub struct CyberpunkGridComposition;

impl NativeComposition for CyberpunkGridComposition {
    fn id(&self) -> &str {
        "CyberpunkGrid"
    }

    fn render(
        &self,
        frame: u32,
        _props: &Value,
        context: NativeCompositionContext,
    ) -> Result<Scene, CompositionError> {
        let width = context.width as f32;
        let height = context.height as f32;
        let t = context.progress(frame);

        let mut scene = Scene::new();

        // 1. Dark grid background
        scene.push(SceneNode::Rect {
            x: 0.0,
            y: 0.0,
            w: width,
            h: height,
            fill: Color::rgb(5, 8, 20),
            stroke: None,
            stroke_width: 0.0,
            corner_radius: 0.0,
        });

        // 2. Perspective grid lines
        let mut grid_nodes = Vec::new();
        let grid_y_start = height * 0.5;
        let n_lines = 12;
        for i in 0..=n_lines {
            let frac = i as f32 / n_lines as f32;
            let y = grid_y_start + (frac * frac) * (height - grid_y_start);
            grid_nodes.push(SceneNode::Rect {
                x: 0.0,
                y,
                w: width,
                h: 1.5,
                fill: Color::rgba(0, 240, 255, (frac * 180.0) as u8),
                stroke: None,
                stroke_width: 0.0,
                corner_radius: 0.0,
            });
        }
        for i in -6..=6 {
            let x_top = width * 0.5 + (i as f32) * (width * 0.03);
            let x_bottom = width * 0.5 + (i as f32) * (width * 0.15) + (t * 20.0);
            let d = format!("M {x_top} {grid_y_start} L {x_bottom} {height}");
            grid_nodes.push(SceneNode::Path {
                d,
                fill: None,
                stroke: Some(Color::rgba(255, 0, 128, 150)),
                stroke_width: 1.5,
                opacity: 0.8,
            });
        }

        // 3. Chromatic aberration layer over grid
        scene.push(SceneNode::Layer {
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            clip: None,
            mask: None,
            mask_mode: MaskMode::Alpha,
            filters: vec![SceneFilter::ChromaticAberration {
                offset_x: 4.0 * (1.0 - t),
                offset_y: 2.0 * (1.0 - t),
                angle_rad: 0.0,
            }],
            shadow: None,
            children: grid_nodes,
        });

        // 4. Glowing center neon sign
        scene.push(SceneNode::Layer {
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            clip: None,
            mask: None,
            mask_mode: MaskMode::Alpha,
            filters: vec![],
            shadow: Some(SceneShadow {
                color: Color::rgb(0, 240, 255),
                blur_sigma: 24.0,
                offset_x: 0.0,
                offset_y: 0.0,
            }),
            children: vec![
                SceneNode::Rect {
                    x: width * 0.5 - 180.0,
                    y: height * 0.3 - 40.0,
                    w: 360.0,
                    h: 80.0,
                    fill: Color::rgba(10, 15, 35, 220),
                    stroke: Some(Color::rgb(0, 240, 255)),
                    stroke_width: 2.5,
                    corner_radius: 8.0,
                },
                SceneNode::Text {
                    x: width * 0.5 - 120.0,
                    y: height * 0.3 + 10.0,
                    content: "CYBERPUNK".to_string(),
                    font_size: 36.0,
                    color: Color::rgb(255, 255, 255),
                    font_weight: 700,
                    font_sources: vec![],
                },
            ],
        });

        Ok(scene)
    }
}

/// Built-in composition exercising multi-stop linear and radial gradients with opacity blending.
pub struct ComplexGradientsComposition;

impl NativeComposition for ComplexGradientsComposition {
    fn id(&self) -> &str {
        "ComplexGradients"
    }

    fn render(
        &self,
        frame: u32,
        _props: &Value,
        context: NativeCompositionContext,
    ) -> Result<Scene, CompositionError> {
        let width = context.width as f32;
        let height = context.height as f32;
        let t = context.progress(frame);

        let mut scene = Scene::new();

        // 1. Multi-stop linear gradient background
        scene.push(SceneNode::LinearGradient {
            x: 0.0,
            y: 0.0,
            w: width,
            h: height,
            angle_deg: 45.0 + t * 180.0,
            stops: vec![
                GradientStop {
                    position: 0.0,
                    color: Color::rgb(76, 29, 149),
                }, // Purple
                GradientStop {
                    position: 0.35,
                    color: Color::rgb(13, 148, 136),
                }, // Teal
                GradientStop {
                    position: 0.70,
                    color: Color::rgb(234, 88, 12),
                }, // Orange
                GradientStop {
                    position: 1.0,
                    color: Color::rgb(225, 29, 72),
                }, // Rose
            ],
        });

        // 2. Overlapping Radial Gradient 1
        scene.push(SceneNode::RadialGradient {
            cx: width * 0.3 + (t * std::f32::consts::TAU).cos() * 50.0,
            cy: height * 0.4 + (t * std::f32::consts::TAU).sin() * 40.0,
            r: width * 0.35,
            stops: vec![
                GradientStop {
                    position: 0.0,
                    color: Color::rgba(255, 255, 255, 180),
                },
                GradientStop {
                    position: 0.5,
                    color: Color::rgba(0, 240, 255, 100),
                },
                GradientStop {
                    position: 1.0,
                    color: Color::rgba(0, 0, 0, 0),
                },
            ],
        });

        // 3. Overlapping Radial Gradient 2
        scene.push(SceneNode::RadialGradient {
            cx: width * 0.7 - (t * std::f32::consts::TAU).cos() * 40.0,
            cy: height * 0.6 - (t * std::f32::consts::TAU).sin() * 50.0,
            r: width * 0.4,
            stops: vec![
                GradientStop {
                    position: 0.0,
                    color: Color::rgba(255, 230, 0, 160),
                },
                GradientStop {
                    position: 0.6,
                    color: Color::rgba(255, 0, 128, 80),
                },
                GradientStop {
                    position: 1.0,
                    color: Color::rgba(0, 0, 0, 0),
                },
            ],
        });

        // 4. Semi-transparent geometric accents
        scene.push(SceneNode::Rect {
            x: width * 0.1,
            y: height * 0.1,
            w: width * 0.8,
            h: height * 0.8,
            fill: Color::rgba(255, 255, 255, 15),
            stroke: Some(Color::rgba(255, 255, 255, 120)),
            stroke_width: 1.5,
            corner_radius: 20.0,
        });

        Ok(scene)
    }
}

fn color_prop(props: &Value, key: &str, fallback: Color) -> Color {
    props
        .get(key)
        .and_then(Value::as_str)
        .and_then(Color::from_hex)
        .unwrap_or(fallback)
}

fn string_prop(props: &Value, key: &str, fallback: &str) -> String {
    props
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_rejects_duplicate_ids_and_reports_known_ids() {
        let mut registry = CompositionRegistry::new();
        registry.register(HelloWorldComposition).unwrap();
        assert_eq!(
            registry.register(HelloWorldComposition),
            Err(CompositionRegistryError::Duplicate("HelloWorld".into()))
        );

        let error = match registry.get("Missing") {
            Ok(_) => panic!("missing composition unexpectedly resolved"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            CompositionRegistryError::Unknown {
                requested: "Missing".into(),
                available: "HelloWorld".into(),
            }
        );
    }

    #[test]
    fn hello_world_uses_props_and_reaches_full_progress() {
        let context = NativeCompositionContext {
            width: 100,
            height: 100,
            fps: 30.0,
            duration_in_frames: 3,
        };
        let props = serde_json::json!({"title": "Custom title"});
        let scene = HelloWorldComposition.render(2, &props, context).unwrap();

        assert!(scene.nodes.iter().any(|node| matches!(
            node,
            SceneNode::Text { content, .. } if content == "Custom title"
        )));
        assert!(scene.nodes.iter().any(|node| matches!(
            node,
            SceneNode::Rect { w, h, .. } if *w == 100.0 && *h == 6.0
        )));
    }
}
