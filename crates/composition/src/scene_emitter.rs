//! Composable browser-free scene emitters.

use crate::{CompositionError, NativeComposition, NativeCompositionContext};
use dioxuscut_rasterizer::{
    layout_text_box, BlendMode, ClipRegion, Color, GradientStop, MaskMode, Scene, SceneFilter,
    SceneNode, SceneShadow, TextBox, TextHorizontalAlign, TextOverflow, TextVerticalAlign,
    Transform2D,
};
use serde_json::Value;

/// Timeline state passed through a native scene-emitter tree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneFrameContext {
    /// Frame local to the current sequence or freeze boundary.
    pub frame: u32,
    /// Unmodified composition frame.
    pub global_frame: u32,
    pub composition: NativeCompositionContext,
}

impl SceneFrameContext {
    pub fn new(frame: u32, composition: NativeCompositionContext) -> Self {
        Self {
            frame,
            global_frame: frame,
            composition,
        }
    }

    pub fn time_secs(self) -> f64 {
        self.frame as f64 / self.composition.fps
    }

    pub fn global_time_secs(self) -> f64 {
        self.global_frame as f64 / self.composition.fps
    }

    fn with_local_frame(self, frame: u32) -> Self {
        Self { frame, ..self }
    }
}

/// Shared primitive contract for media, shapes, captions, transitions, and apps.
///
/// Emitters append nodes to the supplied scene and can be nested through
/// [`SceneSequence`], [`SceneFreeze`], [`SceneGroup`], and [`SceneStack`].
pub trait SceneEmitter: Send + Sync {
    fn emit(
        &self,
        context: SceneFrameContext,
        props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError>;
}

impl<F> SceneEmitter for F
where
    F: Fn(SceneFrameContext, &Value, &mut Scene) -> Result<(), CompositionError> + Send + Sync,
{
    fn emit(
        &self,
        context: SceneFrameContext,
        props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        self(context, props, scene)
    }
}

impl SceneEmitter for SceneNode {
    fn emit(
        &self,
        _context: SceneFrameContext,
        _props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        scene.push(self.clone());
        Ok(())
    }
}

/// Heterogeneous emitter collection rendered in insertion order.
#[derive(Default)]
pub struct SceneStack {
    children: Vec<Box<dyn SceneEmitter>>,
}

impl SceneStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, child: impl SceneEmitter + 'static) {
        self.children.push(Box::new(child));
    }

    pub fn with(mut self, child: impl SceneEmitter + 'static) -> Self {
        self.push(child);
        self
    }

    pub fn len(&self) -> usize {
        self.children.len()
    }

    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

impl SceneEmitter for SceneStack {
    fn emit(
        &self,
        context: SceneFrameContext,
        props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        for child in &self.children {
            child.emit(context, props, scene)?;
        }
        Ok(())
    }
}

/// Native equivalent of Dioxuscut's `<Sequence>` timeline boundary.
pub struct SceneSequence<E> {
    pub from: u32,
    pub duration_in_frames: Option<u32>,
    pub hidden: bool,
    pub child: E,
}

impl<E> SceneSequence<E> {
    pub fn new(from: u32, child: E) -> Self {
        Self {
            from,
            duration_in_frames: None,
            hidden: false,
            child,
        }
    }

    pub fn with_duration(mut self, duration_in_frames: u32) -> Self {
        self.duration_in_frames = Some(duration_in_frames);
        self
    }

    pub fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }
}

impl<E: SceneEmitter> SceneEmitter for SceneSequence<E> {
    fn emit(
        &self,
        context: SceneFrameContext,
        props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        let end = self
            .duration_in_frames
            .map(|duration| self.from.saturating_add(duration))
            .unwrap_or(u32::MAX);
        if self.hidden || context.frame < self.from || context.frame >= end {
            return Ok(());
        }
        self.child.emit(
            context.with_local_frame(context.frame - self.from),
            props,
            scene,
        )
    }
}

/// Native equivalent of `<Freeze>`.
pub struct SceneFreeze<E> {
    pub frame: u32,
    pub child: E,
}

impl<E> SceneFreeze<E> {
    pub fn new(frame: u32, child: E) -> Self {
        Self { frame, child }
    }
}

impl<E: SceneEmitter> SceneEmitter for SceneFreeze<E> {
    fn emit(
        &self,
        context: SceneFrameContext,
        props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        self.child
            .emit(context.with_local_frame(self.frame), props, scene)
    }
}

