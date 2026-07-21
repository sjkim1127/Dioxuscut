//! Thread-safe decoded raster image cache shared by frame renders.

use crate::backend::RasterError;
use image::RgbaImage;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(crate) struct ImageCache {
    decoded: Mutex<HashMap<PathBuf, Arc<RgbaImage>>>,
}

impl ImageCache {
    pub(crate) fn load(&self, src: &str) -> Result<Arc<RgbaImage>, RasterError> {
        let path = local_path(src)?;
        let canonical = path
            .canonicalize()
            .map_err(|error| RasterError::ImageAsset {
                path: path.display().to_string(),
                reason: error.to_string(),
            })?;

        // Keep cache misses serialized so parallel frame workers do not decode
        // the same asset repeatedly during the first rendered batch.
        let mut cache = self.decoded.lock().expect("image cache lock poisoned");
        if let Some(image) = cache.get(&canonical).cloned() {
            return Ok(image);
        }

        let decoded = image::open(&canonical)
            .map_err(|error| RasterError::ImageAsset {
                path: canonical.display().to_string(),
                reason: error.to_string(),
            })?
            .to_rgba8();
        let decoded = Arc::new(decoded);
        cache.insert(canonical, Arc::clone(&decoded));
        Ok(decoded)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.decoded
            .lock()
            .expect("image cache lock poisoned")
            .len()
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
            reason: "only local paths and file:// URIs are supported".into(),
        });
    } else {
        src
    };

    Ok(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_remote_sources_before_io() {
        let error = local_path("https://example.com/image.png").unwrap_err();
        assert!(error.to_string().contains("only local paths"));
    }
}
