//! Composable browser-free scene emitters.

use crate::{CompositionError, NativeComposition, NativeCompositionContext};
use dioxuscut_rasterizer::{
    BlendMode, ClipRegion, MaskMode, Scene, SceneFilter, SceneNode, SceneShadow, Transform2D,
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
}
