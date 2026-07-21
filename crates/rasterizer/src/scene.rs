//! Scene graph intermediate representation for native rendering.
//!
//! Compositions write into a [`Scene`] instead of DOM nodes.
//! The rasterizer backend reads the scene and produces pixel output.

use serde::{Deserialize, Serialize};

/// RGBA color: `[red, green, blue, alpha]` each in `0..=255`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 255)
    }

    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    /// Parse a CSS hex color: `"#rrggbb"` or `"#rrggbbaa"`.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let s = hex.trim_start_matches('#');
        match s.len() {
            6 => {
                let r = u8::from_str_radix(&s[0..2], 16).ok()?;
                let g = u8::from_str_radix(&s[2..4], 16).ok()?;
                let b = u8::from_str_radix(&s[4..6], 16).ok()?;
                Some(Self::rgb(r, g, b))
            }
            8 => {
                let r = u8::from_str_radix(&s[0..2], 16).ok()?;
                let g = u8::from_str_radix(&s[2..4], 16).ok()?;
                let b = u8::from_str_radix(&s[4..6], 16).ok()?;
                let a = u8::from_str_radix(&s[6..8], 16).ok()?;
                Some(Self::rgba(r, g, b, a))
            }
            _ => None,
        }
    }

    /// Apply opacity (0.0 – 1.0) to this colour.
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.a = (self.a as f32 * opacity.clamp(0.0, 1.0)) as u8;
        self
    }

    pub fn to_tiny_skia_color(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba8(self.r, self.g, self.b, self.a)
    }
}

/// A gradient colour stop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub position: f32, // 0.0 – 1.0
    pub color: Color,
}

/// A node in the scene graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SceneNode {
    /// Solid-coloured rectangle with optional corner radius.
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: Color,
        stroke: Option<Color>,
        stroke_width: f32,
        corner_radius: f32,
    },

    /// Circle.
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        fill: Color,
        stroke: Option<Color>,
        stroke_width: f32,
    },

    /// Arbitrary SVG path string (M, L, C, Q, Z commands).
    Path {
        d: String,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_width: f32,
        opacity: f32,
    },

    /// Single-line text.
    Text {
        x: f32,
        y: f32,
        content: String,
        font_size: f32,
        color: Color,
        font_weight: u16, // 400 = normal, 700 = bold
    },

    /// Linear gradient background covering a rectangle.
    LinearGradient {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        angle_deg: f32,
        stops: Vec<GradientStop>,
    },

    /// Radial gradient background.
    RadialGradient {
        cx: f32,
        cy: f32,
        r: f32,
        stops: Vec<GradientStop>,
    },

    /// Group with a 2D transform applied to all children.
    Group {
        transform: Transform2D,
        opacity: f32,
        children: Vec<SceneNode>,
    },
}

/// Affine 2D transform (scale + rotation + translation).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transform2D {
    pub tx: f32,
    pub ty: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rotate_deg: f32,
}

impl Default for Transform2D {
    fn default() -> Self {
        Self {
            tx: 0.0,
            ty: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotate_deg: 0.0,
        }
    }
}

impl Transform2D {
    pub fn to_tiny_skia(&self) -> tiny_skia::Transform {
        tiny_skia::Transform::from_translate(self.tx, self.ty)
            .post_scale(self.scale_x, self.scale_y)
    }
}

/// The root container for a single video frame's scene.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scene {
    pub nodes: Vec<SceneNode>,
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, node: SceneNode) {
        self.nodes.push(node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_from_hex() {
        let c = Color::from_hex("#ff6600").unwrap();
        assert_eq!(c.r, 0xff);
        assert_eq!(c.g, 0x66);
        assert_eq!(c.b, 0x00);
        assert_eq!(c.a, 0xff);
    }

    #[test]
    fn test_scene_push() {
        let mut scene = Scene::new();
        scene.push(SceneNode::Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
            fill: Color::rgb(255, 0, 0),
            stroke: None,
            stroke_width: 0.0,
            corner_radius: 0.0,
        });
        assert_eq!(scene.nodes.len(), 1);
    }
}
