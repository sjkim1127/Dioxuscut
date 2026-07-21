//! Sandboxed Rhai composition runtime.
//!
//! JSON remains the external props format. A Rhai script receives those props
//! plus an immutable frame context and returns a restricted [`SceneBuilder`].

use crate::composition::{
    Composition, CompositionError, NativeCompositionContext, PreparedComposition,
};
use dioxuscut_rasterizer::{AudioTrack, Color, ImageFit, Scene, SceneNode, Transform2D};
use rhai::module_resolvers::DummyModuleResolver;
use rhai::{
    Dynamic, Engine, EvalAltResult, ImmutableString, Map, Position, Scope, AST, FLOAT, INT,
};
use serde_json::Value;
use std::fs;
use std::path::Path;

const MAX_OPERATIONS_PER_FRAME: u64 = 100_000;
const MAX_STRING_SIZE: usize = 1_048_576;
const MAX_ARRAY_SIZE: usize = 4_096;
const MAX_MAP_SIZE: usize = 1_024;

type RhaiResult<T> = Result<T, Box<EvalAltResult>>;

/// A restricted, script-facing builder for the native scene graph.
#[derive(Debug, Clone, Default)]
pub struct SceneBuilder {
    scene: Scene,
}

impl SceneBuilder {
    fn new() -> Self {
        Self::default()
    }

    fn into_scene(self) -> Scene {
        self.scene
    }

    fn rect(&mut self, x: FLOAT, y: FLOAT, w: FLOAT, h: FLOAT, fill: &str) -> RhaiResult<()> {
        self.scene.push(SceneNode::Rect {
            x: finite_f32("x", x)?,
            y: finite_f32("y", y)?,
            w: non_negative_f32("width", w)?,
            h: non_negative_f32("height", h)?,
            fill: parse_color(fill)?,
            stroke: None,
            stroke_width: 0.0,
            corner_radius: 0.0,
        });
        Ok(())
    }

    fn round_rect(
        &mut self,
        x: FLOAT,
        y: FLOAT,
        w: FLOAT,
        h: FLOAT,
        fill: &str,
        radius: FLOAT,
    ) -> RhaiResult<()> {
        self.scene.push(SceneNode::Rect {
            x: finite_f32("x", x)?,
            y: finite_f32("y", y)?,
            w: non_negative_f32("width", w)?,
            h: non_negative_f32("height", h)?,
            fill: parse_color(fill)?,
            stroke: None,
            stroke_width: 0.0,
            corner_radius: non_negative_f32("corner radius", radius)?,
        });
        Ok(())
    }

    fn circle(&mut self, cx: FLOAT, cy: FLOAT, radius: FLOAT, fill: &str) -> RhaiResult<()> {
        self.scene.push(SceneNode::Circle {
            cx: finite_f32("center x", cx)?,
            cy: finite_f32("center y", cy)?,
            r: non_negative_f32("radius", radius)?,
            fill: parse_color(fill)?,
            stroke: None,
            stroke_width: 0.0,
        });
        Ok(())
    }

    fn text(
        &mut self,
        x: FLOAT,
        y: FLOAT,
        content: ImmutableString,
        font_size: FLOAT,
        color: &str,
    ) -> RhaiResult<()> {
        self.push_text(x, y, content, font_size, color, 400)
    }

    fn text_bold(
        &mut self,
        x: FLOAT,
        y: FLOAT,
        content: ImmutableString,
        font_size: FLOAT,
        color: &str,
    ) -> RhaiResult<()> {
        self.push_text(x, y, content, font_size, color, 700)
    }

