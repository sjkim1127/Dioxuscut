//! Font loading and text rasterization via `ab_glyph`.
//!
//! Discovers a system font at runtime and caches it for the lifetime of the renderer.
//! Falls back gracefully if no font is found.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};

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
    font: Option<FontVec>,
    path: Option<String>,
}

impl FontCache {
    /// Discover and load the first available system font.
    pub fn load() -> Self {
        for path in FONT_SEARCH_PATHS {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(font) = FontVec::try_from_vec(bytes) {
                    return Self {
                        font: Some(font),
                        path: Some(path.to_string()),
                    };
                }
            }
        }
        eprintln!("[dioxuscut-rasterizer] Warning: No system font found. Text will be rendered as blocks.");
        Self { font: None, path: None }
    }

    /// Create a FontCache with no font loaded (for headless/test environments).
    pub fn headless() -> Self {
        Self { font: None, path: None }
    }

    pub fn is_loaded(&self) -> bool {
        self.font.is_some()
    }

    pub fn font_path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// Rasterize a string into a list of `(x_offset, y_offset, coverage)` pixel contributions.
    ///
    /// Returns `None` if no font is loaded.
    pub fn rasterize(
        &self,
        text: &str,
        font_size: f32,
    ) -> Option<RenderedText> {
        let font = self.font.as_ref()?;
        let scale = PxScale::from(font_size);
        let scaled = font.as_scaled(scale);

        let mut glyphs: Vec<ab_glyph::Glyph> = Vec::new();
        let mut cursor_x = 0.0f32;
        let mut prev_glyph_id = None;

        for ch in text.chars() {
            let glyph_id = font.glyph_id(ch);
            // Apply kerning
            if let Some(prev) = prev_glyph_id {
                cursor_x += scaled.kern(prev, glyph_id);
            }
            let glyph = glyph_id.with_scale_and_position(scale, ab_glyph::point(cursor_x, 0.0));
            cursor_x += scaled.h_advance(glyph_id);
            glyphs.push(glyph);
            prev_glyph_id = Some(glyph_id);
        }

        let total_width = cursor_x.ceil() as u32;
        let ascent  = scaled.ascent().ceil() as u32;
        let descent = (-scaled.descent()).ceil() as u32;
        let total_height = ascent + descent;

        if total_width == 0 || total_height == 0 {
            return Some(RenderedText {
                pixels: vec![],
                width: 0,
                height: 0,
                baseline: ascent,
            });
        }

        let mut pixels = vec![0u8; (total_width * total_height) as usize];

        for glyph in &glyphs {
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

        Some(RenderedText {
            pixels,
            width: total_width,
            height: total_height,
            baseline: ascent,
        })
    }
}

/// Rasterized text as a greyscale coverage map.
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
        let rendered = cache.rasterize("Hello", 32.0).expect("rasterize failed");
        assert!(rendered.width > 0, "Width should be > 0");
        assert!(rendered.height > 0, "Height should be > 0");
        // At least some pixels should have coverage
        let has_coverage = rendered.pixels.iter().any(|&p| p > 0);
        assert!(has_coverage, "At least one pixel should have coverage");
    }

    #[test]
    fn test_rasterize_empty() {
        let cache = FontCache::load();
        if !cache.is_loaded() { return; }
        let rendered = cache.rasterize("", 24.0).expect("rasterize failed");
        assert_eq!(rendered.width, 0, "Empty string should have 0 width");
    }
}
