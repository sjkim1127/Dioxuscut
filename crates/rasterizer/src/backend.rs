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
    #[error("Security violation for {path}: {reason}")]
    SecurityViolation { path: String, reason: String },
    #[error("Font asset error for {path}: {reason}")]
    FontAsset { path: String, reason: String },
    #[error("Scene compositing error: {0}")]
    Scene(String),
    #[error("Render cancelled")]
    Cancelled,
    #[error("Render timed out")]
    Timeout,
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

/// Capabilities exposed by a rendering backend.
///
/// A frontend (for example a Three.js/Chromium worker) can use this metadata
/// to select an appropriate scene path without guessing from the backend name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendCapabilities {
    pub native_scene: bool,
    pub browser_runtime: bool,
    pub gpu_accelerated: bool,
    pub supports_streaming: bool,
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

    /// Describe the execution environment used by this backend.
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            native_scene: true,
            browser_runtime: false,
            gpu_accelerated: false,
            supports_streaming: self.supports_streaming(),
        }
    }

    /// Whether this backend provides an internal pipelined streaming implementation
    /// (e.g. GPU double-buffered ring buffers).
    fn supports_streaming(&self) -> bool {
        false
    }

    /// Render a sequence of frames in streaming fashion with internal pipelining.
    #[allow(clippy::type_complexity)]
    fn render_stream(
        &self,
        _total: u32,
        _scene_fn: &(dyn Fn(u32) -> Result<Scene, RasterError> + Sync),
        _config_fn: &(dyn Fn(u32) -> FrameConfig + Sync),
        _consume_fn: &mut dyn FnMut(u32, &[u8]) -> Result<(), RasterError>,
    ) -> Result<(), RasterError> {
        Err(RasterError::Init(
            "Streaming is not supported on this backend".into(),
        ))
    }
}
