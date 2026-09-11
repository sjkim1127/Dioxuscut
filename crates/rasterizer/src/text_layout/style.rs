//! Typography and text layout styling options.

use crate::scene::Color;
use serde::{Deserialize, Serialize};

/// Horizontal text alignment within a bounding box.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextAlignment {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

/// Base paragraph text direction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextDirection {
    /// Automatically determine direction from Unicode characters (UAX #9).
    #[default]
    Auto,
    /// Force Left-to-Right.
    Ltr,
    /// Force Right-to-Left (e.g. Arabic, Hebrew).
    Rtl,
}

/// Comprehensive typography style configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    /// Font size in canvas pixels.
    pub font_size: f32,
    /// Nominal font weight (e.g. 400 for Normal, 700 for Bold).
    pub font_weight: u16,
    /// Additional tracking / letter spacing in pixels.
    pub letter_spacing: f32,
    /// Line height multiplier relative to font size (e.g. 1.25).
    pub line_height_mult: f32,
    /// Text fill color.
    pub color: Color,
    /// Horizontal text alignment.
    pub align: TextAlignment,
    /// Base paragraph direction.
    pub direction: TextDirection,
    /// Maximum bounding width for multi-line wrapping (None = single-line unwrapped).
    pub max_width: Option<f32>,
    /// Maximum number of lines before truncating.
    pub max_lines: Option<usize>,
    /// Ordered local font file paths or `<bundled:...>` asset URIs.
    pub font_sources: Vec<String>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_size: 32.0,
            font_weight: 400,
            letter_spacing: 0.0,
            line_height_mult: 1.25,
            color: Color::WHITE,
            align: TextAlignment::Left,
            direction: TextDirection::Auto,
            max_width: None,
            max_lines: None,
            font_sources: Vec::new(),
        }
    }
}

impl TextStyle {
    pub fn new(font_size: f32) -> Self {
        Self {
            font_size,
            ..Default::default()
        }
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn with_weight(mut self, weight: u16) -> Self {
        self.font_weight = weight;
        self
    }

    pub fn with_max_width(mut self, width: f32) -> Self {
        self.max_width = Some(width);
        self
    }

    pub fn with_align(mut self, align: TextAlignment) -> Self {
        self.align = align;
        self
    }

    pub fn with_direction(mut self, direction: TextDirection) -> Self {
        self.direction = direction;
        self
    }

    pub fn with_letter_spacing(mut self, spacing: f32) -> Self {
        self.letter_spacing = spacing;
        self
    }

    pub fn with_line_height_mult(mut self, mult: f32) -> Self {
        self.line_height_mult = mult;
        self
    }

    pub fn with_font_sources(
        mut self,
        sources: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.font_sources = sources.into_iter().map(Into::into).collect();
        self
    }
}
