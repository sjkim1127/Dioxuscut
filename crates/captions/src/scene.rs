//! Native Scene emitter for timed kinetic captions.

use crate::{create_tiktok_style_captions, CaptionToken};
use dioxuscut_composition::{CompositionError, SceneEmitter, SceneFrameContext};
use dioxuscut_rasterizer::{Color, Scene, SceneNode};
use serde_json::Value;

/// Native counterpart of [`crate::TikTokCaptions`].
#[derive(Debug, Clone, PartialEq)]
pub struct SceneCaptions {
    pub tokens: Vec<CaptionToken>,
    pub max_words_per_page: usize,
    pub center_x: f32,
    pub baseline_y: f32,
    pub font_size: f32,
    pub font_weight: u16,
    pub active_color: String,
    pub inactive_color: String,
    pub active_scale: f32,
    pub word_gap: f32,
}

impl SceneCaptions {
    pub fn new(tokens: Vec<CaptionToken>, center_x: f32, baseline_y: f32) -> Self {
        Self {
            tokens,
            max_words_per_page: 3,
            center_x,
            baseline_y,
            font_size: 48.0,
            font_weight: 800,
            active_color: "#ffe600".into(),
            inactive_color: "#ffffff".into(),
            active_scale: 1.15,
            word_gap: 14.0,
        }
    }
}

impl SceneEmitter for SceneCaptions {
    fn emit(
        &self,
        context: SceneFrameContext,
        _props: &Value,
        scene: &mut Scene,
    ) -> Result<(), CompositionError> {
        let current_ms = (context.time_secs().max(0.0) * 1000.0).round() as u64;
        let pages = create_tiktok_style_captions(&self.tokens, self.max_words_per_page);
        let Some(page) = pages
            .iter()
            .find(|page| current_ms >= page.start_ms && current_ms <= page.end_ms)
        else {
            return Ok(());
        };
        let active_color = parse_color(&self.active_color, context)?;
        let inactive_color = parse_color(&self.inactive_color, context)?;

        let metrics = page
            .tokens
            .iter()
            .map(|token| {
                let active = current_ms >= token.start_ms && current_ms <= token.end_ms;
                let size = if active {
                    self.font_size * self.active_scale.max(0.0)
                } else {
                    self.font_size
                };
                let width = estimated_text_width(&token.text, size);
                (token, active, size, width)
            })
            .collect::<Vec<_>>();
        let row_width = metrics.iter().map(|(_, _, _, width)| *width).sum::<f32>()
            + self.word_gap.max(0.0) * metrics.len().saturating_sub(1) as f32;
        let mut x = self.center_x - row_width * 0.5;
        for (token, active, size, width) in metrics {
            scene.push(SceneNode::Text {
                x,
                y: self.baseline_y,
                content: token.text.clone(),
                font_size: size,
                color: if active { active_color } else { inactive_color },
                font_weight: self.font_weight,
            });
            x += width + self.word_gap.max(0.0);
        }
        Ok(())
    }
}

fn estimated_text_width(text: &str, font_size: f32) -> f32 {
    text.chars()
        .map(|character| if character.is_ascii() { 0.6 } else { 1.0 })
        .sum::<f32>()
        * font_size
}

fn parse_color(value: &str, context: SceneFrameContext) -> Result<Color, CompositionError> {
    Color::from_css(value).ok_or_else(|| {
        CompositionError::render(
            context.global_frame,
            format!("unsupported native caption color '{value}'"),
        )
    })
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
            duration_in_frames: 30,
        }
    }

    #[test]
    fn active_caption_page_emits_individually_colored_words() {
        let captions = SceneCaptions::new(
            vec![
                CaptionToken::new("Hello", 0, 500),
                CaptionToken::new("world", 500, 1000),
            ],
            160.0,
            120.0,
        );
        let composition = SceneEmitterComposition::new("captions", captions);
        let scene = composition.render(7, &Value::Null, context()).unwrap();

        assert_eq!(scene.nodes.len(), 2);
        assert!(matches!(
            &scene.nodes[0],
            SceneNode::Text { content, color, .. }
                if content == "Hello" && *color == Color::WHITE
        ));
        assert!(matches!(
            &scene.nodes[1],
            SceneNode::Text { content, color, font_size, .. }
                if content == "world"
                    && *color == Color::rgb(255, 230, 0)
                    && (*font_size - 55.2).abs() < 0.01
        ));
    }

    #[test]
    fn captions_emit_nothing_outside_timed_pages() {
        let captions =
            SceneCaptions::new(vec![CaptionToken::new("Later", 1000, 1500)], 160.0, 120.0);
        let composition = SceneEmitterComposition::new("captions", captions);
        assert!(composition
            .render(0, &Value::Null, context())
            .unwrap()
            .nodes
            .is_empty());
    }
}