/// Applies the Scene graph's transform and opacity primitive to emitted children.
pub struct SceneGroup<E> {
    pub transform: Transform2D,
    pub opacity: f32,
    pub child: E,
}

impl<E> SceneGroup<E> {
    pub fn new(child: E) -> Self {
        Self {
            transform: Transform2D::default(),
            opacity: 1.0,
            child,
        }
    }

    pub fn with_transform(mut self, transform: Transform2D) -> Self {
        self.transform = transform;
        self
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }
}

impl<E: SceneEmitter> SceneEmitter for SceneGroup<E> {
    fn emit(
        &self,
        context: SceneFrameContext,
        props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        let mut child_scene = Scene::new();
        self.child.emit(context, props, &mut child_scene)?;
        if !child_scene.nodes.is_empty() {
            scene.push(SceneNode::Group {
                transform: self.transform,
                opacity: self.opacity,
                children: child_scene.nodes,
            });
        }
        Ok(())
    }
}

/// Creates an offscreen compositing boundary around emitted children.
pub struct SceneLayer<E> {
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub clip: Option<ClipRegion>,
    pub mask: Option<Vec<SceneNode>>,
    pub mask_mode: MaskMode,
    pub filters: Vec<SceneFilter>,
    pub shadow: Option<SceneShadow>,
    pub child: E,
}

impl<E> SceneLayer<E> {
    pub fn new(child: E) -> Self {
        Self {
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            clip: None,
            mask: None,
            mask_mode: MaskMode::Alpha,
            filters: Vec::new(),
            shadow: None,
            child,
        }
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    pub fn with_blend_mode(mut self, blend_mode: BlendMode) -> Self {
        self.blend_mode = blend_mode;
        self
    }

    pub fn with_clip(mut self, clip: ClipRegion) -> Self {
        self.clip = Some(clip);
        self
    }

    pub fn with_mask(mut self, nodes: impl IntoIterator<Item = SceneNode>, mode: MaskMode) -> Self {
        self.mask = Some(nodes.into_iter().collect());
        self.mask_mode = mode;
        self
    }

    pub fn with_filter(mut self, filter: SceneFilter) -> Self {
        self.filters.push(filter);
        self
    }

    pub fn with_shadow(mut self, shadow: SceneShadow) -> Self {
        self.shadow = Some(shadow);
        self
    }
}

impl<E: SceneEmitter> SceneEmitter for SceneLayer<E> {
    fn emit(
        &self,
        context: SceneFrameContext,
        props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        let mut child_scene = Scene::new();
        self.child.emit(context, props, &mut child_scene)?;
        if !child_scene.nodes.is_empty() {
            scene.push(SceneNode::Layer {
                opacity: self.opacity,
                blend_mode: self.blend_mode,
                clip: self.clip.clone(),
                mask: self.mask.clone(),
                mask_mode: self.mask_mode,
                filters: self.filters.clone(),
                shadow: self.shadow.clone(),
                children: child_scene.nodes,
            });
        }
        Ok(())
    }
}

/// Font-aware multiline text that resolves into ordinary baseline Text nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneTextBlock {
    pub layout: TextBox,
    pub color: Color,
    pub font_weight: u16,
}

impl SceneTextBlock {
    pub fn new(
        text: impl Into<String>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        font_size: f32,
    ) -> Self {
        Self {
            layout: TextBox::new(text, x, y, width, height, font_size),
            color: Color::WHITE,
            font_weight: 400,
        }
    }

    pub fn with_min_font_size(mut self, min_font_size: f32) -> Self {
        self.layout.min_font_size = min_font_size;
        self
    }

    pub fn with_line_height(mut self, line_height: f32) -> Self {
        self.layout.line_height = line_height;
        self
    }

    pub fn with_max_lines(mut self, max_lines: usize) -> Self {
        self.layout.max_lines = Some(max_lines);
        self
    }

    pub fn with_alignment(
        mut self,
        horizontal: TextHorizontalAlign,
        vertical: TextVerticalAlign,
    ) -> Self {
        self.layout.horizontal_align = horizontal;
        self.layout.vertical_align = vertical;
        self
    }

    pub fn with_overflow(mut self, overflow: TextOverflow) -> Self {
        self.layout.overflow = overflow;
        self
    }

