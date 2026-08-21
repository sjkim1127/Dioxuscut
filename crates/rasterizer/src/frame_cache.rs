//! In-memory LRU frame cache manager with memory-bounded eviction and thread-safe metrics.
//!
//! Provides fast, lock-free metrics and deterministic LRU frame caching
//! for timeline scrubbing, preview playback, and compositor rendering pipelines.

use crate::backend::RasterError;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Default maximum memory budget for frame cache: 512 MB.
pub const DEFAULT_MAX_CACHE_BYTES: usize = 512 * 1024 * 1024;

/// Configuration for the [`FrameCacheManager`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameCacheConfig {
    /// Maximum byte budget allocated for cached raw RGBA frame buffers.
    pub max_bytes: usize,
}

impl Default for FrameCacheConfig {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_CACHE_BYTES,
        }
    }
}

impl FrameCacheConfig {
    /// Create a new configuration with the specified maximum byte limit.
    pub fn new(max_bytes: usize) -> Self {
        Self { max_bytes }
    }

    /// Set maximum cache capacity in bytes.
    pub fn with_max_bytes(max_bytes: usize) -> Self {
        Self { max_bytes }
    }

    /// Set maximum cache capacity in megabytes (MB).
    pub fn with_megabytes(mb: usize) -> Self {
        Self {
            max_bytes: mb.saturating_mul(1024 * 1024),
        }
    }

    /// Set maximum cache capacity in gigabytes (GB).
    pub fn with_gigabytes(gb: usize) -> Self {
        Self {
            max_bytes: gb.saturating_mul(1024 * 1024 * 1024),
        }
    }
}

/// Composite cache key that uniquely identifies a rendered frame.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameCacheKey {
    /// Identifier for the composition being rendered.
    pub composition_id: String,
    /// Absolute frame index.
    pub frame: u64,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// 64-bit hash of the input parameters / props passed to the composition.
    pub props_hash: u64,
}

impl FrameCacheKey {
    /// Create a new cache key with explicit properties hash.
    pub fn new(
        composition_id: impl Into<String>,
        frame: u64,
        width: u32,
        height: u32,
        props_hash: u64,
    ) -> Self {
        Self {
            composition_id: composition_id.into(),
            frame,
            width,
            height,
            props_hash,
        }
    }

    /// Create a cache key with a 32-bit frame index.
    pub fn new_u32(
        composition_id: impl Into<String>,
        frame: u32,
        width: u32,
        height: u32,
        props_hash: u64,
    ) -> Self {
        Self::new(composition_id, frame as u64, width, height, props_hash)
    }

    /// Create a cache key by hashing a JSON properties value.
    pub fn from_props(
        composition_id: impl Into<String>,
        frame: u64,
        width: u32,
        height: u32,
        props: &serde_json::Value,
    ) -> Self {
        Self {
            composition_id: composition_id.into(),
            frame,
            width,
            height,
            props_hash: Self::hash_props(props),
        }
    }

    /// Compute a deterministic 64-bit hash from JSON value properties.
    pub fn hash_props(props: &serde_json::Value) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        let mut hasher = DefaultHasher::new();
        // Canonical string serialization to ensure stable hashing
        props.to_string().hash(&mut hasher);
        hasher.finish()
    }
}

/// A cached frame entry holding the pixel buffer, byte size, and access metadata.
#[derive(Debug, Clone)]
pub struct CachedFrame {
    /// Shared reference to the decoded/rendered raw RGBA frame buffer.
    pub image: Arc<RgbaImage>,
    /// Byte size in memory (`width * height * 4`).
    pub byte_size: usize,
    /// Monotonically increasing access sequence counter.
    pub access_seq: u64,
}

impl CachedFrame {
    /// Create a new cached frame wrapper.
    pub fn new(image: Arc<RgbaImage>, access_seq: u64) -> Self {
        let byte_size = image.as_raw().len();
        Self {
            image,
            byte_size,
            access_seq,
        }
    }
}

