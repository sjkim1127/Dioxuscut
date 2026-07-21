//! Font loading and text rasterization via `ab_glyph`.
//!
//! Discovers a system font at runtime and caches it for the lifetime of the renderer.
//! Falls back gracefully if no font is found.

use crate::backend::RasterError;
use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use unicode_linebreak::{linebreaks, BreakOpportunity};
use unicode_segmentation::UnicodeSegmentation;

const MAX_FONT_BYTES: u64 = 32 * 1024 * 1024;

/// Horizontal alignment inside a resolved text box.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextHorizontalAlign {
    #[default]
    Start,
    Center,
    End,
}

/// Vertical alignment inside a resolved text box.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextVerticalAlign {
    #[default]
    Start,
    Center,
    End,
}

/// Behavior when text still exceeds its box at the minimum font size.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextOverflow {
    #[default]
    Clip,
    Ellipsis,
}

/// Font-aware text box request resolved before nodes are added to a Scene.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBox {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub font_size: f32,
    pub min_font_size: f32,
    /// Multiplier applied to the resolved font size.
    pub line_height: f32,
    pub max_lines: Option<usize>,
    pub horizontal_align: TextHorizontalAlign,
    pub vertical_align: TextVerticalAlign,
    pub overflow: TextOverflow,
    pub font_sources: Vec<String>,
}

impl TextBox {
    pub fn new(
        text: impl Into<String>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        font_size: f32,
    ) -> Self {
        Self {
            text: text.into(),
            x,
            y,
            width,
            height,
            font_size,
            min_font_size: font_size,
            line_height: 1.2,
            max_lines: None,
            horizontal_align: TextHorizontalAlign::Start,
            vertical_align: TextVerticalAlign::Start,
            overflow: TextOverflow::Clip,
            font_sources: Vec::new(),
        }
    }
}

/// One baseline-positioned line produced by [`layout_text_box`].
#[derive(Debug, Clone, PartialEq)]
pub struct PositionedTextLine {
    pub text: String,
    pub x: f32,
    pub y: f32,
}

/// Resolved size and lines for deterministic native and Player rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBoxLayout {
    pub font_size: f32,
    pub line_height: f32,
    pub lines: Vec<PositionedTextLine>,
}

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
    font: Option<Arc<LoadedFont>>,
    path: Option<String>,
    assets: Mutex<HashMap<String, Arc<LoadedFont>>>,
}

struct LoadedFont {
    raster: FontVec,
    data: Arc<Vec<u8>>,
}

struct ShapedGlyph {
    font: Arc<LoadedFont>,
    glyph: ab_glyph::Glyph,
}