    #[allow(clippy::too_many_arguments)]
    fn image(
        &mut self,
        x: FLOAT,
        y: FLOAT,
        w: FLOAT,
        h: FLOAT,
        src: ImmutableString,
        fit: &str,
        opacity: FLOAT,
    ) -> RhaiResult<()> {
        if src.trim().is_empty() {
            return Err(runtime_error("image source path must not be empty".into()));
        }
        let opacity = finite_f32("opacity", opacity)?;
        if !(0.0..=1.0).contains(&opacity) {
            return Err(runtime_error(
                "opacity must be between 0.0 and 1.0".to_string(),
            ));
        }

        self.scene.push(SceneNode::Image {
            src: src.into_owned(),
            x: finite_f32("x", x)?,
            y: finite_f32("y", y)?,
            w: non_negative_f32("width", w)?,
            h: non_negative_f32("height", h)?,
            fit: parse_image_fit(fit)?,
            opacity,
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn video(
        &mut self,
        x: FLOAT,
        y: FLOAT,
        w: FLOAT,
        h: FLOAT,
        src: ImmutableString,
        time: FLOAT,
        fit: &str,
        opacity: FLOAT,
    ) -> RhaiResult<()> {
        validate_media_source(&src)?;
        let time = non_negative_f64("video time", time)?;
        let opacity = unit_f32("opacity", opacity)?;
        self.scene.push(SceneNode::Video {
            src: src.into_owned(),
            time,
            x: finite_f32("x", x)?,
            y: finite_f32("y", y)?,
            w: non_negative_f32("width", w)?,
            h: non_negative_f32("height", h)?,
            fit: parse_image_fit(fit)?,
            opacity,
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn audio(
        &mut self,
        src: ImmutableString,
        start_from: FLOAT,
        timeline_start: FLOAT,
        duration: FLOAT,
        volume: FLOAT,
        playback_rate: FLOAT,
        looped: bool,
    ) -> RhaiResult<()> {
        validate_media_source(&src)?;
        let duration = non_negative_f64("audio duration", duration)?;
        let playback_rate = finite_f64("audio playback rate", playback_rate)?;
        if !(0.5..=2.0).contains(&playback_rate) {
            return Err(runtime_error(
                "audio playback rate must be between 0.5 and 2.0".into(),
            ));
        }
        self.scene.push(SceneNode::Audio {
            track: AudioTrack {
                src: src.into_owned(),
                start_from: non_negative_f64("audio source offset", start_from)?,
                timeline_start: non_negative_f64("audio timeline offset", timeline_start)?,
                duration: (duration > 0.0).then_some(duration),
                volume: f64::from(unit_f32("audio volume", volume)?),
                playback_rate,
                looped,
            },
        });
        Ok(())
    }

    fn group(
        &mut self,
        children: SceneBuilder,
        tx: FLOAT,
        ty: FLOAT,
        scale: FLOAT,
        rotate_deg: FLOAT,
        opacity: FLOAT,
    ) -> RhaiResult<()> {
        let opacity = finite_f32("opacity", opacity)?;
        if !(0.0..=1.0).contains(&opacity) {
            return Err(runtime_error(
                "opacity must be between 0.0 and 1.0".to_string(),
            ));
        }
        let scale = non_negative_f32("scale", scale)?;
        self.scene.push(SceneNode::Group {
            transform: Transform2D {
                tx: finite_f32("translation x", tx)?,
                ty: finite_f32("translation y", ty)?,
                scale_x: scale,
                scale_y: scale,
                rotate_deg: finite_f32("rotation", rotate_deg)?,
            },
            opacity,
            children: children.into_scene().nodes,
        });
        Ok(())
    }

    fn push_text(
        &mut self,
        x: FLOAT,
        y: FLOAT,
        content: ImmutableString,
        font_size: FLOAT,
        color: &str,
        font_weight: u16,
    ) -> RhaiResult<()> {
        self.scene.push(SceneNode::Text {
            x: finite_f32("x", x)?,
            y: finite_f32("y", y)?,
            content: content.into_owned(),
            font_size: non_negative_f32("font size", font_size)?,
            color: parse_color(color)?,
            font_weight,
        });
        Ok(())
    }
}

/// A compiled Rhai composition. Constructing this type compiles the script once.
pub struct RhaiComposition {
    id: String,
    engine: Engine,
    ast: AST,
}

impl RhaiComposition {
    pub fn from_file(path: &Path) -> Result<Self, CompositionError> {
        let source = fs::read_to_string(path).map_err(|error| {
            CompositionError::Prepare(format!(
                "failed to read Rhai script {}: {error}",
                path.display()
            ))
        })?;
        let id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("RhaiComposition");
        Self::from_source(id, &source)
    }

    pub fn from_source(id: impl Into<String>, source: &str) -> Result<Self, CompositionError> {
        let mut engine = hardened_engine();
        register_scene_api(&mut engine);
        let ast = engine
            .compile(source)
            .map_err(|error| CompositionError::Prepare(format!("Rhai compile error: {error}")))?;

        Ok(Self {
            id: id.into(),
            engine,
            ast,
        })
    }
}

impl Composition for RhaiComposition {
    fn id(&self) -> &str {
        &self.id
    }

    fn prepare(
        &self,
        props: &Value,
        context: NativeCompositionContext,
    ) -> Result<Box<dyn PreparedComposition + '_>, CompositionError> {
        let props = rhai::serde::to_dynamic(props).map_err(|error| {
            CompositionError::Prepare(format!("failed to convert JSON props to Rhai: {error}"))
        })?;

        Ok(Box::new(PreparedRhaiComposition {
            engine: &self.engine,
            ast: &self.ast,
            props,
            context,
        }))
    }
}

struct PreparedRhaiComposition<'a> {
    engine: &'a Engine,
    ast: &'a AST,
    props: Dynamic,
    context: NativeCompositionContext,
}

impl PreparedComposition for PreparedRhaiComposition<'_> {
    fn render(&self, frame: u32) -> Result<Scene, CompositionError> {
        let mut scope = Scope::new();
        let context = context_map(frame, self.context);
        let builder = self
            .engine
            .call_fn::<SceneBuilder>(
                &mut scope,
                self.ast,
                "render",
                (context, self.props.clone()),
            )
            .map_err(|error| CompositionError::render(frame, format!("Rhai error: {error}")))?;
        Ok(builder.into_scene())
    }
}

fn hardened_engine() -> Engine {
    let mut engine = Engine::new();
    engine.set_module_resolver(DummyModuleResolver::new());
    engine.set_max_operations(MAX_OPERATIONS_PER_FRAME);
    engine.set_max_call_levels(32);
    engine.set_max_expr_depths(64, 32);
    engine.set_max_variables(256);
    engine.set_max_functions(128);
    engine.set_max_string_size(MAX_STRING_SIZE);
    engine.set_max_array_size(MAX_ARRAY_SIZE);
    engine.set_max_map_size(MAX_MAP_SIZE);
    engine
}

fn register_scene_api(engine: &mut Engine) {
    engine.register_type_with_name::<SceneBuilder>("Scene");
    engine.register_fn("scene", SceneBuilder::new);
    engine.register_fn("rect", SceneBuilder::rect);
    engine.register_fn("round_rect", SceneBuilder::round_rect);
    engine.register_fn("circle", SceneBuilder::circle);
    engine.register_fn("text", SceneBuilder::text);
    engine.register_fn("text_bold", SceneBuilder::text_bold);
    engine.register_fn("image", SceneBuilder::image);
    engine.register_fn("video", SceneBuilder::video);
    engine.register_fn("audio", SceneBuilder::audio);
    engine.register_fn("group", SceneBuilder::group);
    engine.register_fn(
        "interpolate",
        |value: FLOAT,
         input_start: FLOAT,
         input_end: FLOAT,
         output_start: FLOAT,
         output_end: FLOAT| {
            if input_start == input_end {
                return output_end;
            }
            let t = ((value - input_start) / (input_end - input_start)).clamp(0.0, 1.0);
            output_start + (output_end - output_start) * t
        },
    );
}

fn context_map(frame: u32, context: NativeCompositionContext) -> Map {
    let mut map = Map::new();
    map.insert("frame".into(), Dynamic::from(frame as INT));
    map.insert("width".into(), Dynamic::from(context.width as INT));
    map.insert("height".into(), Dynamic::from(context.height as INT));
    map.insert("fps".into(), Dynamic::from(context.fps as FLOAT));
    map.insert(
        "duration".into(),
        Dynamic::from(context.duration_in_frames as INT),
    );
    map.insert(
        "progress".into(),
        Dynamic::from(context.progress(frame) as FLOAT),
    );
    map
}

fn finite_f32(name: &str, value: FLOAT) -> RhaiResult<f32> {
    if !value.is_finite() || value < f32::MIN as FLOAT || value > f32::MAX as FLOAT {
        return Err(runtime_error(format!(
            "{name} must be a finite 32-bit number"
        )));
    }
    Ok(value as f32)
}

fn non_negative_f32(name: &str, value: FLOAT) -> RhaiResult<f32> {
    let value = finite_f32(name, value)?;
    if value < 0.0 {
        return Err(runtime_error(format!("{name} must not be negative")));
    }
    Ok(value)
}

fn finite_f64(name: &str, value: FLOAT) -> RhaiResult<f64> {
    if !value.is_finite() {
        return Err(runtime_error(format!("{name} must be finite")));
    }
    Ok(value)
}

fn non_negative_f64(name: &str, value: FLOAT) -> RhaiResult<f64> {
    let value = finite_f64(name, value)?;
    if value < 0.0 {
        return Err(runtime_error(format!("{name} must not be negative")));
    }
    Ok(value)
}

fn unit_f32(name: &str, value: FLOAT) -> RhaiResult<f32> {
    let value = finite_f32(name, value)?;
    if !(0.0..=1.0).contains(&value) {
        return Err(runtime_error(format!("{name} must be between 0.0 and 1.0")));
    }
    Ok(value)
}

fn validate_media_source(value: &str) -> RhaiResult<()> {
    if value.trim().is_empty() {
        Err(runtime_error("media source path must not be empty".into()))
    } else {
        Ok(())
    }
}

fn parse_color(value: &str) -> RhaiResult<Color> {
    Color::from_hex(value).ok_or_else(|| {
        runtime_error(format!(
            "invalid color '{value}'; expected #rrggbb or #rrggbbaa"
        ))
    })
}

fn parse_image_fit(value: &str) -> RhaiResult<ImageFit> {
    match value {
        "cover" => Ok(ImageFit::Cover),
        "contain" => Ok(ImageFit::Contain),
        "fill" => Ok(ImageFit::Fill),
        "none" => Ok(ImageFit::None),
        "scale-down" => Ok(ImageFit::ScaleDown),
        _ => Err(runtime_error(format!(
            "invalid image fit '{value}'; expected cover, contain, fill, none, or scale-down"
        ))),
    }
}

fn runtime_error(message: String) -> Box<EvalAltResult> {
    EvalAltResult::ErrorRuntime(message.into(), Position::NONE).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> NativeCompositionContext {
        NativeCompositionContext {
            width: 320,
            height: 180,
            fps: 30.0,
            duration_in_frames: 10,
        }
    }

    #[test]
    fn script_builds_a_deterministic_scene_from_context_and_props() {
        let script = r##"
            fn render(ctx, props) {
                let output = scene();
                output.rect(0.0, 0.0, ctx.width.to_float(), ctx.height.to_float(), props.background);
                let x = interpolate(ctx.frame.to_float(), 0.0, 9.0, 0.0, 90.0);
                output.text_bold(x, 80.0, props.title, 24.0, "#ffffff");
                output
            }
        "##;
        let composition = RhaiComposition::from_source("test", script).unwrap();
        let props = serde_json::json!({
            "background": "#102030",
            "title": "Hello Rhai"
        });
        let prepared = composition.prepare(&props, context()).unwrap();

        let first = prepared.render(3).unwrap();
        let second = prepared.render(3).unwrap();
        assert_eq!(first.nodes, second.nodes);
        assert!(matches!(
            &first.nodes[1],
            SceneNode::Text { x, content, .. }
                if (*x - 30.0).abs() < f32::EPSILON && content == "Hello Rhai"
        ));
    }

    #[test]
    fn operation_limit_stops_an_infinite_loop() {
        let composition = RhaiComposition::from_source(
            "infinite",
            "fn render(ctx, props) { while true {} scene() }",
        )
        .unwrap();
        let prepared = composition
            .prepare(&serde_json::json!({}), context())
            .unwrap();

        let error = prepared.render(0).unwrap_err();
        assert!(error.to_string().contains("Too many operations"));
    }

    #[test]
    fn script_builds_a_local_image_node() {
        let script = r#"
            fn render(ctx, props) {
                let output = scene();
                output.image(10.0, 20.0, 100.0, 60.0, props.src, "contain", 0.75);
                output
            }
        "#;
        let composition = RhaiComposition::from_source("image", script).unwrap();
        let prepared = composition
            .prepare(&serde_json::json!({"src": "assets/card.png"}), context())
            .unwrap();
        let scene = prepared.render(0).unwrap();

        assert!(matches!(
            &scene.nodes[0],
            SceneNode::Image { src, fit: ImageFit::Contain, opacity, .. }
                if src == "assets/card.png" && (*opacity - 0.75).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn script_rejects_invalid_image_fit() {
        let script = r#"
            fn render(ctx, props) {
                let output = scene();
                output.image(0.0, 0.0, 10.0, 10.0, "asset.png", "stretchy", 1.0);
                output
            }
        "#;
        let composition = RhaiComposition::from_source("bad-image", script).unwrap();
        let prepared = composition
            .prepare(&serde_json::json!({}), context())
            .unwrap();
        let error = prepared.render(0).unwrap_err();

        assert!(error.to_string().contains("invalid image fit"));
    }

    #[test]
    fn script_builds_video_and_audio_nodes() {
        let script = r#"
            fn render(ctx, props) {
                let output = scene();
                output.video(0.0, 0.0, 320.0, 180.0, props.video, ctx.frame.to_float() / ctx.fps, "cover", 1.0);
                output.audio(props.video, 0.25, 0.5, 2.0, 0.75, 1.25, true);
                output
            }
        "#;
        let composition = RhaiComposition::from_source("media", script).unwrap();
        let prepared = composition
            .prepare(&serde_json::json!({"video": "assets/clip.mp4"}), context())
            .unwrap();
        let scene = prepared.render(3).unwrap();

        assert!(matches!(
            &scene.nodes[0],
            SceneNode::Video { src, time, fit: ImageFit::Cover, .. }
                if src == "assets/clip.mp4" && (*time - 0.1).abs() < f64::EPSILON
        ));
        let tracks = scene.audio_tracks();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].src, "assets/clip.mp4");
        assert_eq!(tracks[0].duration, Some(2.0));
        assert!(tracks[0].looped);
    }

    #[test]
    fn unregistered_scene_api_is_rejected() {
        let composition = RhaiComposition::from_source(
            "unknown-api",
            "fn render(ctx, props) { let output = scene(); output.read_file(\"secret\"); output }",
        )
        .unwrap();
        let prepared = composition
            .prepare(&serde_json::json!({}), context())
            .unwrap();

        let error = prepared.render(0).unwrap_err();
        assert!(error.to_string().contains("Function not found"));
    }

    #[test]
    fn module_imports_are_disabled() {
        let composition = RhaiComposition::from_source(
            "import",
            "import \"untrusted\" as imported; fn render(ctx, props) { scene() }",
        )
        .unwrap();
        let prepared = composition
            .prepare(&serde_json::json!({}), context())
            .unwrap();
        let error = prepared.render(0).unwrap_err();
        assert!(
            error.to_string().contains("Module not found"),
            "unexpected import error: {error}"
        );
    }
}
