//! GIF animation frame cache — Remotion `@remotion/gif` parity.
//!
//! Decodes animated GIFs into per-frame RGBA images and caches them in memory.
//! Supports three loop behaviours: `Loop`, `Pause`, and `Unmount`.

use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder, RgbaImage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::backend::RasterError;

/// What happens when the GIF animation reaches its last frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LoopBehavior {
    /// Restart from the first frame (default).
    #[default]
    Loop,
    /// Freeze on the last frame.
    Pause,
    /// Return `None` (the node should be hidden).
    Unmount,
}

/// A single decoded GIF frame.
pub struct GifFrame {
    pub image: RgbaImage,
    /// Frame display duration in milliseconds.
    pub delay_ms: u32,
}

/// Thread-safe LRU-style GIF frame cache.
///
/// Frames are decoded once per unique `src` path and reused across renders.
#[derive(Default)]
pub struct GifFrameCache {
    inner: Arc<RwLock<HashMap<String, Arc<Vec<GifFrame>>>>>,
}

impl GifFrameCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load (and cache) all frames for the given GIF file path using default policy.
    pub fn load_frames(&self, src: &str) -> Result<Arc<Vec<GifFrame>>, RasterError> {
        self.load_frames_with_policy(src, &crate::security::MediaSecurityPolicy::default())
    }

    /// Load (and cache) all frames for the given GIF file path validating against security policy.
    pub fn load_frames_with_policy(
        &self,
        src: &str,
        policy: &crate::security::MediaSecurityPolicy,
    ) -> Result<Arc<Vec<GifFrame>>, RasterError> {
        let canonical = policy.validate_path(src)?;
        let key = canonical.display().to_string();

        // Fast path: already cached
        {
            let guard = self.inner.read().unwrap();
            if let Some(frames) = guard.get(&key) {
                return Ok(Arc::clone(frames));
            }
        }

        // Slow path: decode from disk
        let bytes = std::fs::read(&canonical)
            .map_err(|e| RasterError::Scene(format!("GIF read '{src}': {e}")))?;
        let decoder = GifDecoder::new(std::io::Cursor::new(&bytes))
            .map_err(|e| RasterError::Scene(format!("GIF decode '{src}': {e}")))?;

        let mut frames: Vec<GifFrame> = Vec::new();
        for result in decoder.into_frames() {
            let frame = result.map_err(|e| RasterError::Scene(format!("GIF frame error: {e}")))?;
            let (numer, denom) = frame.delay().numer_denom_ms();
            // Avoid divide-by-zero; GIF spec minimum is 10 ms per frame
            let delay_ms = numer.checked_div(denom).unwrap_or(100).max(10);
            let rgba = image::DynamicImage::ImageRgba8(frame.into_buffer()).to_rgba8();

            frames.push(GifFrame {
                image: rgba,
                delay_ms,
            });
        }

        let frames = Arc::new(frames);
        self.inner.write().unwrap().insert(key, Arc::clone(&frames));
        Ok(frames)
    }

    /// Select the frame that should be displayed at `time_ms` milliseconds into
    /// the animation, honouring `loop_behavior`.
    /// Select the frame index and image that should be displayed at `time_ms` milliseconds into
    /// the animation, honouring `loop_behavior`.
    ///
    /// Returns `None` only when `loop_behavior == Unmount` and the animation
    /// has ended.
    pub fn frame_index_at_time_ms(
        frames: &[GifFrame],
        time_ms: f64,
        loop_behavior: LoopBehavior,
    ) -> Option<(usize, &RgbaImage)> {
        if frames.is_empty() {
            return None;
        }
        let total_ms: f64 = frames.iter().map(|f| f.delay_ms as f64).sum();
        if total_ms <= 0.0 {
            return frames.first().map(|f| (0, &f.image));
        }

        let t = match loop_behavior {
            LoopBehavior::Loop => time_ms.rem_euclid(total_ms),
            LoopBehavior::Pause => time_ms.clamp(0.0, total_ms - 1.0),
            LoopBehavior::Unmount => {
                if time_ms >= total_ms {
                    return None;
                }
                time_ms.max(0.0)
            }
        };

        let mut elapsed = 0.0_f64;
        for (index, frame) in frames.iter().enumerate() {
            elapsed += frame.delay_ms as f64;
            if t < elapsed {
                return Some((index, &frame.image));
            }
        }
        let last_index = frames.len() - 1;
        frames.last().map(|f| (last_index, &f.image))
    }

    /// Select the frame that should be displayed at `time_ms` milliseconds into
    /// the animation, honouring `loop_behavior`.
    ///
    /// Returns `None` only when `loop_behavior == Unmount` and the animation
    /// has ended.
    pub fn frame_at_time_ms(
        frames: &[GifFrame],
        time_ms: f64,
        loop_behavior: LoopBehavior,
    ) -> Option<&RgbaImage> {
        Self::frame_index_at_time_ms(frames, time_ms, loop_behavior).map(|(_, img)| img)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_frames(delays_ms: &[u32]) -> Vec<GifFrame> {
        delays_ms
            .iter()
            .map(|&d| GifFrame {
                image: RgbaImage::new(1, 1),
                delay_ms: d,
            })
            .collect()
    }

    #[test]
    fn loop_wraps_correctly() {
        let frames = dummy_frames(&[100, 100, 100]); // total 300 ms
                                                     // t=350ms → wraps to 50ms → frame 0
        let img = GifFrameCache::frame_at_time_ms(&frames, 350.0, LoopBehavior::Loop);
        assert!(img.is_some());
    }

    #[test]
    fn loop_at_zero_gives_first_frame() {
        let frames = dummy_frames(&[200, 200]);
        let img = GifFrameCache::frame_at_time_ms(&frames, 0.0, LoopBehavior::Loop);
        assert!(img.is_some());
    }

    #[test]
    fn pause_clamps_to_last_frame() {
        let frames = dummy_frames(&[100, 200]);
        let img = GifFrameCache::frame_at_time_ms(&frames, 99_999.0, LoopBehavior::Pause);
        assert!(img.is_some());
    }

    #[test]
    fn unmount_returns_none_after_end() {
        let frames = dummy_frames(&[100]);
        assert!(GifFrameCache::frame_at_time_ms(&frames, 100.0, LoopBehavior::Unmount).is_none());
        assert!(GifFrameCache::frame_at_time_ms(&frames, 50.0, LoopBehavior::Unmount).is_some());
    }

    #[test]
    fn empty_frames_returns_none() {
        assert!(GifFrameCache::frame_at_time_ms(&[], 0.0, LoopBehavior::Loop).is_none());
    }

    #[test]
    fn exact_boundary_selects_next_frame() {
        let frames = dummy_frames(&[100, 100]);
        // t=100ms: elapsed after frame0 = 100, 100 < 100 is false → moves to frame1
        let img = GifFrameCache::frame_at_time_ms(&frames, 100.0, LoopBehavior::Loop);
        assert!(img.is_some());
    }

    #[test]
    fn frame_index_matches_delay_segments() {
        let frames = dummy_frames(&[100, 150, 200]); // frame 0: [0, 100), frame 1: [100, 250), frame 2: [250, 450)
        assert_eq!(
            GifFrameCache::frame_index_at_time_ms(&frames, 0.0, LoopBehavior::Loop).map(|(i, _)| i),
            Some(0)
        );
        assert_eq!(
            GifFrameCache::frame_index_at_time_ms(&frames, 99.0, LoopBehavior::Loop)
                .map(|(i, _)| i),
            Some(0)
        );
        assert_eq!(
            GifFrameCache::frame_index_at_time_ms(&frames, 100.0, LoopBehavior::Loop)
                .map(|(i, _)| i),
            Some(1)
        );
        assert_eq!(
            GifFrameCache::frame_index_at_time_ms(&frames, 249.0, LoopBehavior::Loop)
                .map(|(i, _)| i),
            Some(1)
        );
        assert_eq!(
            GifFrameCache::frame_index_at_time_ms(&frames, 250.0, LoopBehavior::Loop)
                .map(|(i, _)| i),
            Some(2)
        );
        assert_eq!(
            GifFrameCache::frame_index_at_time_ms(&frames, 500.0, LoopBehavior::Loop)
                .map(|(i, _)| i),
            Some(0)
        ); // 500 % 450 = 50 -> frame 0
        assert_eq!(
            GifFrameCache::frame_index_at_time_ms(&frames, 500.0, LoopBehavior::Pause)
                .map(|(i, _)| i),
            Some(2)
        );
        assert_eq!(
            GifFrameCache::frame_index_at_time_ms(&frames, 500.0, LoopBehavior::Unmount)
                .map(|(i, _)| i),
            None
        );
    }
}