impl LoadedFont {
    fn from_bytes(bytes: Vec<u8>) -> Result<Self, ab_glyph::InvalidFont> {
        let raster = FontVec::try_from_vec(bytes.clone())?;
        Ok(Self {
            raster,
            data: Arc::new(bytes),
        })
    }
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
                if let Ok(font) = LoadedFont::from_bytes(bytes) {
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
        let metric_ascent = fonts
            .iter()
            .map(|font| font.raster.as_scaled(scale).ascent())
            .fold(0.0_f32, f32::max)
            .ceil();
        let metric_descent = fonts
            .iter()
            .map(|font| -font.raster.as_scaled(scale).descent())
            .fold(0.0_f32, f32::max)
            .ceil();
        let (mut glyphs, advance) = shape_runs(text, font_size, &fonts)?;
        let mut left = 0.0_f32;
        let mut right = advance.max(0.0);
        let mut top = -metric_ascent;
        let mut bottom = metric_descent;
        for glyph in &glyphs {
            if let Some(outlined) = glyph.font.raster.outline_glyph(glyph.glyph.clone()) {
                let bounds = outlined.px_bounds();
                left = left.min(bounds.min.x.floor());
                right = right.max(bounds.max.x.ceil());
                top = top.min(bounds.min.y.floor());
                bottom = bottom.max(bounds.max.y.ceil());
            }
        }
        let horizontal_shift = -left;
        for glyph in &mut glyphs {
            glyph.glyph.position.x += horizontal_shift;
        }
        let total_width = (right - left).ceil().max(0.0) as u32;
        let total_height = (bottom - top).ceil().max(0.0) as u32;
        let baseline = (-top).ceil().max(0.0) as u32;

        if total_width == 0 || total_height == 0 {
            return Ok(Some(RenderedText {
                pixels: vec![],
                width: 0,
                height: 0,
                baseline,
            }));
        }

        let mut pixels = vec![0u8; (total_width * total_height) as usize];

        for glyph in &glyphs {
            if let Some(outlined) = glyph.font.raster.outline_glyph(glyph.glyph.clone()) {
                let bounds = outlined.px_bounds();
                let gx = bounds.min.x.floor() as i32;
                let gy = bounds.min.y.floor() as i32 + baseline as i32;

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
            baseline,
        }))
    }

    fn font_chain(&self, sources: &[String]) -> Result<Vec<Arc<LoadedFont>>, FontLoadError> {
        let mut fonts = Vec::with_capacity(sources.len() + usize::from(self.font.is_some()));
        for source in sources {
            fonts.push(self.load_asset(source)?);
        }
        if let Some(font) = &self.font {
            fonts.push(font.clone());
        }
        Ok(fonts)
    }

    fn load_asset(&self, source: &str) -> Result<Arc<LoadedFont>, FontLoadError> {
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
        let font = Arc::new(
            LoadedFont::from_bytes(bytes).map_err(|error| FontLoadError {
                path: source.into(),
                reason: format!("unsupported or invalid font data: {error:?}"),
            })?,
        );
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

static TEXT_LAYOUT_FONT_CACHE: OnceLock<FontCache> = OnceLock::new();

/// Resolve Unicode line breaks, font fitting, ellipsis, and box alignment.
///
/// The returned lines retain explicit baseline positions and can therefore be
/// emitted as ordinary [`crate::scene::SceneNode::Text`] nodes for both native
/// export and Player preview.
pub fn layout_text_box(request: &TextBox) -> Result<TextBoxLayout, RasterError> {
    validate_text_box(request)?;
    let cache = TEXT_LAYOUT_FONT_CACHE.get_or_init(FontCache::load);
    let fonts = cache
        .font_chain(&request.font_sources)
        .map_err(font_asset_error)?;
    if fonts.is_empty() {
        return Err(RasterError::Scene(
            "text box layout requires an explicit or system font".into(),
        ));
    }

    let mut font_size = request.font_size;
    let mut lines;
    loop {
        lines =
            wrap_text(&request.text, request.width, font_size, &fonts).map_err(font_asset_error)?;
        let line_height = font_size * request.line_height;
        let fits_line_limit = request
            .max_lines
            .is_none_or(|maximum| lines.len() <= maximum);
        let fits_height = line_height * lines.len() as f32 <= request.height + 0.01;
        if (fits_line_limit && fits_height) || font_size <= request.min_font_size + 0.01 {
            break;
        }
        font_size = (font_size - 0.5).max(request.min_font_size);
    }

    let line_height = font_size * request.line_height;
    let height_line_limit = (request.height / line_height).floor().max(1.0) as usize;
    let allowed_lines = request
        .max_lines
        .unwrap_or(usize::MAX)
        .min(height_line_limit);
    let truncated = lines.len() > allowed_lines;
    lines.truncate(allowed_lines);
    if truncated && request.overflow == TextOverflow::Ellipsis {
        if let Some(last) = lines.last_mut() {
            add_ellipsis(last, request.width, font_size, &fonts).map_err(font_asset_error)?;
        }
    }

    let content_height = line_height * lines.len() as f32;
    let vertical_offset = match request.vertical_align {
        TextVerticalAlign::Start => 0.0,
        TextVerticalAlign::Center => (request.height - content_height).max(0.0) * 0.5,
        TextVerticalAlign::End => (request.height - content_height).max(0.0),
    };
    let ascent = fonts
        .iter()
        .map(|font| font.raster.as_scaled(PxScale::from(font_size)).ascent())
        .fold(0.0_f32, f32::max);
    let mut positioned = Vec::with_capacity(lines.len());
    for (index, line) in lines.into_iter().enumerate() {
        let width = measure_text(&line, font_size, &fonts).map_err(font_asset_error)?;
        let horizontal_offset = match request.horizontal_align {
            TextHorizontalAlign::Start => 0.0,
            TextHorizontalAlign::Center => (request.width - width).max(0.0) * 0.5,
            TextHorizontalAlign::End => (request.width - width).max(0.0),
        };
        positioned.push(PositionedTextLine {
            text: line,
            x: request.x + horizontal_offset,
            y: request.y + vertical_offset + ascent + index as f32 * line_height,
        });
    }

    Ok(TextBoxLayout {
        font_size,
        line_height,
        lines: positioned,
    })
}

/// Measure a shaped single line using the same font chain as native rendering.
pub fn measure_text_width(
    text: &str,
    font_size: f32,
    font_sources: &[String],
) -> Result<f32, RasterError> {
    if !font_size.is_finite() || font_size <= 0.0 || font_size > 4096.0 {
        return Err(RasterError::Scene(
            "text measurement font size must be between 0 and 4096".into(),
        ));
    }
    let cache = TEXT_LAYOUT_FONT_CACHE.get_or_init(FontCache::load);
    let fonts = cache.font_chain(font_sources).map_err(font_asset_error)?;
    if fonts.is_empty() {
        return Err(RasterError::Scene(
            "text measurement requires an explicit or system font".into(),
        ));
    }
    measure_text(text, font_size, &fonts).map_err(font_asset_error)
}

fn validate_text_box(request: &TextBox) -> Result<(), RasterError> {
    let finite = [
        ("x", request.x),
        ("y", request.y),
        ("width", request.width),
        ("height", request.height),
        ("font size", request.font_size),
        ("minimum font size", request.min_font_size),
        ("line height", request.line_height),
    ];
    if let Some((name, value)) = finite.iter().find(|(_, value)| !value.is_finite()) {
        return Err(RasterError::Scene(format!(
            "text box {name} must be finite, got {value}"
        )));
    }
    if request.width <= 0.0 || request.height <= 0.0 {
        return Err(RasterError::Scene(
            "text box width and height must be positive".into(),
        ));
    }
    if request.font_size <= 0.0 || request.font_size > 4096.0 {
        return Err(RasterError::Scene(
            "text box font size must be between 0 and 4096".into(),
        ));
    }
    if request.min_font_size <= 0.0 || request.min_font_size > request.font_size {
        return Err(RasterError::Scene(
            "minimum font size must be positive and no larger than font size".into(),
        ));
    }
    if !(0.5..=10.0).contains(&request.line_height) {
        return Err(RasterError::Scene(
            "text box line height multiplier must be between 0.5 and 10".into(),
        ));
    }
    if request.max_lines == Some(0) {
        return Err(RasterError::Scene(
            "text box max lines must be at least one".into(),
        ));
    }
    Ok(())
}

fn wrap_text(
    text: &str,
    max_width: f32,
    font_size: f32,
    fonts: &[Arc<LoadedFont>],
) -> Result<Vec<String>, FontLoadError> {
    if text.is_empty() {
        return Ok(vec![String::new()]);
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut segment_start = 0;
    for (break_index, opportunity) in linebreaks(text) {
        let raw_segment = &text[segment_start..break_index];
        let segment = if opportunity == BreakOpportunity::Mandatory {
            raw_segment.trim_end_matches(['\r', '\n'])
        } else {
            raw_segment
        };
        append_wrapped_segment(
            &mut lines,
            &mut current,
            segment,
            max_width,
            font_size,
            fonts,
        )?;
        if opportunity == BreakOpportunity::Mandatory {
            lines.push(current.trim_end().to_string());
            current.clear();
        }
        segment_start = break_index;
    }
    if segment_start < text.len() {
        append_wrapped_segment(
            &mut lines,
            &mut current,
            &text[segment_start..],
            max_width,
            font_size,
            fonts,
        )?;
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current.trim_end().to_string());
    }
    Ok(lines)
}

fn append_wrapped_segment(
    lines: &mut Vec<String>,
    current: &mut String,
    segment: &str,
    max_width: f32,
    font_size: f32,
    fonts: &[Arc<LoadedFont>],
) -> Result<(), FontLoadError> {
    let candidate = format!("{current}{segment}");
    if measure_text(&candidate, font_size, fonts)? <= max_width || current.is_empty() {
        *current = candidate;
    } else {
        lines.push(current.trim_end().to_string());
        *current = segment.trim_start().to_string();
    }
    if measure_text(current, font_size, fonts)? <= max_width {
        return Ok(());
    }

    let oversized = std::mem::take(current);
    let mut part = String::new();
    for grapheme in oversized.graphemes(true) {
        let candidate = format!("{part}{grapheme}");
        if !part.is_empty() && measure_text(&candidate, font_size, fonts)? > max_width {
            lines.push(part);
            part = grapheme.to_string();
        } else {
            part = candidate;
        }
    }
    *current = part;
    Ok(())
}

fn add_ellipsis(
    line: &mut String,
    max_width: f32,
    font_size: f32,
    fonts: &[Arc<LoadedFont>],
) -> Result<(), FontLoadError> {
    *line = line.trim_end().to_string();
    loop {
        let candidate = format!("{line}…");
        if measure_text(&candidate, font_size, fonts)? <= max_width || line.is_empty() {
            *line = candidate;
            return Ok(());
        }
        let Some((index, _)) = line.grapheme_indices(true).next_back() else {
            line.push('…');
            return Ok(());
        };
        line.truncate(index);
    }
}

fn measure_text(
    text: &str,
    font_size: f32,
    fonts: &[Arc<LoadedFont>],
) -> Result<f32, FontLoadError> {
    let (_, advance) = shape_runs(text, font_size, fonts)?;
    Ok(advance.abs())
}

fn font_asset_error(error: FontLoadError) -> RasterError {
    RasterError::FontAsset {
        path: error.path,
        reason: error.reason,
    }
}

fn shape_runs(
    text: &str,
    font_size: f32,
    fonts: &[Arc<LoadedFont>],
) -> Result<(Vec<ShapedGlyph>, f32), FontLoadError> {
    let mut runs: Vec<(usize, usize, usize)> = Vec::new();
    for (start, grapheme) in text.grapheme_indices(true) {
        let font_index = fonts
            .iter()
            .position(|font| grapheme_supported(&font.raster, grapheme))
            .unwrap_or(0);
        let end = start + grapheme.len();
        if let Some((last_font, _, last_end)) = runs.last_mut() {
            if *last_font == font_index && *last_end == start {
                *last_end = end;
                continue;
            }
        }
        runs.push((font_index, start, end));
    }

    let mut output = Vec::new();
    let mut cursor_x = 0.0_f32;
    for (font_index, start, end) in runs {
        let font = fonts[font_index].clone();
        let face =
            rustybuzz::Face::from_slice(font.data.as_slice(), 0).ok_or_else(|| FontLoadError {
                path: "<loaded font>".into(),
                reason: "font could not be opened by the shaping engine".into(),
            })?;
        let units_per_em = (face.units_per_em() as f32).max(1.0);
        let unit_scale = font_size / units_per_em;
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(&text[start..end]);
        buffer.guess_segment_properties();
        let shaped = rustybuzz::shape(&face, &[], buffer);
        for (info, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            let Ok(glyph_id) = u16::try_from(info.glyph_id) else {
                continue;
            };
            let x = cursor_x + position.x_offset as f32 * unit_scale;
            let y = -(position.y_offset as f32 * unit_scale);
            output.push(ShapedGlyph {
                font: font.clone(),
                glyph: ab_glyph::GlyphId(glyph_id)
                    .with_scale_and_position(PxScale::from(font_size), ab_glyph::point(x, y)),
            });
            cursor_x += position.x_advance as f32 * unit_scale;
        }
    }
    Ok((output, cursor_x))
}

fn grapheme_supported(font: &FontVec, grapheme: &str) -> bool {
    grapheme.chars().all(|character| {
        font.glyph_id(character).0 != 0
            || character.is_control()
            || character.is_whitespace()
            || character == '\u{200d}'
            || ('\u{fe00}'..='\u{fe0f}').contains(&character)
            || ('\u{e0100}'..='\u{e01ef}').contains(&character)
    })
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

    #[test]
    fn shaping_applies_standard_ligatures() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/remotion-4.0.495/packages/example/public/Roboto-Medium.ttf");
        let cache = FontCache::headless();
        let fonts = cache
            .font_chain(&[path.display().to_string()])
            .expect("fixture font should load");
        let (glyphs, advance) = shape_runs("office", 32.0, &fonts).unwrap();

        assert!(glyphs.len() < "office".chars().count());
        assert!(advance > 0.0);
    }

    fn roboto_fixture() -> String {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/remotion-4.0.495/packages/example/public/Roboto-Medium.ttf")
            .display()
            .to_string()
    }

    #[test]
    fn text_box_wraps_fits_and_adds_ellipsis() {
        let mut request = TextBox::new(
            "one two three four five six seven eight",
            10.0,
            20.0,
            120.0,
            48.0,
            30.0,
        );
        request.min_font_size = 14.0;
        request.max_lines = Some(2);
        request.horizontal_align = TextHorizontalAlign::Center;
        request.vertical_align = TextVerticalAlign::Center;
        request.overflow = TextOverflow::Ellipsis;
        request.font_sources = vec![roboto_fixture()];

        let layout = layout_text_box(&request).unwrap();

        assert!(layout.font_size < 30.0);
        assert!(layout.lines.len() <= 2);
        assert!(layout.lines.iter().all(|line| line.x >= request.x));
    }

    #[test]
    fn text_box_preserves_mandatory_line_breaks() {
        let mut request = TextBox::new("first\nsecond", 0.0, 0.0, 300.0, 100.0, 24.0);
        request.font_sources = vec![roboto_fixture()];
        let layout = layout_text_box(&request).unwrap();

        assert_eq!(layout.lines.len(), 2);
        assert_eq!(layout.lines[0].text, "first");
        assert_eq!(layout.lines[1].text, "second");
        assert!(layout.lines[1].y > layout.lines[0].y);
    }

    #[test]
    fn text_box_ellipsizes_when_minimum_size_still_overflows() {
        let mut request = TextBox::new("one two three four five six", 0.0, 0.0, 90.0, 30.0, 24.0);
        request.max_lines = Some(1);
        request.overflow = TextOverflow::Ellipsis;
        request.font_sources = vec![roboto_fixture()];
        let layout = layout_text_box(&request).unwrap();

        assert_eq!(layout.lines.len(), 1);
        assert!(layout.lines[0].text.ends_with('…'));
        assert!(
            measure_text_width(
                &layout.lines[0].text,
                layout.font_size,
                &request.font_sources
            )
            .unwrap()
                <= request.width
        );
    }

    #[test]
    fn text_box_rejects_invalid_bounds() {
        let request = TextBox::new("invalid", 0.0, 0.0, 0.0, 100.0, 24.0);
        assert!(layout_text_box(&request)
            .unwrap_err()
            .to_string()
            .contains("width and height"));
    }
}
