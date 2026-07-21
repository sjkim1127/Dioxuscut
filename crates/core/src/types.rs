//! Shared types — VideoConfig, PixelFormat, etc.

use serde::{Deserialize, Serialize};

/// Pixel format for output video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PixelFormat {
    Yuv420p,
    Yuva420p,
    Yuv422p,
    Yuv444p,
    Yuv420p10le,
    Yuva444p10le,
    Yuv444p10le,
    Yuv422p10le,
}

/// Video codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    H265,
    Vp8,
    Vp9,
    Av1,
    ProRes,
    Mp3,
    Aac,
    Wav,
    Opus,
    Gif,
}

/// The configuration of a video composition.
///
/// Equivalent to Remotion's `VideoConfig` type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoConfig {
    /// Unique composition ID.
    pub id: String,
    /// Width of the composition in pixels.
    pub width: u32,
    /// Height of the composition in pixels.
    pub height: u32,
    /// Frames per second.
    pub fps: f64,
    /// Total number of frames.
    pub duration_in_frames: u32,
    /// Default codec when rendering.
    pub default_codec: Option<Codec>,
    /// Default pixel format.
    pub default_pixel_format: Option<PixelFormat>,
}

impl VideoConfig {
    /// Total duration of the composition in seconds.
    pub fn duration_in_seconds(&self) -> f64 {
        self.duration_in_frames as f64 / self.fps
    }
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            id: "composition".to_string(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            duration_in_frames: 150,
            default_codec: Some(Codec::H264),
            default_pixel_format: Some(PixelFormat::Yuv420p),
        }
    }
}
