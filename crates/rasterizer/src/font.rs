//! Font loading and text rasterization via `ab_glyph`.
//!
//! Discovers a system font at runtime and caches it for the lifetime of the renderer.
//! Falls back gracefully if no font is found.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const MAX_FONT_BYTES: u64 = 32 * 1024 * 1024;

/// Platform-specific font search paths, in preference order.
#[cfg(target_os = "macos")]
const FONT_SEARCH_PATHS: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/System/Library/Fonts/Supplemental/Verdana.ttf",
    "/System/Library/Fonts/Supplemental/Georgia.ttf",
    "/System/Library/Fonts/SFNS.ttf",
];

#[cfg(target_os = "linux")]
const FONT_SEARCH_PATHS: &[&str] = &[
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
];

#[cfg(target_os = "windows")]
const FONT_SEARCH_PATHS: &[&str] = &[
    "C:\\Windows\\Fonts\\arial.ttf",
    "C:\\Windows\\Fonts\\segoeui.ttf",
    "C:\\Windows\\Fonts\\calibri.ttf",
];

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
const FONT_SEARCH_PATHS: &[&str] = &[];

/// A loaded, ready-to-use font.
pub struct FontCache {
    font: Option<Arc<FontVec>>,
    path: Option<String>,
    assets: Mutex<HashMap<String, Arc<FontVec>>>,
}

#[derive(Debug)]
pub(crate) struct FontLoadError {
    pub path: String,
    pub reason: String,
}

impl FontCache {
    /// Discover and load the first available system font.
    pub fn load() -> Self {
        for path in FONT_SEARCH_PATHS {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(font) = FontVec::try_from_vec(bytes) {
                    return Self {
                        font: Some(Arc::new(font)),
                        path: Some(path.to_string()),
                        assets: Mutex::new(HashMap::new()),
                    };
                }
            }
        }
        eprintln!("[dioxuscut-rasterizer] Warning: No system font found. Text will be rendered as blocks.");
        Self {
            font: None,
            path: None,
            assets: Mutex::new(HashMap::new()),
        }
    }

    /// Create a FontCache with no font loaded (for headless/test environments).
    pub fn headless() -> Self {
        Self {
            font: None,
            path: None,
            assets: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.font.is_some()
    }

    pub fn font_path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// Rasterize text with ordered explicit local fonts followed by the system fallback.
    pub(crate) fn rasterize(
        &self,
        text: &str,
        font_size: f32,
        sources: &[String],
    ) -> Result<Option<RenderedText>, FontLoadError> {
        let fonts = self.font_chain(sources)?;
        if fonts.is_empty() {
            return Ok(None);
        }
        let scale = PxScale::from(font_size);
        let ascent = fonts
            .iter()
            .map(|font| font.as_scaled(scale).ascent())
            .fold(0.0_f32, f32::max)
            .ceil() as u32;
        let descent = fonts
            .iter()
            .map(|font| -font.as_scaled(scale).descent())
            .fold(0.0_f32, f32::max)
            .ceil() as u32;

        let mut glyphs: Vec<(Arc<FontVec>, ab_glyph::Glyph)> = Vec::new();
        let mut cursor_x = 0.0f32;
        let mut previous: Option<(Arc<FontVec>, ab_glyph::GlyphId)> = None;

        for ch in text.chars() {
            let font = fonts
                .iter()
                .find(|font| font.glyph_id(ch).0 != 0)
                .unwrap_or(&fonts[0])
                .clone();
            let glyph_id = font.glyph_id(ch);
            let scaled = font.as_scaled(scale);
            if let Some((previous_font, previous_id)) = &previous {
                if Arc::ptr_eq(previous_font, &font) {
                    cursor_x += scaled.kern(*previous_id, glyph_id);
                }
            }
            let glyph = glyph_id.with_scale_and_position(scale, ab_glyph::point(cursor_x, 0.0));
            cursor_x += scaled.h_advance(glyph_id);
            glyphs.push((font.clone(), glyph));
            previous = Some((font, glyph_id));
        }

        let total_width = cursor_x.ceil() as u32;
        let total_height = ascent + descent;

        if total_width == 0 || total_height == 0 {
            return Ok(Some(RenderedText {
                pixels: vec![],
                width: 0,
                height: 0,
                baseline: ascent,
            }));
        }

        let mut pixels = vec![0u8; (total_width * total_height) as usize];

        for (font, glyph) in &glyphs {
            if let Some(outlined) = font.outline_glyph(glyph.clone()) {
                let bounds = outlined.px_bounds();
                let gx = bounds.min.x.floor() as i32;
                let gy = bounds.min.y.floor() as i32 + ascent as i32;

                outlined.draw(|rx, ry, coverage| {
                    let px = gx + rx as i32;
                    let py = gy + ry as i32;
                    if px >= 0 && py >= 0 {
                        let px = px as u32;
                        let py = py as u32;
                        if px < total_width && py < total_height {
                            let idx = (py * total_width + px) as usize;
                            // Accumulate coverage (clamp to 255)
                            let existing = pixels[idx] as f32 / 255.0;
                            let blended = (existing + coverage * (1.0 - existing)).min(1.0);
                            pixels[idx] = (blended * 255.0) as u8;
                        }
                    }
                });
            }
        }

        Ok(Some(RenderedText {
            pixels,
            width: total_width,
            height: total_height,
            baseline: ascent,
        }))
    }

    fn font_chain(&self, sources: &[String]) -> Result<Vec<Arc<FontVec>>, FontLoadError> {
        let mut fonts = Vec::with_capacity(sources.len() + usize::from(self.font.is_some()));
        for source in sources {
            fonts.push(self.load_asset(source)?);
        }
        if let Some(font) = &self.font {
            fonts.push(font.clone());
        }
        Ok(fonts)
    }

    fn load_asset(&self, source: &str) -> Result<Arc<FontVec>, FontLoadError> {
        let source = source.trim();
        let path = source.strip_prefix("file://").unwrap_or(source);
        if path.is_empty() {
            return Err(FontLoadError {
                path: source.into(),
                reason: "font source path must not be empty".into(),
            });
        }
        if source.contains("://") && !source.starts_with("file://") {
            return Err(FontLoadError {
                path: source.into(),
                reason: "remote font sources are not supported by native rendering".into(),
            });
        }
        if let Some(font) = self
            .assets
            .lock()
            .expect("font cache lock poisoned")
            .get(path)
            .cloned()
        {
            return Ok(font);
        }

        let metadata = std::fs::metadata(path).map_err(|error| FontLoadError {
            path: source.into(),
            reason: error.to_string(),
        })?;
        if !metadata.is_file() {
            return Err(FontLoadError {
                path: source.into(),
                reason: "font source is not a regular file".into(),
            });
        }
        if metadata.len() > MAX_FONT_BYTES {
            return Err(FontLoadError {
                path: source.into(),
                reason: format!("font exceeds the {MAX_FONT_BYTES} byte safety limit"),
            });
        }
        let bytes = std::fs::read(path).map_err(|error| FontLoadError {
            path: source.into(),
            reason: error.to_string(),
        })?;
        let font = Arc::new(FontVec::try_from_vec(bytes).map_err(|error| FontLoadError {
            path: source.into(),
            reason: format!("unsupported or invalid font data: {error:?}"),
        })?);
        self.assets
            .lock()
            .expect("font cache lock poisoned")
            .insert(path.into(), font.clone());
        Ok(font)
    }

    #[cfg(test)]
    fn asset_count(&self) -> usize {
        self.assets.lock().expect("font cache lock poisoned").len()
    }
}

