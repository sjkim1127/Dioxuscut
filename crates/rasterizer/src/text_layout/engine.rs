//! High-level facade for text shaping, bidi analysis, and layout.

use crate::backend::RasterError;
use crate::font::FontCache;
use crate::text_layout::bidi::segment_bidi_runs;
use crate::text_layout::font_resolver::FontResolver;
use crate::text_layout::layout::{compute_layout, Rect, TextLayout};
use crate::text_layout::line_breaker::break_lines;
use crate::text_layout::shaper::shape_bidi_runs;
use crate::text_layout::style::TextStyle;
use std::sync::Arc;

/// Subsystem engine responsible for end-to-end typography shaping and multi-line layout.
#[derive(Clone)]
pub struct TextLayoutEngine {
    font_cache: Arc<FontCache>,
}

impl TextLayoutEngine {
    pub fn new(font_cache: Arc<FontCache>) -> Self {
        Self { font_cache }
    }

    /// Full layout pipeline from input text and style to positioned glyph runs.
    pub fn layout(&self, text: &str, style: &TextStyle) -> Result<TextLayout, RasterError> {
        if text.is_empty() {
            return Ok(TextLayout {
                lines: Vec::new(),
                width: 0.0,
                height: 0.0,
                bounds: Rect::default(),
            });
        }

        // 1. Resolve font fallback chain
        let resolver = FontResolver::new(&self.font_cache, &style.font_sources)?;

        // 2. UAX #9 Bidirectional segmentation
        let bidi_runs = segment_bidi_runs(text, style.direction);

        // 3. HarfBuzz OpenType shaping via rustybuzz
        let shaped_runs = shape_bidi_runs(text, &bidi_runs, &resolver, style)?;

        // 4. UAX #14 Line breaking and word wrapping
        let broken_lines = break_lines(text, &shaped_runs, style.max_width, style.max_lines);

        // 5. Positioned glyph coordinates and alignment
        let layout = compute_layout(broken_lines, &resolver, style);

        Ok(layout)
    }

    /// Measures the dimensions of a text layout without rasterizing pixels.
    pub fn measure(&self, text: &str, style: &TextStyle) -> Result<Rect, RasterError> {
        let layout = self.layout(text, style)?;
        Ok(layout.bounds)
    }
}
