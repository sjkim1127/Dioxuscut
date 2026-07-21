//! Bounded cache for video frames decoded through FFmpeg.

use crate::backend::RasterError;
use image::RgbaImage;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

const MAX_CACHE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct VideoFrameKey {
    path: PathBuf,
    timestamp_micros: u64,
}

#[derive(Default)]
struct CacheState {
    frames: VecDeque<(VideoFrameKey, Arc<RgbaImage>)>,
    bytes: usize,
}

#[derive(Default)]
pub(crate) struct VideoFrameCache {
    state: Mutex<CacheState>,
}

impl VideoFrameCache {
    pub(crate) fn load(&self, src: &str, time: f64) -> Result<Arc<RgbaImage>, RasterError> {
        if !time.is_finite() || time < 0.0 {
            return Err(media_error(
                src,
                "video time must be finite and non-negative",
            ));
        }
        let path = canonical_local_path(src)?;
        let timestamp_micros = (time * 1_000_000.0).round().clamp(0.0, u64::MAX as f64) as u64;
        let key = VideoFrameKey {
            path: path.clone(),
            timestamp_micros,
        };

        if let Some(frame) = self.cached(&key) {
            return Ok(frame);
        }

        let frame = Arc::new(decode_frame(&path, timestamp_micros)?);
        self.insert(key, Arc::clone(&frame));
        Ok(frame)
    }

    fn cached(&self, key: &VideoFrameKey) -> Option<Arc<RgbaImage>> {
        let mut state = self.state.lock().expect("video cache lock poisoned");
        let index = state
            .frames
            .iter()
            .position(|(candidate, _)| candidate == key)?;
        let entry = state.frames.remove(index)?;
        let frame = Arc::clone(&entry.1);
        state.frames.push_back(entry);
        Some(frame)
    }

    fn insert(&self, key: VideoFrameKey, frame: Arc<RgbaImage>) {
        let frame_bytes = frame.as_raw().len();
        if frame_bytes > MAX_CACHE_BYTES {
            return;
        }

        let mut state = self.state.lock().expect("video cache lock poisoned");
        if let Some(index) = state
            .frames
            .iter()
            .position(|(candidate, _)| candidate == &key)
        {
            if let Some((_, duplicate)) = state.frames.remove(index) {
                state.bytes = state.bytes.saturating_sub(duplicate.as_raw().len());
            }
        }
        while state.bytes.saturating_add(frame_bytes) > MAX_CACHE_BYTES {
            let Some((_, evicted)) = state.frames.pop_front() else {
                break;
            };
            state.bytes = state.bytes.saturating_sub(evicted.as_raw().len());
        }
        state.bytes += frame_bytes;
        state.frames.push_back((key, frame));
    }

    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        self.state.lock().expect("video cache lock poisoned").bytes
    }
}

pub(crate) fn canonical_local_path(src: &str) -> Result<PathBuf, RasterError> {
    let src = src.trim();
    if src.is_empty() {
        return Err(media_error(src, "source path is empty"));
    }
    let path = if let Some(path) = src.strip_prefix("file://") {
        path
    } else if src.contains("://") || src.starts_with("data:") {
        return Err(media_error(
            src,
            "only local paths and file:// URIs are supported",
        ));
    } else {
        src
    };
    Path::new(path)
        .canonicalize()
        .map_err(|error| media_error(src, error.to_string()))
}

fn decode_frame(path: &Path, timestamp_micros: u64) -> Result<RgbaImage, RasterError> {
    let timestamp = timestamp_micros as f64 / 1_000_000.0;
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-ss"])
        .arg(format!("{timestamp:.6}"))
        .arg("-i")
        .arg(path)
        .args([
            "-map",
            "0:v:0",
            "-frames:v",
            "1",
            "-an",
            "-f",
            "image2pipe",
            "-c:v",
            "png",
            "pipe:1",
        ])
        .output()
        .map_err(|error| {
            media_error(
                path.display().to_string(),
                format!("failed to run FFmpeg: {error}"),
            )
        })?;

    if !output.status.success() {
        return Err(media_error(
            path.display().to_string(),
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    if output.stdout.is_empty() {
        return Err(media_error(
            path.display().to_string(),
            format!("no video frame exists at {timestamp:.6} seconds"),
        ));
    }

    image::load_from_memory(&output.stdout)
        .map(|image| image.to_rgba8())
        .map_err(|error| media_error(path.display().to_string(), error.to_string()))
}

fn media_error(path: impl Into<String>, reason: impl Into<String>) -> RasterError {
    RasterError::MediaAsset {
        path: path.into(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_remote_video_sources() {
        let error = canonical_local_path("https://example.com/video.mp4").unwrap_err();
        assert!(error.to_string().contains("only local paths"));
    }

    #[test]
    fn replacing_a_cached_key_does_not_double_count_bytes() {
        let cache = VideoFrameCache::default();
        let key = VideoFrameKey {
            path: PathBuf::from("clip.mp4"),
            timestamp_micros: 0,
        };
        let frame = Arc::new(RgbaImage::new(2, 2));
        cache.insert(key.clone(), Arc::clone(&frame));
        cache.insert(key, frame);

        let state = cache.state.lock().unwrap();
        assert_eq!(state.frames.len(), 1);
        assert_eq!(state.bytes, 16);
    }
}