/// Rasterized text as a greyscale coverage map.
#[derive(Debug)]
pub struct RenderedText {
    /// Single-channel (alpha coverage) pixel data, row-major.
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Row index of the baseline within the pixel buffer.
    pub baseline: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_font_cache_loads() {
        let cache = FontCache::load();
        // On macOS this should always succeed; on other platforms it may not
        if cache.is_loaded() {
            println!("Loaded font from: {:?}", cache.font_path());
        } else {
            println!("No system font found — placeholder mode active");
        }
    }

    #[test]
    fn test_rasterize_hello() {
        let cache = FontCache::load();
        if !cache.is_loaded() {
            return; // skip if no font available
        }
        let rendered = cache
            .rasterize("Hello", 32.0, &[])
            .expect("font load failed")
            .expect("rasterize failed");
        assert!(rendered.width > 0, "Width should be > 0");
        assert!(rendered.height > 0, "Height should be > 0");
        // At least some pixels should have coverage
        let has_coverage = rendered.pixels.iter().any(|&p| p > 0);
        assert!(has_coverage, "At least one pixel should have coverage");
    }

    #[test]
    fn test_rasterize_empty() {
        let cache = FontCache::load();
        if !cache.is_loaded() {
            return;
        }
        let rendered = cache
            .rasterize("", 24.0, &[])
            .expect("font load failed")
            .expect("rasterize failed");
        assert_eq!(rendered.width, 0, "Empty string should have 0 width");
    }

    #[test]
    fn explicit_font_sources_are_cached() {
        let Some(path) = FONT_SEARCH_PATHS
            .iter()
            .find(|path| std::path::Path::new(path).is_file())
        else {
            return;
        };
        let cache = FontCache::headless();
        let sources = vec![path.to_string()];
        let first = cache.rasterize("Explicit", 24.0, &sources).unwrap();
        let second = cache.rasterize("Explicit", 24.0, &sources).unwrap();

        assert!(first.is_some());
        assert!(second.is_some());
        assert_eq!(cache.asset_count(), 1);
    }

    #[test]
    fn missing_explicit_font_is_an_error() {
        let cache = FontCache::headless();
        let sources = vec!["/dioxuscut/does-not-exist.ttf".to_string()];
        let error = cache.rasterize("Missing", 24.0, &sources).unwrap_err();

        assert!(error.path.ends_with("does-not-exist.ttf"));
    }
}
