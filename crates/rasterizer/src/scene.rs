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

    /// Parse common CSS color forms used by Dioxuscut primitives.
    pub fn from_css(value: &str) -> Option<Self> {
        let value = value.trim();
        if let Some(color) = Self::from_hex(value) {
            return Some(color);
        }
        match value.to_ascii_lowercase().as_str() {
            "black" => return Some(Self::BLACK),
            "white" => return Some(Self::WHITE),
            "transparent" => return Some(Self::TRANSPARENT),
            _ => {}
        }

        let (contents, has_alpha) = if let Some(contents) = value
            .strip_prefix("rgba(")
            .and_then(|value| value.strip_suffix(')'))
        {
            (contents, true)
        } else {
            let contents = value
                .strip_prefix("rgb(")
                .and_then(|value| value.strip_suffix(')'))?;
            (contents, false)
        };
        let components = contents.split(',').map(str::trim).collect::<Vec<_>>();
        if components.len() != if has_alpha { 4 } else { 3 } {
            return None;
        }
        let channel = |value: &str| -> Option<u8> {
            if let Some(percent) = value.strip_suffix('%') {
                let percent = percent.parse::<f32>().ok()?;
                Some((percent.clamp(0.0, 100.0) * 2.55).round() as u8)
            } else {
                Some(value.parse::<f32>().ok()?.clamp(0.0, 255.0).round() as u8)
            }
        };
        let alpha = if has_alpha {
            let value = components[3];
            if let Some(percent) = value.strip_suffix('%') {
                (percent.parse::<f32>().ok()?.clamp(0.0, 100.0) * 2.55).round() as u8
            } else {
                (value.parse::<f32>().ok()?.clamp(0.0, 1.0) * 255.0).round() as u8
            }
        } else {
            255
        };
        Some(Self::rgba(
            channel(components[0])?,
            channel(components[1])?,
            channel(components[2])?,
            alpha,
        ))
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

/// How a raster image is fitted into its destination rectangle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageFit {
    /// Preserve aspect ratio and fill the destination, cropping overflow.
    #[default]
    Cover,
    /// Preserve aspect ratio and show the complete image with transparent letterboxing.
    Contain,
    /// Stretch the image to exactly match the destination.
    Fill,
    /// Keep the image at its natural pixel size, centered and clipped.
    None,
    /// Use the natural size unless the image must be reduced to fit.
    ScaleDown,
}

/// Layer blend mode shared by CPU rendering and SVG preview.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
}

/// Geometric clip applied to a composited layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClipRegion {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        corner_radius: f32,
    },
    Path {
        d: String,
    },
}

/// How rendered mask nodes are converted to coverage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaskMode {
    #[default]
    Alpha,
    Luminance,
}

/// Pixel filter applied to an offscreen layer in declaration order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SceneFilter {
    Blur { sigma: f32 },
    Brightness { amount: f32 },
    Grayscale { amount: f32 },
    Opacity { amount: f32 },
}

/// Drop shadow generated from the filtered and masked layer alpha.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_sigma: f32,
    pub color: Color,
}

const fn default_opacity() -> f32 {
    1.0
}

/// Audio source mixed into the encoded output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioTrack {
    /// Local audio or video file containing an audio stream.
    pub src: String,
    /// Source offset in seconds.
    pub start_from: f64,
    /// Composition timeline offset in seconds.
    pub timeline_start: f64,
    /// Optional audible duration on the composition timeline.
    pub duration: Option<f64>,
    /// Linear gain in `0.0..=1.0`.
    pub volume: f64,
    /// Playback speed supported by FFmpeg `atempo` (`0.5..=2.0`).
    pub playback_rate: f64,
    /// Repeat the source when it is shorter than the requested duration.
    pub looped: bool,
}

impl AudioTrack {
    pub fn new(src: impl Into<String>) -> Self {
        Self {
            src: src.into(),
            start_from: 0.0,
            timeline_start: 0.0,
            duration: None,
            volume: 1.0,
            playback_rate: 1.0,
            looped: false,
        }
    }
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

    /// Local raster image asset. `src` may be a filesystem path or `file://` URI.
    Image {
        src: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fit: ImageFit,
        opacity: f32,
    },

    /// A decoded frame from a local video file at `time` seconds.
    Video {
        src: String,
        time: f64,
        /// Repeat the video timeline after the source duration.
        #[serde(default)]
        looped: bool,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fit: ImageFit,
        opacity: f32,
    },