/// Real-time snapshot of frame cache performance and memory metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheMetrics {
    /// Total successful cache lookups.
    pub hits: u64,
    /// Total cache lookup misses.
    pub misses: u64,
    /// Total frames evicted due to memory pressure.
    pub evictions: u64,
    /// Current number of frames stored in cache.
    pub entry_count: usize,
    /// Current total bytes occupied by cached frame buffers.
    pub current_bytes: usize,
    /// Configured maximum memory budget in bytes.
    pub max_bytes: usize,
}

impl CacheMetrics {
    /// Compute cache hit ratio in the range `[0.0, 1.0]`.
    /// Returns `0.0` if total queries (`hits + misses`) is zero.
    pub fn hit_ratio(&self) -> f64 {
        let total = self.hits.saturating_add(self.misses);
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

/// Internal mutable state protected by `RwLock`.
#[derive(Default)]
struct FrameCacheState {
    entries: HashMap<FrameCacheKey, CachedFrame>,
    lru_order: VecDeque<FrameCacheKey>,
    current_bytes: usize,
}

/// Thread-safe in-memory LRU frame cache manager with memory-bounded eviction.
pub struct FrameCacheManager {
    inner: RwLock<FrameCacheState>,
    max_bytes: AtomicUsize,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    access_counter: AtomicU64,
}

impl Default for FrameCacheManager {
    fn default() -> Self {
        Self::new(FrameCacheConfig::default())
    }
}

impl FrameCacheManager {
    /// Create a new `FrameCacheManager` with the specified configuration.
    pub fn new(config: FrameCacheConfig) -> Self {
        Self {
            inner: RwLock::new(FrameCacheState::default()),
            max_bytes: AtomicUsize::new(config.max_bytes),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
            access_counter: AtomicU64::new(0),
        }
    }

    /// Helper to safely acquire a write guard, recovering from poisoned locks.
    fn write_state(&self) -> RwLockWriteGuard<'_, FrameCacheState> {
        self.inner.write().unwrap_or_else(|p| p.into_inner())
    }

    /// Helper to safely acquire a read guard, recovering from poisoned locks.
    fn read_state(&self) -> RwLockReadGuard<'_, FrameCacheState> {
        self.inner.read().unwrap_or_else(|p| p.into_inner())
    }

