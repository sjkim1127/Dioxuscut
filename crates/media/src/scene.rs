//! Native Scene emitters matching the Dioxus media components.

use crate::img::ImageFit;
use dioxuscut_composition::{CompositionError, SceneEmitter, SceneFrameContext};
use dioxuscut_rasterizer::{AudioTrack, ImageFit as SceneImageFit, Scene, SceneNode};
use serde_json::Value;

impl From<ImageFit> for SceneImageFit {
    fn from(fit: ImageFit) -> Self {
        match fit {
            ImageFit::Cover => Self::Cover,
            ImageFit::Contain => Self::Contain,
            ImageFit::Fill => Self::Fill,
            ImageFit::None => Self::None,
            ImageFit::ScaleDown => Self::ScaleDown,
        }
    }
}

/// Native counterpart of [`crate::Img`].
#[derive(Debug, Clone, PartialEq)]
pub struct SceneImage {
    pub src: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub fit: ImageFit,
    pub opacity: f32,
}

impl SceneImage {
    pub fn new(src: impl Into<String>, width: f32, height: f32) -> Self {
        Self {
            src: src.into(),
            x: 0.0,
            y: 0.0,
            width,
            height,
            fit: ImageFit::Cover,
            opacity: 1.0,
        }
    }
}

impl SceneEmitter for SceneImage {
    fn emit(
        &self,
        _context: SceneFrameContext,
        _props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        scene.push(SceneNode::Image {
            src: self.src.clone(),
            x: self.x,
            y: self.y,
            w: self.width.max(0.0),
            h: self.height.max(0.0),
            fit: self.fit.into(),
            opacity: self.opacity.clamp(0.0, 1.0),
        });
        Ok(())
    }
}

/// Timeline-aware native counterpart of [`crate::Video`].
#[derive(Debug, Clone, PartialEq)]
pub struct SceneVideo {
    pub src: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub fit: ImageFit,
    pub opacity: f32,
    pub start_from: f64,
    pub timeline_start: f64,
    pub duration: Option<f64>,
    pub playback_rate: f64,
    pub looped: bool,
}

impl SceneVideo {
    pub fn new(src: impl Into<String>, width: f32, height: f32) -> Self {
        Self {
            src: src.into(),
            x: 0.0,
            y: 0.0,
            width,
            height,
            fit: ImageFit::Cover,
            opacity: 1.0,
            start_from: 0.0,
            timeline_start: 0.0,
            duration: None,
            playback_rate: 1.0,
            looped: false,
        }
    }
}

impl SceneEmitter for SceneVideo {
    fn emit(
        &self,
        context: SceneFrameContext,
        _props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        let elapsed = context.time_secs() - self.timeline_start;
        if elapsed < 0.0 || self.duration.is_some_and(|duration| elapsed >= duration) {
            return Ok(());
        }
        if !self.playback_rate.is_finite() || self.playback_rate <= 0.0 {
            return Err(CompositionError::render(
                context.global_frame,
                "video playback rate must be finite and greater than zero",
            ));
        }
        scene.push(SceneNode::Video {
            src: self.src.clone(),
            time: self.start_from.max(0.0) + elapsed * self.playback_rate,
            looped: self.looped,
            x: self.x,
            y: self.y,
            w: self.width.max(0.0),
            h: self.height.max(0.0),
            fit: self.fit.into(),
            opacity: self.opacity.clamp(0.0, 1.0),
        });
        Ok(())
    }
}

/// Native counterpart of [`crate::Audio`].
#[derive(Debug, Clone, PartialEq)]
pub struct SceneAudio {
    pub track: AudioTrack,
}

impl SceneAudio {
    pub fn new(src: impl Into<String>) -> Self {
        Self {
            track: AudioTrack::new(src),
        }
    }
}

impl SceneEmitter for SceneAudio {
    fn emit(
        &self,
        _context: SceneFrameContext,
        _props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        scene.push(SceneNode::Audio {
            track: self.track.clone(),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxuscut_composition::{
        NativeComposition, NativeCompositionContext, SceneEmitterComposition,
    };

    fn context() -> NativeCompositionContext {
        NativeCompositionContext {
            width: 320,
            height: 180,
            fps: 10.0,
            duration_in_frames: 100,
        }
    }

    #[test]
    fn video_emitter_applies_trim_rate_and_active_window() {
        let mut video = SceneVideo::new("clip.mp4", 320.0, 180.0);
        video.start_from = 2.0;
        video.timeline_start = 1.0;
        video.duration = Some(2.0);
        video.playback_rate = 1.5;
        let composition = SceneEmitterComposition::new("video", video);

        assert!(composition
            .render(5, &Value::Null, context())
            .unwrap()
            .nodes
            .is_empty());
        let active = composition.render(20, &Value::Null, context()).unwrap();
        assert!(matches!(
            &active.nodes[0],
            SceneNode::Video { time, .. } if (*time - 3.5).abs() < f64::EPSILON
        ));
        assert!(composition
            .render(30, &Value::Null, context())
            .unwrap()
            .nodes
            .is_empty());
    }

    #[test]
    fn image_and_audio_emit_shared_scene_nodes() {
        let image = SceneImage::new("card.png", 100.0, 50.0);
        let mut image_scene = Scene::new();
        image
            .emit(
                SceneFrameContext::new(0, context()),
                &Value::Null,
                &mut image_scene,
            )
            .unwrap();
        assert!(matches!(image_scene.nodes[0], SceneNode::Image { .. }));

        let audio = SceneAudio::new("voice.wav");
        let mut audio_scene = Scene::new();
        audio
            .emit(
                SceneFrameContext::new(0, context()),
                &Value::Null,
                &mut audio_scene,
            )
            .unwrap();
        assert_eq!(audio_scene.audio_tracks()[0].src, "voice.wav");
    }
}