    pub fn with_font_sources(mut self, sources: impl IntoIterator<Item = String>) -> Self {
        self.layout.font_sources = sources.into_iter().collect();
        self
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn with_font_weight(mut self, font_weight: u16) -> Self {
        self.font_weight = font_weight;
        self
    }
}

impl SceneEmitter for SceneTextBlock {
    fn emit(
        &self,
        context: SceneFrameContext,
        _props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        let layout = layout_text_box(&self.layout).map_err(|error| {
            CompositionError::render(
                context.global_frame,
                format!("failed to layout text block: {error}"),
            )
        })?;
        for line in layout.lines {
            if line.text.is_empty() {
                continue;
            }
            scene.push(SceneNode::Text {
                x: line.x,
                y: line.y,
                content: line.text,
                font_size: layout.font_size,
                color: self.color,
                font_weight: self.font_weight,
                font_sources: self.layout.font_sources.clone(),
            });
        }
        Ok(())
    }
}

/// Adapts an emitter tree to the existing composition registry contract.
pub struct SceneEmitterComposition<E> {
    id: String,
    root: E,
}

impl<E> SceneEmitterComposition<E> {
    pub fn new(id: impl Into<String>, root: E) -> Self {
        Self {
            id: id.into(),
            root,
        }
    }

    pub fn root(&self) -> &E {
        &self.root
    }
}

impl<E: SceneEmitter> NativeComposition for SceneEmitterComposition<E> {
    fn id(&self) -> &str {
        &self.id
    }

    fn render(
        &self,
        frame: u32,
        props: &Value,
        context: NativeCompositionContext,
    ) -> Result<Scene, CompositionError> {
        let mut scene = Scene::new();
        self.root
            .emit(SceneFrameContext::new(frame, context), props, &mut scene)?;
        Ok(scene)
    }
}

// ── Primitive emitters ────────────────────────────────────────────────────────

/// Solid-colour rectangle placed directly on the scene.
///
/// Equivalent to emitting a [`SceneNode::Rect`] without having to construct it manually.
/// Use [`SceneTextBlock`] for multi-line text layout.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub fill: Color,
    pub stroke: Option<Color>,
    pub stroke_width: f32,
    pub corner_radius: f32,
}

impl SceneRect {
    pub fn new(x: f32, y: f32, w: f32, h: f32, fill: Color) -> Self {
        Self {
            x,
            y,
            w: w.max(0.0),
            h: h.max(0.0),
            fill,
            stroke: None,
            stroke_width: 0.0,
            corner_radius: 0.0,
        }
    }

    pub fn with_stroke(mut self, color: Color, width: f32) -> Self {
        self.stroke = Some(color);
        self.stroke_width = width.max(0.0);
        self
    }

    pub fn with_corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius.max(0.0);
        self
    }
}

impl SceneEmitter for SceneRect {
    fn emit(
        &self,
        _context: SceneFrameContext,
        _props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        scene.push(SceneNode::Rect {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
            fill: self.fill,
            stroke: self.stroke,
            stroke_width: self.stroke_width,
            corner_radius: self.corner_radius,
        });
        Ok(())
    }
}

/// Single-line text placed directly on the scene.
///
/// Use [`SceneTextBlock`] when you need word-wrapping, multi-line layout, or
/// alignment/overflow control. `SceneText` is a zero-cost shortcut for placing
/// a pre-positioned single text run.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneText {
    pub x: f32,
    pub y: f32,
    pub content: String,
    pub font_size: f32,
    pub color: Color,
    pub font_weight: u16,
    pub font_sources: Vec<String>,
}

impl SceneText {
    pub fn new(content: impl Into<String>, x: f32, y: f32, font_size: f32, color: Color) -> Self {
        Self {
            x,
            y,
            content: content.into(),
            font_size: font_size.max(0.0),
            color,
            font_weight: 400,
            font_sources: Vec::new(),
        }
    }

    pub fn with_font_weight(mut self, weight: u16) -> Self {
        self.font_weight = weight;
        self
    }

    pub fn with_font_sources(mut self, sources: impl IntoIterator<Item = String>) -> Self {
        self.font_sources = sources.into_iter().collect();
        self
    }
}

impl SceneEmitter for SceneText {
    fn emit(
        &self,
        _context: SceneFrameContext,
        _props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        if self.content.is_empty() {
            return Ok(());
        }
        scene.push(SceneNode::Text {
            x: self.x,
            y: self.y,
            content: self.content.clone(),
            font_size: self.font_size,
            color: self.color,
            font_weight: self.font_weight,
            font_sources: self.font_sources.clone(),
        });
        Ok(())
    }
}

