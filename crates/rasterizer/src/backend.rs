//! Rasterizer backend trait and frame configuration.

use crate::scene::Scene;
use image::RgbaImage;
use thiserror::Error;

/// Error type for rasterization failures.
#[derive(Error, Debug)]
pub enum RasterError {
    #[error("Backend initialization failed: {0}")]
    Init(String),
    #[error("Failed to render frame {frame}: {reason}")]
    Frame { frame: u32, reason: String },
    #[error("Image encode error: {0}")]
    ImageEncode(String),
    #[error("Image asset error for {path}: {reason}")]
    ImageAsset { path: String, reason: String },
    #[error("Media asset error for {path}: {reason}")]
    MediaAsset { path: String, reason: String },
    #[error("Scene compositing error: {0}")]
    Scene(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Per-frame rendering configuration.
#[derive(Debug, Clone)]
pub struct FrameConfig {
    pub width: u32,
    pub height: u32,
    pub frame: u32,
    pub fps: f64,
}

impl FrameConfig {
    pub fn new(width: u32, height: u32, frame: u32, fps: f64) -> Self {
        Self {
            width,
            height,
            frame,
            fps,
        }
    }

    /// Current playback time in seconds.
    pub fn time_secs(&self) -> f64 {
        self.frame as f64 / self.fps
    }
}

/// Trait implemented by every rasterizer backend.
pub trait RasterizerBackend: Send + Sync {
    /// Render a single `Scene` into an `RgbaImage`.
    fn render_frame(&self, scene: &Scene, config: &FrameConfig) -> Result<RgbaImage, RasterError>;
}
