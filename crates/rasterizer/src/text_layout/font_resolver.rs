//! Font collection and multi-script fallback resolver.

use crate::font::{FontCache, LoadedFont};
use std::sync::Arc;

/// Priority-ordered font collection for resolving glyphs across multiple writing systems.
#[derive(Clone)]
pub struct FontResolver {
    fonts: Vec<Arc<LoadedFont>>,
}

impl FontResolver {
    /// Constructs a `FontResolver` from a `FontCache` and explicit font sources.
    pub fn new(
        font_cache: &FontCache,
        sources: &[String],
    ) -> Result<Self, crate::backend::RasterError> {
        let fonts =
            font_cache
                .font_chain(sources)
                .map_err(|e| crate::backend::RasterError::FontAsset {
                    path: e.path,
                    reason: e.reason,
                })?;
        Ok(Self { fonts })
    }

    /// Creates a resolver directly from a loaded font list.
    #[allow(dead_code)]
    pub(crate) fn from_fonts(fonts: Vec<Arc<LoadedFont>>) -> Self {
        Self { fonts }
    }

    /// Returns the resolved fonts in priority order.
    pub(crate) fn fonts(&self) -> &[Arc<LoadedFont>] {
        &self.fonts
    }

    /// Returns the primary/default font.
    #[allow(dead_code)]
    pub(crate) fn primary_font(&self) -> Option<&Arc<LoadedFont>> {
        self.fonts.first()
    }

    /// Finds the index of the first font in the chain that supports the given grapheme cluster.
    pub fn resolve_font_for_grapheme(&self, grapheme: &str) -> usize {
        for (idx, font) in self.fonts.iter().enumerate() {
            if grapheme_supported(&font.raster, grapheme) {
                return idx;
            }
        }
        0
    }
}

/// Checks whether a font rasterizer supports all characters in a grapheme cluster.
pub(crate) fn grapheme_supported(font: &ab_glyph::FontVec, grapheme: &str) -> bool {
    use ab_glyph::Font;
    grapheme.chars().all(|character| {
        font.glyph_id(character).0 != 0
            || character.is_control()
            || character.is_whitespace()
            || character == '\u{200d}'
            || ('\u{fe00}'..='\u{fe0f}').contains(&character)
            || ('\u{e0100}'..='\u{e01ef}').contains(&character)
    })
}
