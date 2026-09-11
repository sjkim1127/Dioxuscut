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
    #[allow(dead_code)]
    pub(crate) fn load(&self, src: &str) -> Result<Arc<RgbaImage>, RasterError> {
        self.load_with_policy(src, &crate::security::MediaSecurityPolicy::default())
    }

    pub(crate) fn load_with_policy(
        &self,
        src: &str,
        policy: &crate::security::MediaSecurityPolicy,
    ) -> Result<Arc<RgbaImage>, RasterError> {
        let canonical = policy.validate_path(src).map_err(|e| match e {
            RasterError::MediaAsset { path, reason } => RasterError::ImageAsset { path, reason },
            other => other,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_remote_sources_before_io() {
        let cache = ImageCache::default();
        let error = cache.load("https://example.com/image.png").unwrap_err();
        assert!(error.to_string().contains("only local paths"));
    }
}