    /// Look up a frame by cache key.
    ///
    /// If present, marks the entry as most-recently-used (MRU), increments
    /// the atomic `hits` metric, and returns a cloned reference (`Arc<RgbaImage>`).
    /// If absent, increments the atomic `misses` metric and returns `None`.
    pub fn get(&self, key: &FrameCacheKey) -> Option<Arc<RgbaImage>> {
        let mut state = self.write_state();
        if let Some(entry) = state.entries.get_mut(key) {
            let seq = self.access_counter.fetch_add(1, Ordering::Relaxed);
            entry.access_seq = seq;
            let image = Arc::clone(&entry.image);

            // Move accessed key to the back of LRU queue (MRU position)
            if let Some(pos) = state.lru_order.iter().position(|k| k == key) {
                state.lru_order.remove(pos);
            }
            state.lru_order.push_back(key.clone());

            self.hits.fetch_add(1, Ordering::Relaxed);
            Some(image)
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Check whether a frame key is currently present in cache without updating LRU order or metrics.
    pub fn contains(&self, key: &FrameCacheKey) -> bool {
        let state = self.read_state();
        state.entries.contains_key(key)
    }

    /// Insert a rendered frame into the cache.
    ///
    /// If inserting this frame would cause `current_bytes` to exceed `max_bytes`,
    /// least recently used entries are evicted until sufficient space is available.
    /// If a single frame's size exceeds `max_bytes`, caching is gracefully skipped.
    pub fn insert(&self, key: FrameCacheKey, frame: Arc<RgbaImage>) {
        let byte_size = frame.as_raw().len();
        let max_bytes = self.max_bytes.load(Ordering::Relaxed);

        let mut state = self.write_state();

        // If the frame itself is larger than the entire cache budget, skip caching
        if byte_size > max_bytes {
            // If the key already existed, remove old entry and update bytes
            if let Some(old) = state.entries.remove(&key) {
                state.current_bytes = state.current_bytes.saturating_sub(old.byte_size);
                if let Some(pos) = state.lru_order.iter().position(|k| k == &key) {
                    state.lru_order.remove(pos);
                }
            }
            return;
        }

        // If key already exists in cache, remove old entry to update byte calculations
        if let Some(old) = state.entries.remove(&key) {
            state.current_bytes = state.current_bytes.saturating_sub(old.byte_size);
            if let Some(pos) = state.lru_order.iter().position(|k| k == &key) {
                state.lru_order.remove(pos);
            }
        }

        // Evict LRU entries until the new frame fits within the memory budget
        while state.current_bytes.saturating_add(byte_size) > max_bytes {
            if let Some(lru_key) = state.lru_order.pop_front() {
                if let Some(evicted) = state.entries.remove(&lru_key) {
                    state.current_bytes = state.current_bytes.saturating_sub(evicted.byte_size);
                    self.evictions.fetch_add(1, Ordering::Relaxed);
                }
            } else {
                break;
            }
        }

        // Store new entry at the MRU position (back of queue)
        let seq = self.access_counter.fetch_add(1, Ordering::Relaxed);
        let cached = CachedFrame::new(frame, seq);
        state.current_bytes = state.current_bytes.saturating_add(byte_size);
        state.entries.insert(key.clone(), cached);
        state.lru_order.push_back(key);
    }

    /// Convenience helper to insert an owned `RgbaImage`.
    pub fn insert_image(&self, key: FrameCacheKey, image: RgbaImage) -> Arc<RgbaImage> {
        let frame = Arc::new(image);
        self.insert(key, Arc::clone(&frame));
        frame
    }

    /// Retrieve a frame from the cache, or compute and insert it using `render_fn` on cache miss.
    pub fn get_or_render<F>(
        &self,
        key: FrameCacheKey,
        render_fn: F,
    ) -> Result<Arc<RgbaImage>, RasterError>
    where
        F: FnOnce() -> Result<RgbaImage, RasterError>,
    {
        if let Some(frame) = self.get(&key) {
            return Ok(frame);
        }

        let rendered = render_fn()?;
        let frame = Arc::new(rendered);
        self.insert(key, Arc::clone(&frame));
        Ok(frame)
    }

    /// Remove a specific frame from the cache.
    pub fn remove(&self, key: &FrameCacheKey) -> Option<Arc<RgbaImage>> {
        let mut state = self.write_state();
        if let Some(entry) = state.entries.remove(key) {
            state.current_bytes = state.current_bytes.saturating_sub(entry.byte_size);
            if let Some(pos) = state.lru_order.iter().position(|k| k == key) {
                state.lru_order.remove(pos);
            }
            Some(entry.image)
        } else {
            None
        }
    }

    /// Invalidate all cached frames for a given `composition_id`.
    /// Returns the number of evicted entries.
    pub fn invalidate_composition(&self, composition_id: &str) -> usize {
        let mut state = self.write_state();
        let to_remove: Vec<FrameCacheKey> = state
            .entries
            .keys()
            .filter(|k| k.composition_id == composition_id)
            .cloned()
            .collect();

        let count = to_remove.len();
        for key in to_remove {
            if let Some(entry) = state.entries.remove(&key) {
                state.current_bytes = state.current_bytes.saturating_sub(entry.byte_size);
                if let Some(pos) = state.lru_order.iter().position(|k| k == &key) {
                    state.lru_order.remove(pos);
                }
            }
        }
        count
    }

    /// Invalidate all cached frames for a given `composition_id` within a frame range `[start_frame, end_frame]`.
    /// Returns the number of evicted entries.
    pub fn invalidate_range(
        &self,
        composition_id: &str,
        start_frame: u64,
        end_frame: u64,
    ) -> usize {
        let mut state = self.write_state();
        let to_remove: Vec<FrameCacheKey> = state
            .entries
            .keys()
            .filter(|k| {
                k.composition_id == composition_id
                    && k.frame >= start_frame
                    && k.frame <= end_frame
            })
            .cloned()
            .collect();

        let count = to_remove.len();
        for key in to_remove {
            if let Some(entry) = state.entries.remove(&key) {
                state.current_bytes = state.current_bytes.saturating_sub(entry.byte_size);
                if let Some(pos) = state.lru_order.iter().position(|k| k == &key) {
                    state.lru_order.remove(pos);
                }
            }
        }
        count
    }

    /// Clear all cached frames and reset byte usage.
    /// Note: Lifetime metrics (hits, misses, evictions) are preserved.
    pub fn clear(&self) {
        let mut state = self.write_state();
        state.entries.clear();
        state.lru_order.clear();
        state.current_bytes = 0;
    }

    /// Reset all metrics counters (hits, misses, evictions) to zero.
    pub fn reset_metrics(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
    }

    /// Dynamically update the maximum cache capacity in bytes.
    /// If the new capacity is lower than `current_bytes`, least recently used entries are evicted immediately.
    pub fn set_max_bytes(&self, new_max_bytes: usize) {
        self.max_bytes.store(new_max_bytes, Ordering::Relaxed);
        let mut state = self.write_state();
        while state.current_bytes > new_max_bytes {
            if let Some(lru_key) = state.lru_order.pop_front() {
                if let Some(evicted) = state.entries.remove(&lru_key) {
                    state.current_bytes = state.current_bytes.saturating_sub(evicted.byte_size);
                    self.evictions.fetch_add(1, Ordering::Relaxed);
                }
            } else {
                break;
            }
        }
    }

    /// Get current metrics snapshot.
    pub fn metrics(&self) -> CacheMetrics {
        let (entry_count, current_bytes) = {
            let state = self.read_state();
            (state.entries.len(), state.current_bytes)
        };

        CacheMetrics {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            entry_count,
            current_bytes,
            max_bytes: self.max_bytes.load(Ordering::Relaxed),
        }
    }

    /// Current number of cached frames.
    pub fn entry_count(&self) -> usize {
        let state = self.read_state();
        state.entries.len()
    }

    /// Current memory usage in bytes.
    pub fn current_bytes(&self) -> usize {
        let state = self.read_state();
        state.current_bytes
    }

    /// Configured maximum memory budget in bytes.
    pub fn max_bytes(&self) -> usize {
        self.max_bytes.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_frame(width: u32, height: u32, fill_r: u8) -> Arc<RgbaImage> {
        let mut img = RgbaImage::new(width, height);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgba([fill_r, 0, 0, 255]);
        }
        Arc::new(img)
    }

    #[test]
    fn test_basic_put_and_get() {
        let cache = FrameCacheManager::new(FrameCacheConfig::with_megabytes(10));
        let key = FrameCacheKey::new("comp1", 0, 100, 100, 42);
        let frame = create_test_frame(100, 100, 255);

        assert!(cache.get(&key).is_none());
        assert_eq!(cache.metrics().misses, 1);
        assert_eq!(cache.metrics().hits, 0);

        cache.insert(key.clone(), Arc::clone(&frame));
        assert!(cache.contains(&key));

        let retrieved = cache.get(&key).expect("frame should be in cache");
        assert_eq!(retrieved.width(), 100);
        assert_eq!(retrieved.height(), 100);
        assert_eq!(retrieved.as_raw(), frame.as_raw());

        let metrics = cache.metrics();
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.misses, 1);
        assert_eq!(metrics.entry_count, 1);
        assert_eq!(metrics.current_bytes, 100 * 100 * 4);
        assert_eq!(metrics.hit_ratio(), 0.5);
    }

    #[test]
    fn test_lru_eviction_ordering() {
        // Frame size for 10x10 is 400 bytes.
        // Set capacity to hold exactly 2 frames (800 bytes).
        let cache = FrameCacheManager::new(FrameCacheConfig::with_max_bytes(800));

        let key_a = FrameCacheKey::new("comp", 1, 10, 10, 0);
        let key_b = FrameCacheKey::new("comp", 2, 10, 10, 0);
        let key_c = FrameCacheKey::new("comp", 3, 10, 10, 0);

        let frame_a = create_test_frame(10, 10, 10);
        let frame_b = create_test_frame(10, 10, 20);
        let frame_c = create_test_frame(10, 10, 30);

        cache.insert(key_a.clone(), frame_a);
        cache.insert(key_b.clone(), frame_b);

        assert_eq!(cache.entry_count(), 2);
        assert_eq!(cache.current_bytes(), 800);
        assert_eq!(cache.metrics().evictions, 0);

        // Access Key A so it becomes most recently used (MRU)
        assert!(cache.get(&key_a).is_some());

        // Insert Key C -> should evict Key B (the least recently used)
        cache.insert(key_c.clone(), frame_c);

        assert_eq!(cache.entry_count(), 2);
        assert_eq!(cache.current_bytes(), 800);
        assert_eq!(cache.metrics().evictions, 1);

        // Key A and Key C should exist, Key B should have been evicted
        assert!(cache.contains(&key_a));
        assert!(cache.contains(&key_c));
        assert!(!cache.contains(&key_b));
    }

    #[test]
    fn test_key_replacement_and_byte_accounting() {
        let cache = FrameCacheManager::new(FrameCacheConfig::with_megabytes(10));
        let key = FrameCacheKey::new("comp", 1, 10, 10, 0);

        let frame_small = create_test_frame(10, 10, 1); // 400 bytes
        let frame_large = create_test_frame(20, 20, 2); // 1600 bytes

        cache.insert(key.clone(), frame_small);
        assert_eq!(cache.entry_count(), 1);
        assert_eq!(cache.current_bytes(), 400);

        // Replace with larger frame under same key
        cache.insert(key.clone(), frame_large);
        assert_eq!(cache.entry_count(), 1);
        assert_eq!(cache.current_bytes(), 1600);

        let retrieved = cache.get(&key).unwrap();
        assert_eq!(retrieved.width(), 20);
        assert_eq!(retrieved.height(), 20);
    }

    #[test]
    fn test_oversized_single_frame_graceful_handling() {
        // Cache max bytes is 500 bytes. Frame size is 1000 bytes.
        let cache = FrameCacheManager::new(FrameCacheConfig::with_max_bytes(500));
        let key_valid = FrameCacheKey::new("comp", 1, 10, 10, 0); // 400 bytes
        let frame_valid = create_test_frame(10, 10, 1);

        cache.insert(key_valid.clone(), frame_valid);
        assert_eq!(cache.entry_count(), 1);
        assert_eq!(cache.current_bytes(), 400);

        let key_oversized = FrameCacheKey::new("comp", 2, 20, 20, 0); // 1600 bytes
        let frame_oversized = create_test_frame(20, 20, 2);

        // Should not panic or evict existing valid entries
        cache.insert(key_oversized.clone(), frame_oversized);
        assert!(!cache.contains(&key_oversized));
        assert!(cache.contains(&key_valid));
        assert_eq!(cache.entry_count(), 1);
        assert_eq!(cache.current_bytes(), 400);
    }

    #[test]
    fn test_get_or_render() {
        let cache = FrameCacheManager::new(FrameCacheConfig::with_megabytes(1));
        let key = FrameCacheKey::new("comp", 5, 10, 10, 123);

        let mut render_count = 0;
        let res = cache.get_or_render(key.clone(), || {
            render_count += 1;
            Ok((*create_test_frame(10, 10, 99)).clone())
        });
        assert!(res.is_ok());
        assert_eq!(render_count, 1);
        assert_eq!(cache.metrics().misses, 1);

        // Second call should hit cache without calling render_fn
        let res2 = cache.get_or_render(key.clone(), || {
            panic!("render_fn should not be called on cache hit");
        });
        assert!(res2.is_ok());
        assert_eq!(cache.metrics().hits, 1);
    }

    #[test]
    fn test_invalidation() {
        let cache = FrameCacheManager::new(FrameCacheConfig::with_megabytes(10));
        let k1 = FrameCacheKey::new("compA", 1, 10, 10, 0);
        let k2 = FrameCacheKey::new("compA", 2, 10, 10, 0);
        let k3 = FrameCacheKey::new("compB", 1, 10, 10, 0);

        cache.insert(k1.clone(), create_test_frame(10, 10, 1));
        cache.insert(k2.clone(), create_test_frame(10, 10, 2));
        cache.insert(k3.clone(), create_test_frame(10, 10, 3));

        assert_eq!(cache.entry_count(), 3);
        assert_eq!(cache.current_bytes(), 1200);

        // Invalidate range for compA frame 2..2
        let removed = cache.invalidate_range("compA", 2, 2);
        assert_eq!(removed, 1);
        assert_eq!(cache.entry_count(), 2);
        assert!(cache.contains(&k1));
        assert!(!cache.contains(&k2));
        assert!(cache.contains(&k3));

        // Invalidate all for compA
        let removed2 = cache.invalidate_composition("compA");
        assert_eq!(removed2, 1);
        assert_eq!(cache.entry_count(), 1);
        assert!(!cache.contains(&k1));
        assert!(cache.contains(&k3));
        assert_eq!(cache.current_bytes(), 400);

        // Clear all
        cache.clear();
        assert_eq!(cache.entry_count(), 0);
        assert_eq!(cache.current_bytes(), 0);
    }

    #[test]
    fn test_dynamic_set_max_bytes_eviction() {
        let cache = FrameCacheManager::new(FrameCacheConfig::with_max_bytes(1200));
        let k1 = FrameCacheKey::new("c", 1, 10, 10, 0);
        let k2 = FrameCacheKey::new("c", 2, 10, 10, 0);
        let k3 = FrameCacheKey::new("c", 3, 10, 10, 0);

        cache.insert(k1.clone(), create_test_frame(10, 10, 1));
        cache.insert(k2.clone(), create_test_frame(10, 10, 2));
        cache.insert(k3.clone(), create_test_frame(10, 10, 3));

        assert_eq!(cache.entry_count(), 3);
        assert_eq!(cache.current_bytes(), 1200);

        // Shrink capacity down to 400 bytes (1 frame)
        cache.set_max_bytes(400);
        assert_eq!(cache.entry_count(), 1);
        assert_eq!(cache.current_bytes(), 400);
        assert_eq!(cache.metrics().evictions, 2);
        assert!(cache.contains(&k3)); // k3 was MRU
    }

    #[test]
    fn test_props_hashing() {
        let props1 = serde_json::json!({"text": "Hello", "count": 1});
        let props2 = serde_json::json!({"text": "World", "count": 2});
        let props3 = serde_json::json!({"text": "Hello", "count": 1});

        let key1 = FrameCacheKey::from_props("c", 0, 1920, 1080, &props1);
        let key2 = FrameCacheKey::from_props("c", 0, 1920, 1080, &props2);
        let key3 = FrameCacheKey::from_props("c", 0, 1920, 1080, &props3);

        assert_eq!(key1, key3);
        assert_ne!(key1, key2);
    }
}
