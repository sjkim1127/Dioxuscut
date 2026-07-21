//! Shared native composition contract and built-in composition registry.

use dioxuscut_rasterizer::{Color, GradientStop, Scene, SceneNode};
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
        });
        scene.push(SceneNode::Text {
            x: text_x,
            y: text_y + font_size * 0.8,
            content: subtitle,
            font_size: font_size * 0.45,
            color: Color::rgb(0, 242, 254),
            font_weight: 400,
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
