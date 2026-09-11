//! Positioned multi-line text layout and geometry.

use crate::text_layout::font_resolver::FontResolver;
use crate::text_layout::line_breaker::LineGlyphs;
use crate::text_layout::style::{TextAlignment, TextStyle};
use ab_glyph::{Font, PxScale, ScaleFont};
use serde::{Deserialize, Serialize};

/// 2D axis-aligned bounding rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A shaped glyph positioned precisely on canvas coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionedGlyph {
    pub font_index: usize,
    pub glyph_id: u16,
    pub x: f32,
    pub y: f32,
    pub font_size: f32,
    pub cluster: u32,
}

/// A fully laid out and aligned line of text.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutLine {
    pub glyphs: Vec<PositionedGlyph>,
    pub baseline_y: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_height: f32,
    pub width: f32,
    pub x: f32,
}

/// Completed text layout ready for deterministic rasterization.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout {
    pub lines: Vec<LayoutLine>,
    pub width: f32,
    pub height: f32,
    pub bounds: Rect,
}

impl TextLayout {
    /// Total number of positioned glyphs across all lines.
    pub fn glyph_count(&self) -> usize {
        self.lines.iter().map(|l| l.glyphs.len()).sum()
    }
}

/// Computes exact subpixel coordinates and alignment for broken lines.
pub fn compute_layout(
    lines: Vec<LineGlyphs>,
    font_resolver: &FontResolver,
    style: &TextStyle,
) -> TextLayout {
    if lines.is_empty() {
        return TextLayout {
            lines: Vec::new(),
            width: 0.0,
            height: 0.0,
            bounds: Rect::default(),
        };
    }

    let fonts = font_resolver.fonts();
    let scale = PxScale::from(style.font_size);

    // Compute ascent and descent across loaded fonts
    let ascent = fonts
        .iter()
        .map(|f| f.raster.as_scaled(scale).ascent())
        .fold(0.0_f32, f32::max)
        .max(style.font_size * 0.8);

    let descent = fonts
        .iter()
        .map(|f| -f.raster.as_scaled(scale).descent())
        .fold(0.0_f32, f32::max)
        .max(style.font_size * 0.2);

    let line_height = style.font_size * style.line_height_mult;
    let mut layout_lines = Vec::new();
    let mut current_y = 0.0_f32;
    let mut max_line_width = 0.0_f32;

    let target_width = style
        .max_width
        .unwrap_or_else(|| lines.iter().map(|l| l.width).fold(0.0_f32, f32::max));

    for line in lines {
        let baseline_y = current_y + ascent;

        // Alignment offset
        let line_x = match style.align {
            TextAlignment::Left => 0.0,
            TextAlignment::Center => ((target_width - line.width) / 2.0).max(0.0),
            TextAlignment::Right => (target_width - line.width).max(0.0),
            TextAlignment::Justify => 0.0,
        };

        let mut pen_x = line_x;
        let mut positioned_glyphs = Vec::with_capacity(line.glyphs.len());

        for g in line.glyphs {
            positioned_glyphs.push(PositionedGlyph {
                font_index: g.font_index,
                glyph_id: g.glyph_id,
                x: pen_x + g.x_offset,
                y: baseline_y + g.y_offset,
                font_size: style.font_size,
                cluster: g.cluster,
            });
            pen_x += g.x_advance;
        }

        max_line_width = max_line_width.max(line.width);
        layout_lines.push(LayoutLine {
            glyphs: positioned_glyphs,
            baseline_y,
            ascent,
            descent,
            line_height,
            width: line.width,
            x: line_x,
        });

        current_y += line_height;
    }

    let total_height = current_y.max(ascent + descent);

    TextLayout {
        lines: layout_lines,
        width: target_width.max(max_line_width),
        height: total_height,
        bounds: Rect {
            x: 0.0,
            y: 0.0,
            width: target_width.max(max_line_width),
            height: total_height,
        },
    }
}
