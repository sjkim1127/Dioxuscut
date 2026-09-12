//! Thread-safe decoded raster image cache shared by frame renders.

use crate::backend::RasterError;
use base64::Engine as _;
use image::RgbaImage;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(crate) struct ImageCache {
    decoded: Mutex<ImageCacheState>,
}

const MAX_DATA_URI_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Default)]
struct ImageCacheState {
    images: HashMap<String, Arc<RgbaImage>>,
    lru: VecDeque<String>,
    bytes: usize,
}

impl ImageCacheState {
    fn get(&mut self, key: &str) -> Option<Arc<RgbaImage>> {
        let image = self.images.get(key).cloned()?;
        self.lru.retain(|entry| entry != key);
        self.lru.push_back(key.to_string());
        Some(image)
    }

    fn insert(&mut self, key: String, image: Arc<RgbaImage>) -> Arc<RgbaImage> {
        if let Some(existing) = self.get(&key) {
            return existing;
        }
        let size = image.as_raw().len();
        self.images.insert(key.clone(), Arc::clone(&image));
        self.lru.push_back(key);
        self.bytes = self.bytes.saturating_add(size);
        while self.bytes > MAX_CACHE_BYTES {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            if let Some(evicted) = self.images.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(evicted.as_raw().len());
            }
        }
        image
    }
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
        let trimmed = src.trim();
        if let Some(data) = trimmed.strip_prefix("data:") {
            return self.load_data_uri(trimmed, data);
        }

        let canonical = policy.validate_path(trimmed).map_err(|e| match e {
            RasterError::MediaAsset { path, reason } => RasterError::ImageAsset { path, reason },
            other => other,
        })?;

        let key = canonical.display().to_string();
        if let Some(image) = self
            .decoded
            .lock()
            .expect("image cache lock poisoned")
            .get(&key)
        {
            return Ok(image);
        }

        // Decode outside the global cache lock so different assets can load in
        // parallel. A second lookup below prevents replacing an image decoded
        // concurrently for the same key.
        let decoded = image::open(&canonical)
            .map_err(|error| RasterError::ImageAsset {
                path: canonical.display().to_string(),
                reason: error.to_string(),
            })?
            .to_rgba8();
        let decoded = Arc::new(decoded);
        let mut cache = self.decoded.lock().expect("image cache lock poisoned");
        Ok(cache.insert(key, decoded))
    }

    fn load_data_uri(&self, src: &str, data: &str) -> Result<Arc<RgbaImage>, RasterError> {
        if let Some(image) = self
            .decoded
            .lock()
            .expect("image cache lock poisoned")
            .get(src)
        {
            return Ok(image);
        }
        let Some((metadata, encoded)) = data.split_once(',') else {
            return Err(RasterError::ImageAsset {
                path: src.to_string(),
                reason: "data URI is missing its payload".into(),
            });
        };
        let mime = metadata
            .split(';')
            .next()
            .filter(|value| matches!(*value, "image/png" | "image/jpeg" | "image/webp"))
            .ok_or_else(|| RasterError::ImageAsset {
                path: src.to_string(),
                reason: "only image/png, image/jpeg, and image/webp data URIs are supported".into(),
            })?;
        if !metadata.split(';').any(|part| part == "base64") {
            return Err(RasterError::ImageAsset {
                path: src.to_string(),
                reason: "only base64-encoded image data URIs are supported".into(),
            });
        }
        if encoded.len() > (MAX_DATA_URI_BYTES * 4 / 3) + 4 {
            return Err(RasterError::ImageAsset {
                path: src.to_string(),
                reason: format!("data URI exceeds the {MAX_DATA_URI_BYTES} byte limit"),
            });
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| RasterError::ImageAsset {
                path: src.to_string(),
                reason: format!("invalid base64 image data: {error}"),
            })?;
        if bytes.len() > MAX_DATA_URI_BYTES {
            return Err(RasterError::ImageAsset {
                path: src.to_string(),
                reason: format!("data URI exceeds the {MAX_DATA_URI_BYTES} byte limit"),
            });
        }
        let decoded = image::load_from_memory_with_format(
            &bytes,
            match mime {
                "image/png" => image::ImageFormat::Png,
                "image/jpeg" => image::ImageFormat::Jpeg,
                _ => image::ImageFormat::WebP,
            },
        )
        .map_err(|error| RasterError::ImageAsset {
            path: src.to_string(),
            reason: error.to_string(),
        })?
        .to_rgba8();
        let decoded = Arc::new(decoded);
        let mut cache = self.decoded.lock().expect("image cache lock poisoned");
        Ok(cache.insert(src.to_string(), decoded))
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.decoded
            .lock()
            .expect("image cache lock poisoned")
            .images
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

    #[test]
    fn decodes_and_caches_png_data_uri() {
        let cache = ImageCache::default();
        let source = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let image = cache.load(source).unwrap();
        assert_eq!((image.width(), image.height()), (1, 1));
        assert_eq!(cache.len(), 1);
        let cached = cache.load(source).unwrap();
        assert!(std::sync::Arc::ptr_eq(&image, &cached));
    }
}
