//! Batch frame renderer: renders all frames of a Scene sequence to PNG files.

use crate::backend::{FrameConfig, RasterError, RasterizerBackend};
use crate::scene::Scene;
use std::path::{Path, PathBuf};
use image::RgbaImage;

/// Configuration for a native render job.
#[derive(Debug, Clone)]
pub struct NativeRenderConfig {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_in_frames: u32,
    pub output_dir: PathBuf,
}

impl NativeRenderConfig {
    pub fn new(width: u32, height: u32, fps: f64, duration_in_frames: u32, output_dir: impl Into<PathBuf>) -> Self {
        Self { width, height, fps, duration_in_frames, output_dir: output_dir.into() }
    }
}

/// Render all frames by calling `scene_fn(frame)` for each frame index.
///
/// `scene_fn` receives the current frame number and should return the [`Scene`] for that frame.
/// The rendered PNG files are written to `config.output_dir` as `frame_000001.png`, etc.
pub fn render_all_frames<F>(
    backend: &dyn RasterizerBackend,
    config: &NativeRenderConfig,
    mut scene_fn: F,
) -> Result<Vec<PathBuf>, RasterError>
where
    F: FnMut(u32) -> Scene,
{
    std::fs::create_dir_all(&config.output_dir)?;

    let mut paths = Vec::with_capacity(config.duration_in_frames as usize);

    for frame in 0..config.duration_in_frames {
        let scene = scene_fn(frame);
        let frame_config = FrameConfig::new(config.width, config.height, frame, config.fps);
        let img = backend.render_frame(&scene, &frame_config)?;

        let path = config.output_dir.join(format!("frame_{:06}.png", frame + 1));
        img.save(&path).map_err(|e| RasterError::ImageEncode(e.to_string()))?;
        paths.push(path);
    }

    Ok(paths)
}

/// Save a single `RgbaImage` frame to disk.
pub fn save_frame(img: &RgbaImage, path: &Path) -> Result<(), RasterError> {
    img.save(path).map_err(|e| RasterError::ImageEncode(e.to_string()))
}