    /// Non-visual audio track collected by the output encoder.
    Audio { track: AudioTrack },

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

    /// Offscreen compositing boundary with filters, clipping, masking, shadow,
    /// and a destination blend mode.
    Layer {
        #[serde(default = "default_opacity")]
        opacity: f32,
        #[serde(default)]
        blend_mode: BlendMode,
        #[serde(default)]
        clip: Option<ClipRegion>,
        #[serde(default)]
        mask: Option<Vec<SceneNode>>,
        #[serde(default)]
        mask_mode: MaskMode,
        #[serde(default)]
        filters: Vec<SceneFilter>,
        #[serde(default)]
        shadow: Option<SceneShadow>,
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
            .post_rotate(self.rotate_deg)
    }
}

/// The root container for a single video frame's scene.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

    /// Collect audio declared in this scene, including nested groups and layers.
    pub fn audio_tracks(&self) -> Vec<AudioTrack> {
        fn collect(nodes: &[SceneNode], parent_volume: f64, output: &mut Vec<AudioTrack>) {
            for node in nodes {
                match node {
                    SceneNode::Audio { track } => {
                        let mut track = track.clone();
                        track.volume *= parent_volume;
                        output.push(track);
                    }
                    SceneNode::Group {
                        opacity, children, ..
                    } => collect(children, parent_volume * f64::from(*opacity), output),
                    SceneNode::Layer {
                        opacity, children, ..
                    } => collect(children, parent_volume * f64::from(*opacity), output),
                    _ => {}
                }
            }
        }

        let mut tracks = Vec::new();
        collect(&self.nodes, 1.0, &mut tracks);
        tracks
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
    fn css_rgb_and_rgba_colors_are_parsed() {
        assert_eq!(
            Color::from_css("rgb(255, 0, 128)"),
            Some(Color::rgb(255, 0, 128))
        );
        assert_eq!(
            Color::from_css("rgba(100%, 0%, 0%, 0.5)"),
            Some(Color::rgba(255, 0, 0, 128))
        );
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

    #[test]
    fn image_node_round_trips_through_json() {
        let scene = Scene {
            nodes: vec![SceneNode::Image {
                src: "assets/card.png".into(),
                x: 10.0,
                y: 20.0,
                w: 320.0,
                h: 180.0,
                fit: ImageFit::Contain,
                opacity: 0.75,
            }],
        };

        let json = serde_json::to_string(&scene).unwrap();
        assert!(json.contains("contain"));
        assert_eq!(serde_json::from_str::<Scene>(&json).unwrap(), scene);
    }

    #[test]
    fn video_loop_defaults_to_false_for_existing_scene_json() {
        let json = r#"{
            "Video": {
                "src": "clip.mp4", "time": 0.0,
                "x": 0.0, "y": 0.0, "w": 10.0, "h": 10.0,
                "fit": "cover", "opacity": 1.0
            }
        }"#;
        let node = serde_json::from_str::<SceneNode>(json).unwrap();
        assert!(matches!(node, SceneNode::Video { looped: false, .. }));
    }

    #[test]
    fn nested_audio_tracks_inherit_group_opacity() {
        let scene = Scene {
            nodes: vec![SceneNode::Group {
                transform: Transform2D::default(),
                opacity: 0.5,
                children: vec![SceneNode::Audio {
                    track: AudioTrack::new("sound.wav"),
                }],
            }],
        };

        let tracks = scene.audio_tracks();
        assert_eq!(tracks.len(), 1);
        assert!((tracks[0].volume - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn nested_audio_tracks_inherit_layer_opacity() {
        let mut track = AudioTrack::new("voice.wav");
        track.volume = 0.8;
        let scene = Scene {
            nodes: vec![SceneNode::Layer {
                opacity: 0.25,
                blend_mode: BlendMode::Normal,
                clip: None,
                mask: None,
                mask_mode: MaskMode::Alpha,
                filters: Vec::new(),
                shadow: None,
                children: vec![SceneNode::Audio { track }],
            }],
        };

        assert!((scene.audio_tracks()[0].volume - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn layer_deserialization_defaults_compositing_options() {
        let node: SceneNode = serde_json::from_str(r#"{"Layer":{"children":[]}}"#).unwrap();
        assert!(matches!(
            node,
            SceneNode::Layer {
                opacity,
                blend_mode: BlendMode::Normal,
                mask_mode: MaskMode::Alpha,
                ..
            } if (opacity - 1.0).abs() < f32::EPSILON
        ));
    }
}
