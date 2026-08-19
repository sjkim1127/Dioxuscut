//! Font loading and text rasterization via `ab_glyph`.
//!
//! Font discovery follows a three-step priority order:
//!
//! 1. `DIOXUSCUT_FONT_PATH` environment variable — highest priority, useful for CI overrides.
//! 2. Platform-specific system font search paths — picks up the user's installed fonts.
//! 3. Bundled fallback (`NotoSans-Regular.ttf` compiled in with `include_bytes!`) — guarantees
//!    reproducible rendering in any environment, including minimal Docker images and CI runners
//!    that have no fonts installed.
//!
//! The bundled font ensures that `FontCache::load()` always returns a loaded cache, eliminating
//! the "No system font found" warning and making output deterministic across platforms.

/// NotoSans Regular compiled into the binary as a reproducible font fallback.
///
/// Source: Google Fonts, licensed under the SIL Open Font License 1.1.
/// The font is always available regardless of the host operating system or installed fonts.
const BUNDLED_FONT: &[u8] = include_bytes!("../../../assets/fonts/NotoSans-Regular.ttf");

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
    /// Load a font using a three-step priority order:
    ///
    /// 1. `DIOXUSCUT_FONT_PATH` env var — e.g. `DIOXUSCUT_FONT_PATH=/fonts/MyFont.ttf`
    /// 2. Platform system font search paths
    /// 3. Bundled `NotoSans-Regular.ttf` — always succeeds
    ///
    /// This method never returns a cache with `is_loaded() == false`.
    pub fn load() -> Self {
        // Step 1: environment-variable override
        if let Ok(path) = std::env::var("DIOXUSCUT_FONT_PATH") {
            let path = path.trim().to_string();
            if !path.is_empty() {
                match std::fs::read(&path).and_then(|b| {
                    LoadedFont::from_bytes(b).map_err(|e| std::io::Error::other(e.to_string()))
                }) {
                    Ok(font) => {
                        tracing::info!(font_path = %path, "Loaded font from DIOXUSCUT_FONT_PATH");
                        return Self {
                            font: Some(Arc::new(font)),
                            path: Some(path),
                            assets: Mutex::new(HashMap::new()),
                        };
                    }
                    Err(err) => {
                        tracing::warn!(
                            font_path = %path,
                            error = %err,
                            "DIOXUSCUT_FONT_PATH could not be loaded, falling back"
                        );
                    }
                }
            }
        }

        // Step 2: system search paths
        for path in FONT_SEARCH_PATHS {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(font) = LoadedFont::from_bytes(bytes) {
                    tracing::debug!(font_path = %path, "Loaded system font");
                    return Self {
                        font: Some(Arc::new(font)),
                        path: Some(path.to_string()),
                        assets: Mutex::new(HashMap::new()),
                    };
                }
            }
        }

        // Step 3: bundled fallback — always succeeds
        tracing::debug!("No system font found; using bundled NotoSans-Regular");
        Self::bundled()
    }

    /// Return a `FontCache` loaded with the bundled `NotoSans-Regular.ttf`.
    ///
    /// Unlike [`Self::load`], this skips env-var and system-font discovery and
    /// directly uses the font compiled into the binary. Useful for tests that
    /// need a deterministic font independent of the host machine.
    pub fn bundled() -> Self {
        let font = LoadedFont::from_bytes(BUNDLED_FONT.to_vec())
            .expect("bundled NotoSans-Regular.ttf is a valid font");
        Self {
            font: Some(Arc::new(font)),
            path: Some("<bundled:NotoSans-Regular>".into()),
            assets: Mutex::new(HashMap::new()),
        }
    }

    /// Create a FontCache with no font loaded (for tests that verify tofu-box rendering).
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

