//! Video encoding — stitches rendered frame PNGs into a video file via FFmpeg.

use std::path::PathBuf;
use crate::render_frames::RenderError;

/// Configuration for the encoding step.
#[derive(Debug, Clone)]
pub struct EncodeConfig {
    /// Directory containing `frame-000000.png` files.
    pub frames_dir: PathBuf,
    /// Output video file path.
    pub output: PathBuf,
    /// Frames per second.
    pub fps: f64,
    /// CRF quality (lower = better; 0–51 for H.264).
    pub crf: u32,
    /// Video codec string for FFmpeg (e.g., `"libx264"`).
    pub codec: String,
    /// Pixel format string for FFmpeg (e.g., `"yuv420p"`).
    pub pixel_format: String,
}

impl EncodeConfig {
    /// Create an H.264 config with sensible defaults.
    pub fn h264(frames_dir: impl Into<PathBuf>, output: impl Into<PathBuf>, fps: f64) -> Self {
        Self {
            frames_dir: frames_dir.into(),
            output: output.into(),
            fps,
            crf: 18,
            codec: "libx264".to_string(),
            pixel_format: "yuv420p".to_string(),
        }
    }
}

/// Encode rendered frames into a video file using FFmpeg.
///
/// Requires `ffmpeg` to be available in `PATH`.
pub async fn encode_frames(config: &EncodeConfig) -> Result<(), RenderError> {
    let input_pattern = config
        .frames_dir
        .join("frame-%06d.png")
        .to_string_lossy()
        .to_string();

    tracing::info!(
        "Encoding {} → {} (codec={}, crf={}, fps={})",
        input_pattern,
        config.output.display(),
        config.codec,
        config.crf,
        config.fps,
    );

    let status = tokio::process::Command::new("ffmpeg")
        .args([
            "-y",                          // overwrite output
            "-framerate", &config.fps.to_string(),
            "-i",         &input_pattern,
            "-c:v",       &config.codec,
            "-crf",       &config.crf.to_string(),
            "-pix_fmt",   &config.pixel_format,
            config.output.to_str().unwrap_or("output.mp4"),
        ])
        .status()
        .await
        .map_err(|e| RenderError::Encode(e.to_string()))?;

    if !status.success() {
        return Err(RenderError::Encode(format!(
            "ffmpeg exited with status: {status}"
        )));
    }

    tracing::info!("Encode complete → {}", config.output.display());
    Ok(())
}
