//! Thread-safe Lottie animation asset cache and headless rasterizer.

use crate::backend::RasterError;
use image::RgbaImage;
use rasterlottie::{Animation, RenderConfig, Renderer};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Cached Lottie animation entry.
struct CachedLottie {
    animation: Animation,
    width: f32,
    height: f32,
    total_frames: f32,
    frame_rate: f32,
}

type LottieFrameKey = (PathBuf, u32, u32, u32);

#[derive(Default)]
pub(crate) struct LottieCache {
    animations: Mutex<HashMap<PathBuf, Arc<CachedLottie>>>,
    rendered_frames: Mutex<HashMap<LottieFrameKey, Arc<RgbaImage>>>,
}

impl LottieCache {
    /// Loads or retrieves a parsed Lottie animation.
    fn get_or_load(&self, src: &str) -> Result<Arc<CachedLottie>, RasterError> {
        let path = local_path(src)?;
        let canonical = path.canonicalize().unwrap_or(path);

        let mut cache = self.animations.lock().expect("lottie cache poisoned");
        if let Some(entry) = cache.get(&canonical) {
            return Ok(Arc::clone(entry));
        }

        let json_content =
            std::fs::read_to_string(&canonical).map_err(|e| RasterError::ImageAsset {
                path: canonical.display().to_string(),
                reason: format!("failed to read Lottie file: {e}"),
            })?;

        let animation =
            Animation::from_json_str(&json_content).map_err(|e| RasterError::ImageAsset {
                path: canonical.display().to_string(),
                reason: format!("failed to parse Lottie JSON: {e}"),
            })?;

        let width = animation.width as f32;
        let height = animation.height as f32;
        let total_frames = animation.out_point - animation.in_point;
        let frame_rate = animation.frame_rate;

        let entry = Arc::new(CachedLottie {
            animation,
            width,
            height,
            total_frames: total_frames.max(1.0),
            frame_rate: if frame_rate > 0.0 { frame_rate } else { 30.0 },
        });

        cache.insert(canonical, Arc::clone(&entry));
        Ok(entry)
    }

    /// Renders a specific frame of a Lottie animation at `target_w x target_h`.
    pub(crate) fn render(
        &self,
        src: &str,
        time_secs: f64,
        target_w: u32,
        target_h: u32,
        loop_behavior: crate::gif_cache::LoopBehavior,
    ) -> Result<Arc<RgbaImage>, RasterError> {
        let entry = self.get_or_load(src)?;
        let path = local_path(src)?;
        let canonical = path.canonicalize().unwrap_or(path);

        let total_duration_secs = (entry.total_frames / entry.frame_rate) as f64;
        let effective_time = match loop_behavior {
            crate::gif_cache::LoopBehavior::Loop => {
                if total_duration_secs > 0.0 {
                    (time_secs % total_duration_secs + total_duration_secs) % total_duration_secs
                } else {
                    0.0
                }
            }
            crate::gif_cache::LoopBehavior::Pause => time_secs.max(0.0).min(total_duration_secs),
            crate::gif_cache::LoopBehavior::Unmount => {
                if time_secs > total_duration_secs {
                    return Ok(Arc::new(RgbaImage::new(1, 1)));
                }
                time_secs.max(0.0)
            }
        };

        // Calculate frame index
        let frame_idx = (entry.animation.in_point + (effective_time as f32 * entry.frame_rate))
            .clamp(entry.animation.in_point, entry.animation.out_point);

        let quant_frame = (frame_idx.round() as u32).min(entry.animation.out_point as u32);
        let key = (canonical.clone(), quant_frame, target_w, target_h);

        {
            let frame_cache = self
                .rendered_frames
                .lock()
                .expect("rendered frames lock poisoned");
            if let Some(rendered) = frame_cache.get(&key) {
                return Ok(Arc::clone(rendered));
            }
        }

        let scale = if entry.width > 0.0 {
            (target_w as f32 / entry.width)
                .min(target_h as f32 / entry.height)
                .max(0.01)
        } else {
            1.0
        };

        let mut config = RenderConfig::default();
        config.scale = scale;

        let renderer = Renderer::default();
        let frame = renderer
            .render_frame(&entry.animation, frame_idx, config)
            .map_err(|e| RasterError::ImageAsset {
                path: canonical.display().to_string(),
                reason: format!("Lottie render error: {e}"),
            })?;

        let img =
            RgbaImage::from_raw(frame.width, frame.height, frame.pixels).ok_or_else(|| {
                RasterError::ImageAsset {
                    path: canonical.display().to_string(),
                    reason: "failed to construct RgbaImage from Lottie frame pixels".into(),
                }
            })?;
        let arc_img = Arc::new(img);

        let mut frame_cache = self
            .rendered_frames
            .lock()
            .expect("rendered frames lock poisoned");
        frame_cache.insert(key, Arc::clone(&arc_img));

        Ok(arc_img)
    }
}

fn local_path(src: &str) -> Result<PathBuf, RasterError> {
    let src = src.trim();
    if src.is_empty() {
        return Err(RasterError::ImageAsset {
            path: src.into(),
            reason: "source path is empty".into(),
        });
    }

    let path = if let Some(path) = src.strip_prefix("file://") {
        path
    } else if src.contains("://") || src.starts_with("data:") {
        return Err(RasterError::ImageAsset {
            path: src.into(),
            reason: "only local paths and file:// URIs are supported for Lottie".into(),
        });
    } else {
        src
    };

    Ok(PathBuf::from(path))
}