/// Linear gradient rectangle placed directly on the scene.
///
/// A convenience wrapper around [`SceneNode::LinearGradient`] for use inside
/// emitter trees.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneLinearGradient {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub angle_deg: f32,
    pub stops: Vec<GradientStop>,
}

impl SceneLinearGradient {
    /// Horizontal left-to-right gradient (0°) spanning the given rectangle.
    pub fn new(x: f32, y: f32, w: f32, h: f32, stops: Vec<GradientStop>) -> Self {
        Self {
            x,
            y,
            w: w.max(0.0),
            h: h.max(0.0),
            angle_deg: 0.0,
            stops,
        }
    }

    pub fn with_angle(mut self, angle_deg: f32) -> Self {
        self.angle_deg = angle_deg;
        self
    }
}

impl SceneEmitter for SceneLinearGradient {
    fn emit(
        &self,
        _context: SceneFrameContext,
        _props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        scene.push(SceneNode::LinearGradient {
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
            angle_deg: self.angle_deg,
            stops: self.stops.clone(),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxuscut_rasterizer::{Color, ImageFit};

    fn context() -> NativeCompositionContext {
        NativeCompositionContext {
            width: 320,
            height: 180,
            fps: 30.0,
            duration_in_frames: 90,
        }
    }

    fn frame_text() -> impl SceneEmitter {
        |context: SceneFrameContext, _props: &Value, scene: &mut Scene| {
            scene.push(SceneNode::Text {
                x: 0.0,
                y: 20.0,
                content: context.frame.to_string(),
                font_size: 20.0,
                color: Color::WHITE,
                font_weight: 400,
                font_sources: Vec::new(),
            });
            Ok(())
        }
    }

    #[test]
    fn sequence_uses_local_frames_and_duration() {
        let composition = SceneEmitterComposition::new(
            "sequence",
            SceneSequence::new(10, frame_text()).with_duration(5),
        );
        let active = composition.render(12, &Value::Null, context()).unwrap();
        let inactive = composition.render(15, &Value::Null, context()).unwrap();

        assert!(matches!(
            &active.nodes[0],
            SceneNode::Text { content, .. } if content == "2"
        ));
        assert!(inactive.nodes.is_empty());
    }

    #[test]
    fn freeze_replaces_only_the_local_frame() {
        let composition = SceneEmitterComposition::new("freeze", SceneFreeze::new(7, frame_text()));
        let scene = composition.render(42, &Value::Null, context()).unwrap();
        assert!(matches!(
            &scene.nodes[0],
            SceneNode::Text { content, .. } if content == "7"
        ));
    }

    #[test]
    fn stack_and_group_preserve_primitive_order() {
        let image = SceneNode::Image {
            src: "card.png".into(),
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
            fit: ImageFit::Contain,
            opacity: 1.0,
        };
        let stack = SceneStack::new().with(image).with(frame_text());
        let composition =
            SceneEmitterComposition::new("group", SceneGroup::new(stack).with_opacity(0.5));
        let scene = composition.render(3, &Value::Null, context()).unwrap();

        assert!(matches!(
            &scene.nodes[0],
            SceneNode::Group { opacity, children, .. }
                if (*opacity - 0.5).abs() < f32::EPSILON && children.len() == 2
        ));
    }

    #[test]
    fn layer_collects_compositing_options_and_children() {
        let layer = SceneLayer::new(frame_text())
            .with_opacity(0.75)
            .with_blend_mode(BlendMode::Multiply)
            .with_clip(ClipRegion::Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
                corner_radius: 8.0,
            })
            .with_filter(SceneFilter::Grayscale { amount: 1.0 })
            .with_shadow(SceneShadow {
                offset_x: 4.0,
                offset_y: 6.0,
                blur_sigma: 3.0,
                color: Color::rgba(0, 0, 0, 128),
            });
        let composition = SceneEmitterComposition::new("layer", layer);
        let scene = composition.render(3, &Value::Null, context()).unwrap();

        assert!(matches!(
            &scene.nodes[0],
            SceneNode::Layer {
                opacity,
                blend_mode: BlendMode::Multiply,
                clip: Some(ClipRegion::Rect { .. }),
                filters,
                shadow: Some(_),
                children,
                ..
            } if (*opacity - 0.75).abs() < f32::EPSILON
                && filters.len() == 1
                && children.len() == 1
        ));
    }

    #[test]
    fn text_block_resolves_wrapping_fitting_and_alignment() {
        let font_cache = dioxuscut_rasterizer::FontCache::load();
        let Some(font) = font_cache.font_path() else {
            return;
        };
        let block =
            SceneTextBlock::new("one two three four five six", 10.0, 20.0, 120.0, 52.0, 32.0)
                .with_min_font_size(14.0)
                .with_max_lines(2)
                .with_alignment(TextHorizontalAlign::Center, TextVerticalAlign::Center)
                .with_overflow(TextOverflow::Ellipsis)
                .with_font_sources([font.to_string()]);
        let composition = SceneEmitterComposition::new("text-block", block);
        let scene = composition.render(0, &Value::Null, context()).unwrap();

        assert!(!scene.nodes.is_empty());
        assert!(scene.nodes.len() <= 2);
        assert!(scene.nodes.iter().all(|node| matches!(
            node,
            SceneNode::Text { x, font_size, .. } if *x >= 10.0 && *font_size <= 32.0
        )));
    }

    #[test]
    fn scene_rect_emits_correct_rect_node() {
        let rect = SceneRect::new(10.0, 20.0, 100.0, 50.0, Color::rgb(255, 0, 0))
            .with_stroke(Color::WHITE, 2.0)
            .with_corner_radius(8.0);
        let composition = SceneEmitterComposition::new("rect", rect);
        let scene = composition.render(0, &Value::Null, context()).unwrap();

        assert_eq!(scene.nodes.len(), 1);
        assert!(matches!(
            &scene.nodes[0],
            SceneNode::Rect { x, y, w, h, fill, stroke: Some(_), corner_radius, .. }
                if (*x - 10.0).abs() < f32::EPSILON
                    && (*y - 20.0).abs() < f32::EPSILON
                    && (*w - 100.0).abs() < f32::EPSILON
                    && (*h - 50.0).abs() < f32::EPSILON
                    && *fill == Color::rgb(255, 0, 0)
                    && (*corner_radius - 8.0).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn scene_rect_clamps_negative_dimensions_to_zero() {
        let rect = SceneRect::new(0.0, 0.0, -10.0, -5.0, Color::BLACK);
        let composition = SceneEmitterComposition::new("rect-clamp", rect);
        let scene = composition.render(0, &Value::Null, context()).unwrap();

        assert!(matches!(
            &scene.nodes[0],
            SceneNode::Rect { w, h, .. } if *w == 0.0 && *h == 0.0
        ));
    }

    #[test]
    fn scene_text_emits_correct_text_node() {
        let text = SceneText::new("Hello", 5.0, 30.0, 24.0, Color::WHITE).with_font_weight(700);
        let composition = SceneEmitterComposition::new("text", text);
        let scene = composition.render(0, &Value::Null, context()).unwrap();

        assert_eq!(scene.nodes.len(), 1);
        assert!(matches!(
            &scene.nodes[0],
            SceneNode::Text { x, y, content, font_size, font_weight, .. }
                if (*x - 5.0).abs() < f32::EPSILON
                    && (*y - 30.0).abs() < f32::EPSILON
                    && content == "Hello"
                    && (*font_size - 24.0).abs() < f32::EPSILON
                    && *font_weight == 700
        ));
    }

    #[test]
    fn scene_text_emits_nothing_for_empty_content() {
        let text = SceneText::new("", 0.0, 0.0, 16.0, Color::WHITE);
        let composition = SceneEmitterComposition::new("text-empty", text);
        let scene = composition.render(0, &Value::Null, context()).unwrap();
        assert!(scene.nodes.is_empty());
    }

    #[test]
    fn scene_linear_gradient_emits_correct_gradient_node() {
        use dioxuscut_rasterizer::GradientStop;
        let gradient = SceneLinearGradient::new(
            0.0,
            0.0,
            320.0,
            180.0,
            vec![
                GradientStop {
                    position: 0.0,
                    color: Color::BLACK,
                },
                GradientStop {
                    position: 1.0,
                    color: Color::WHITE,
                },
            ],
        )
        .with_angle(135.0);
        let composition = SceneEmitterComposition::new("grad", gradient);
        let scene = composition.render(0, &Value::Null, context()).unwrap();

        assert_eq!(scene.nodes.len(), 1);
        assert!(matches!(
            &scene.nodes[0],
            SceneNode::LinearGradient { w, h, angle_deg, stops, .. }
                if (*w - 320.0).abs() < f32::EPSILON
                    && (*h - 180.0).abs() < f32::EPSILON
                    && (*angle_deg - 135.0).abs() < f32::EPSILON
                    && stops.len() == 2
        ));
    }
}