/// Find the largest font size at which `text` fits within `max_width`.
///
/// Binary search between `min_font_size` and `max_font_size` with sub-pixel precision.
/// Returns the font size that just fits, or `min_font_size` if text cannot fit.
///
/// # Arguments
/// * `text` - the text to measure
/// * `max_width` - maximum allowed width in pixels
/// * `font_sources` - font file paths (same as `measure_text_width`)
/// * `min_font_size` - lower bound for binary search (must be positive)
/// * `max_font_size` - upper bound for binary search (must be >= `min_font_size` and <= 4096.0)
pub fn fit_text(
    text: &str,
    max_width: f64,
    font_sources: &[String],
    min_font_size: f64,
    max_font_size: f64,
) -> Result<f64, RasterError> {
    if !max_width.is_finite() || !min_font_size.is_finite() || !max_font_size.is_finite() {
        return Err(RasterError::Scene(
            "fit_text parameters must be finite".into(),
        ));
    }
    if max_width <= 0.0 {
        return Err(RasterError::Scene(
            "fit_text max_width must be positive".into(),
        ));
    }
    if min_font_size <= 0.0 {
        return Err(RasterError::Scene(
            "fit_text min_font_size must be positive".into(),
        ));
    }
    if max_font_size < min_font_size {
        return Err(RasterError::Scene(
            "fit_text max_font_size must be greater than or equal to min_font_size".into(),
        ));
    }
    if max_font_size > 4096.0 {
        return Err(RasterError::Scene(
            "fit_text max_font_size must not exceed 4096".into(),
        ));
    }

    if text.is_empty() {
        return Ok(max_font_size);
    }

    let max_width_actual = f64::from(measure_text_width(
        text,
        max_font_size as f32,
        font_sources,
    )?);
    if max_width_actual <= max_width {
        return Ok(max_font_size);
    }

    let min_width_actual = f64::from(measure_text_width(
        text,
        min_font_size as f32,
        font_sources,
    )?);
    if min_width_actual > max_width {
        return Ok(min_font_size);
    }

    let mut lo = min_font_size;
    let mut hi = max_font_size;
    let mut best = min_font_size;

    // Up to 25 iterations of binary search → precision ~0.1px
    for _ in 0..25 {
        if hi - lo < 0.1 {
            break;
        }
        let mid = (lo + hi) / 2.0;
        let width = f64::from(measure_text_width(text, mid as f32, font_sources)?);
        if width <= max_width {
            best = mid;
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(best)
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
    fn fit_text_finds_font_size_within_width() {
        let cache = FontCache::bundled();
        let _font_path = cache.font_path().unwrap().to_string();
        let sources: Vec<String> = vec![];
        let size = fit_text("Hello World", 200.0, &sources, 8.0, 48.0);
        assert!(size.is_ok(), "fit_text should succeed");
        let size = size.unwrap();
        assert!(
            (8.0..=48.0).contains(&size),
            "font size {size} out of range"
        );
        let measured = measure_text_width("Hello World", size as f32, &sources).unwrap();
        assert!(
            f64::from(measured) <= 200.0,
            "measured width {measured} should be <= max_width 200.0"
        );
    }

    #[test]
    fn fit_text_exact_max_font_size_for_short_text() {
        let sources: Vec<String> = vec![];
        let size = fit_text("Hi", 1000.0, &sources, 10.0, 50.0).unwrap();
        assert_eq!(
            size, 50.0,
            "should return max_font_size when text easily fits"
        );
    }

    #[test]
    fn fit_text_empty_string_returns_max_font_size() {
        let sources: Vec<String> = vec![];
        let size = fit_text("", 200.0, &sources, 8.0, 48.0).unwrap();
        assert_eq!(size, 48.0, "empty text should return max_font_size");
    }

    #[test]
    fn fit_text_returns_min_font_size_when_overflows() {
        let sources: Vec<String> = vec![];
        let size = fit_text(
            "This is a very long sentence that cannot possibly fit in a tiny width",
            5.0,
            &sources,
            12.0,
            64.0,
        )
        .unwrap();
        assert_eq!(
            size, 12.0,
            "should return min_font_size when text cannot fit"
        );
    }

    #[test]
    fn fit_text_equal_min_and_max_font_size() {
        let sources: Vec<String> = vec![];
        // Fits:
        let size_fit = fit_text("Hello", 500.0, &sources, 24.0, 24.0).unwrap();
        assert_eq!(size_fit, 24.0);

        // Does not fit:
        let size_overflow = fit_text("Hello World Very Long", 5.0, &sources, 24.0, 24.0).unwrap();
        assert_eq!(size_overflow, 24.0);
    }

    #[test]
    fn fit_text_rejects_non_finite_inputs() {
        let sources: Vec<String> = vec![];
        assert!(fit_text("test", f64::NAN, &sources, 8.0, 48.0).is_err());
        assert!(fit_text("test", 200.0, &sources, f64::NAN, 48.0).is_err());
        assert!(fit_text("test", 200.0, &sources, 8.0, f64::NAN).is_err());
        assert!(fit_text("test", f64::INFINITY, &sources, 8.0, 48.0).is_err());
        assert!(fit_text("test", 200.0, &sources, f64::INFINITY, 48.0).is_err());
        assert!(fit_text("test", 200.0, &sources, 8.0, f64::INFINITY).is_err());
    }

    #[test]
    fn fit_text_rejects_invalid_bounds() {
        let sources: Vec<String> = vec![];
        // Non-positive max_width
        assert!(fit_text("test", 0.0, &sources, 8.0, 48.0).is_err());
        assert!(fit_text("test", -100.0, &sources, 8.0, 48.0).is_err());

        // Non-positive min_font_size
        assert!(fit_text("test", 200.0, &sources, 0.0, 48.0).is_err());
        assert!(fit_text("test", 200.0, &sources, -10.0, 48.0).is_err());

        // max_font_size < min_font_size
        assert!(fit_text("test", 200.0, &sources, 50.0, 20.0).is_err());

        // max_font_size > 4096.0
        assert!(fit_text("test", 200.0, &sources, 10.0, 5000.0).is_err());
    }

    #[test]
    fn fit_text_propagates_font_load_error_for_invalid_source() {
        let sources = vec!["/nonexistent/path/custom_font.ttf".to_string()];
        let err = fit_text("test", 200.0, &sources, 8.0, 48.0);
        assert!(err.is_err());
        match err.unwrap_err() {
            RasterError::FontAsset { path, .. } => {
                assert!(path.contains("custom_font.ttf"));
            }
            other => panic!("expected RasterError::FontAsset, got {other:?}"),
        }
    }

    #[test]
    fn measure_text_width_empty_string() {
        let width = measure_text_width("", 24.0, &[]).unwrap();
        assert_eq!(width, 0.0);
    }

    #[test]
    fn measure_text_width_scales_with_font_size() {
        let w1 = measure_text_width("Scaling Test", 16.0, &[]).unwrap();
        let w2 = measure_text_width("Scaling Test", 32.0, &[]).unwrap();
        assert!(w1 > 0.0);
        assert!(
            w2 > w1 * 1.8,
            "doubling font size should roughly double width"
        );
    }

    #[test]
    fn measure_text_width_scales_with_text_length() {
        let w_short = measure_text_width("Short", 20.0, &[]).unwrap();
        let w_long = measure_text_width("Short and much longer text", 20.0, &[]).unwrap();
        assert!(w_long > w_short);
    }

    #[test]
    fn measure_text_width_rejects_invalid_font_size() {
        assert!(measure_text_width("test", f32::NAN, &[]).is_err());
        assert!(measure_text_width("test", 0.0, &[]).is_err());
        assert!(measure_text_width("test", -5.0, &[]).is_err());
        assert!(measure_text_width("test", 5000.0, &[]).is_err());
    }

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
    fn shaping_produces_positioned_glyphs() {
        let Some(path) = font_fixture() else {
            return;
        };
        let cache = FontCache::headless();
        let fonts = cache
            .font_chain(&[path])
            .expect("system fixture font should load");
        let (glyphs, advance) = shape_runs("office", 32.0, &fonts).unwrap();

        assert!(!glyphs.is_empty());
        assert!(glyphs.len() <= "office".chars().count());
        assert!(advance > 0.0);
    }

    fn font_fixture() -> Option<String> {
        FontCache::load().font_path().map(str::to_owned)
    }

    #[test]
    fn text_box_wraps_fits_and_adds_ellipsis() {
        let Some(font) = font_fixture() else {
            return;
        };
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
        request.font_sources = vec![font];

        let layout = layout_text_box(&request).unwrap();

        assert!(layout.font_size < 30.0);
        assert!(layout.lines.len() <= 2);
        assert!(layout.lines.iter().all(|line| line.x >= request.x));
    }

    #[test]
    fn text_box_preserves_mandatory_line_breaks() {
        let Some(font) = font_fixture() else {
            return;
        };
        let mut request = TextBox::new("first\nsecond", 0.0, 0.0, 300.0, 100.0, 24.0);
        request.font_sources = vec![font];
        let layout = layout_text_box(&request).unwrap();

        assert_eq!(layout.lines.len(), 2);
        assert_eq!(layout.lines[0].text, "first");
        assert_eq!(layout.lines[1].text, "second");
        assert!(layout.lines[1].y > layout.lines[0].y);
    }

    #[test]
    fn text_box_ellipsizes_when_minimum_size_still_overflows() {
        let Some(font) = font_fixture() else {
            return;
        };
        let mut request = TextBox::new("one two three four five six", 0.0, 0.0, 90.0, 30.0, 24.0);
        request.max_lines = Some(1);
        request.overflow = TextOverflow::Ellipsis;
        request.font_sources = vec![font];
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

    // ── Bundled font tests ────────────────────────────────────────────────────

    #[test]
    fn font_cache_load_always_returns_a_loaded_cache() {
        // FontCache::load() must never return a headless (unloaded) cache,
        // because the bundled NotoSans fallback is always compiled in.
        let cache = FontCache::load();
        assert!(
            cache.is_loaded(),
            "FontCache::load() must always be loaded (bundled font is the last resort fallback)"
        );
        assert!(
            cache.font_path().is_some(),
            "font_path() must be Some after a successful load"
        );
    }

    #[test]
    fn font_cache_bundled_is_always_loaded() {
        let cache = FontCache::bundled();
        assert!(
            cache.is_loaded(),
            "FontCache::bundled() must always be loaded"
        );
        assert_eq!(
            cache.font_path(),
            Some("<bundled:NotoSans-Regular>"),
            "font_path() should identify the bundled font"
        );
    }

    #[test]
    fn bundled_font_can_rasterize_ascii_text() {
        let cache = FontCache::bundled();
        // Rasterize a short ASCII string — must succeed and produce non-empty pixels.
        let result = cache.rasterize("Hello", 24.0, &[]).unwrap();
        let rendered = result.expect("bundled font must produce rendered text for ASCII");
        assert!(rendered.width > 0, "rendered text must have non-zero width");
        assert!(
            rendered.height > 0,
            "rendered text must have non-zero height"
        );
        assert!(
            !rendered.pixels.is_empty(),
            "rendered text must have pixel data"
        );
    }

    #[test]
    fn env_override_takes_precedence_over_bundled() {
        // When DIOXUSCUT_FONT_PATH points to a non-existent file,
        // FontCache::load() must still succeed via the bundled fallback.
        std::env::set_var("DIOXUSCUT_FONT_PATH", "/nonexistent/path/font.ttf");
        let cache = FontCache::load();
        std::env::remove_var("DIOXUSCUT_FONT_PATH");
        assert!(
            cache.is_loaded(),
            "FontCache::load() must fall back to bundled when env path is invalid"
        );
    }
}
