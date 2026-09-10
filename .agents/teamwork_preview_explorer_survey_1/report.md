# Exploration & Architecture Survey Report: Requirement R1 (LRU Frame Cache & Long-Running Compositor Pipeline)

**Author:** Explorer 1 (Compositor & Cache Specialist)  
**Date:** 2026-08-19  
**Scope:** `crates/rasterizer`, `crates/renderer`, `crates/composition`, `crates/player`, `crates/cli`, `apps/studio`

---

## 1. Executive Summary

This survey analyzes the existing codebase in `/Users/sjkim1127/Dioxuscut` with a primary focus on **Requirement R1**:
1. **In-Memory Memory-Bounded LRU Frame Cache (`FrameCacheManager`)**: Eliminating redundant frame rasterization during timeline scrubbing, thumbnail virtualization, and frame playback.
2. **Long-Running Persistent Compositor Worker Daemon/Process**: Modeled after Remotion's Rust Compositor (`packages/compositor/rust`), keeping rendering backends, composition registries, and asset caches warm across frame requests.

### Baseline Health
- **Compilation:** `cargo check --locked --workspace --all-targets --all-features` passes cleanly with exit code 0.
- **Test Suite:** `cargo test --locked --workspace --all-features` passes 100% across all crates (144+ unit/integration tests passing).

---

## 2. Analysis of Existing Codebase & Caching Mechanisms

### 2.1 Current Rendering Architecture (`crates/rasterizer`)
- **Backends:**
  - `TinySkiaBackend` (`src/tiny_skia_backend.rs`): Pure-Rust CPU rasterizer using `tiny-skia` (2D paths, shapes, gradients, text, clipping, blending, filters). Renders a `Scene` into an `RgbaImage` via `RasterizerBackend::render_frame(&self, scene: &Scene, config: &FrameConfig) -> Result<RgbaImage, RasterError>`.
  - `WgpuBackend` (`src/wgpu_backend.rs`): GPU rasterizer feature.
- **Current Asset Caches:**
  - `ImageCache` (`src/image_cache.rs`): Thread-safe decoded raster image cache using `Mutex<HashMap<PathBuf, Arc<RgbaImage>>>`. Serializes misses. *Unbounded in memory*.
  - `VideoFrameCache` (`src/video_cache.rs`): Bounded decoded video frame cache with `MAX_CACHE_BYTES = 128MB` and `MAX_DECODER_SOURCES = 4`. Holds persistent FFmpeg decoder child processes reading raw RGBA frames over stdout pipe. Uses `VecDeque<(VideoFrameKey, Arc<RgbaImage>)>` with linear LRU updates.
  - `FontCache` (`src/font.rs`): Caches loaded TTF font data, glyph metrics, and shaped text layout using `ab_glyph` and `rustybuzz`.
- **Render Pipelines (`src/render.rs`):**
  - Sequential PNG export (`render_all_frames`).
  - Parallel Rayon export (`render_parallel`).
  - Streaming FFmpeg pipe export (`render_to_ffmpeg_pipe`): Bounded Rayon batch rendering directly written into FFmpeg stdin.
- **Architectural Gap:**
  There is currently **no composition-level rendered frame cache**. Every time a composition frame is requested (e.g. scrubbing in Studio, previewing in Player, or rendering non-sequential frames), the scene emitter re-evaluates the scene and `TinySkiaBackend` performs full rasterization from scratch.

### 2.2 Current Renderer Architecture (`crates/renderer`)
- **Existing Modules:**
  - `encode.rs`: Configures and executes `ffmpeg` CLI commands (`encode_mp4`, `encode_frames`) to stitch frame PNGs into MP4.
  - `render_frames.rs`: Defines `RenderConfig` and `RenderError`.
  - `server.rs`: Spawns static/command HTTP web server (`axum` / `ServeDir`) with health-check polling and dynamic port allocation.
- **Architectural Gap:**
  `crates/renderer` does not currently contain a persistent compositor daemon or worker process. It relies on external process calls (`ffmpeg`) or static servers.

### 2.3 Composition and Preview Integration
- `crates/composition`: `Composition` and `NativeComposition` traits produce a `Scene` per frame given input props and `NativeCompositionContext`.
- `crates/player`: `NativeCompositionPreview` converts `Scene` into SVG elements (`SceneView`).
- `apps/studio`: Desktop studio UI with properties panel, render queue, and timeline preview. Currently renders frames synchronously per state change without an IPC link to a compositor daemon.

---

## 3. Architectural Design for `FrameCacheManager`

### 3.1 Crate Location & Visibility
`FrameCacheManager` should be implemented in `crates/rasterizer/src/frame_cache.rs` and re-exported in `crates/rasterizer/src/lib.rs`.  
This ensures:
1. It is directly co-located with `RasterizerBackend`, `Scene`, `FrameConfig`, and `RgbaImage`.
2. Both `crates/renderer` (the compositor daemon), `crates/player` (preview), `crates/cli`, and `apps/studio` can leverage it without circular dependencies.

### 3.2 Key Data Structures

#### 3.2.1 `FrameCacheKey`
A composite hash key representing the exact parameters that uniquely determine a rendered frame's pixel output:
```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FrameCacheKey {
    pub composition_id: String,
    pub frame: u32,
    pub width: u32,
    pub height: u32,
    pub props_hash: u64,
}

impl FrameCacheKey {
    pub fn new(
        composition_id: impl Into<String>,
        frame: u32,
        width: u32,
        height: u32,
        props: &serde_json::Value,
    ) -> Self {
        use std::hash::{Hash, Hasher};
        use std::collections::hash_map::DefaultHasher;

        let mut hasher = DefaultHasher::new();
        props.to_string().hash(&mut hasher);
        let props_hash = hasher.finish();

        Self {
            composition_id: composition_id.into(),
            frame,
            width,
            height,
            props_hash,
        }
    }
}
```

#### 3.2.2 `CachedFrame`
```rust
#[derive(Clone, Debug)]
pub struct CachedFrame {
    pub image: Arc<RgbaImage>,
    pub byte_size: usize,
    pub access_seq: u64,
}
```

#### 3.2.3 `CacheMetrics`
```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CacheMetrics {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub entry_count: usize,
    pub current_bytes: usize,
    pub max_bytes: usize,
}

impl CacheMetrics {
    pub fn hit_ratio(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}
```

### 3.3 Thread-Safe Memory-Bounded LRU Cache Implementation

```rust
pub struct FrameCacheManager {
    inner: std::sync::RwLock<FrameCacheState>,
    max_bytes: usize,
    hits: std::sync::atomic::AtomicU64,
    misses: std::sync::atomic::AtomicU64,
    evictions: std::sync::atomic::AtomicU64,
}

struct FrameCacheState {
    entries: std::collections::HashMap<FrameCacheKey, CachedFrame>,
    lru_order: std::collections::VecDeque<FrameCacheKey>,
    current_bytes: usize,
    next_access_seq: u64,
}
```

### 3.4 Eviction Policy & Invariant Handling
1. **Strict Memory Bounding:**
   - Frame byte size is accurately calculated as `width * height * 4` (from `RgbaImage::as_raw().len()`).
   - Default capacity: 512 MB (`512 * 1024 * 1024` bytes), configurable via `FrameCacheConfig`.
2. **Eviction Algorithm:**
   - On `insert(key, frame)`:
     1. If single frame's `byte_size > max_bytes`, skip caching gracefully without polluting or evicting valid entries.
     2. If `key` is already present, subtract old entry's bytes from `current_bytes` and remove old key from `lru_order`.
     3. While `current_bytes + byte_size > max_bytes` and `lru_order` is not empty:
        - Pop least recently used key from front of `lru_order`.
        - Remove entry from `entries`, subtract its bytes from `current_bytes`, and increment `evictions` counter.
     4. Insert new `CachedFrame`, add `byte_size` to `current_bytes`, push `key` to back of `lru_order`.
3. **Concurrency:**
   - Fast concurrent queries: `get(&self, key: &FrameCacheKey)` uses `RwLock::read()`. If hit, records atomic hit counter and returns cloned `Arc<RgbaImage>`.
   - Thread-safe updates: LRU order is adjusted under write lock or via access sequence counters.
   - Batch invalidation:
     - `clear()`: Wipes all entries, resets `current_bytes` to 0.
     - `invalidate_composition(composition_id: &str)`: Removes entries matching the composition ID.
     - `invalidate_range(composition_id: &str, start_frame: u32, end_frame: u32)`.

---

## 4. Architectural Design for Long-Running Compositor Pipeline

### 4.1 Crate Location
The persistent daemon compositor worker pipeline should be located in `crates/renderer/src/compositor/` with submodules:
- `mod.rs`: Public types, `CompositorConfig`, `CompositorDaemon`, `CompositorHandle`.
- `worker.rs`: Background worker execution loop processing incoming frame commands.
- `protocol.rs` / `messages.rs`: Strongly typed request/response definitions.

### 4.2 Compositor Daemon Lifecycle
```text
┌─────────────────────────────────────────────────────────────┐
│                      Client / UI Process                    │
│    (Studio, Player, Timeline Virtualizer, CLI Client)       │
└──────────────────────────────┬──────────────────────────────┘
                               │ Request Frame (Nonce, Key)
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                 CompositorDaemon (crates/renderer)          │
│                                                             │
│   ┌───────────────────────────────────────────────────────┐ │
│   │                 FrameCacheManager (LRU)               │ │
│   │   [Key 1 -> Frame 0]  [Key 2 -> Frame 1] ...          │ │
│   └──────────────────────────┬────────────────────────────┘ │
│                              │ Cache Miss                   │
│                              ▼                              │
│   ┌───────────────────────────────────────────────────────┐ │
│   │               Composition Registry                    │ │
│   │    NativeComposition::render(frame, props, context)   │ │
│   └──────────────────────────┬────────────────────────────┘ │
│                              │ Scene IR                     │
│                              ▼                              │
│   ┌───────────────────────────────────────────────────────┐ │
│   │               TinySkiaBackend (CPU)                   │ │
│   │   - Persistent FontCache                              │ │
│   │   - Persistent ImageCache                             │ │
│   │   - Persistent VideoFrameCache (FFmpeg decoders)      │ │
│   └──────────────────────────┬────────────────────────────┘ │
│                              │ Arc<RgbaImage>               │
│                              ▼                              │
│   ┌───────────────────────────────────────────────────────┐ │
│   │ Store in LRU Cache & Stream Response to Client        │ │
│   └───────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

### 4.3 Request / Response Model
```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum CompositorRequest {
    RenderFrame {
        nonce: u64,
        composition_id: String,
        frame: u32,
        width: u32,
        height: u32,
        fps: f64,
        props: serde_json::Value,
    },
    RenderBatch {
        nonce: u64,
        composition_id: String,
        frames: Vec<u32>,
        width: u32,
        height: u32,
        fps: f64,
        props: serde_json::Value,
    },
    ClearCache {
        nonce: u64,
    },
    GetMetrics {
        nonce: u64,
    },
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum CompositorResponse {
    FrameReady {
        nonce: u64,
        frame: u32,
        width: u32,
        height: u32,
        pixels: Arc<RgbaImage>,
        cached: bool,
        render_time_ms: f64,
    },
    Metrics {
        nonce: u64,
        metrics: CacheMetrics,
    },
    Success {
        nonce: u64,
    },
    Error {
        nonce: u64,
        code: u32,
        message: String,
    },
}
```

### 4.4 Decoded Native Frame Memory Lifecycle
- **Zero-Copy Sharing:** `Arc<RgbaImage>` allows decoded/rendered frames to be shared between `FrameCacheManager`, Rayon render workers, background compositor tasks, and IPC encoders without memcpy.
- **Warm Asset Persistence:** `TinySkiaBackend` holds persistent `FontCache`, `ImageCache`, and `VideoFrameCache` (FFmpeg decoder child processes). By running continuously in `CompositorDaemon`, subsequent frame renders avoid spawning new `ffmpeg` instances or reloading fonts, maintaining high throughput (>60 FPS for typical timeline scrubbing).
- **Graceful Resource Shutdown:** On `Shutdown` request or `Drop`, `CompositorDaemon` calls `backend.shutdown_media()`, terminating all background FFmpeg decoding processes and freeing GPU/CPU buffers.

---

## 5. Dependency & Type Mapping

### 5.1 Crate Dependencies
- `crates/rasterizer/Cargo.toml`:
  - Needs `serde` (already present).
  - No new external crate dependencies required (uses `std::sync::{RwLock, atomic}`, `std::collections::{HashMap, VecDeque}`).
- `crates/renderer/Cargo.toml`:
  - Add workspace dependencies:
    ```toml
    dioxuscut-rasterizer = { workspace = true }
    dioxuscut-composition = { workspace = true }
    ```

### 5.2 Error Type Hierarchy
```rust
#[derive(Debug, thiserror::Error)]
pub enum CompositorError {
    #[error("Composition '{0}' not found in registry")]
    CompositionNotFound(String),
    #[error("Composition render failed: {0}")]
    Composition(#[from] dioxuscut_composition::CompositionError),
    #[error("Rasterization backend failed: {0}")]
    Raster(#[from] dioxuscut_rasterizer::RasterError),
    #[error("Channel communication error: {0}")]
    Channel(String),
    #[error("Compositor worker shut down")]
    WorkerShutdown,
}
```

---

## 6. Comprehensive Test Plan for R1

### 6.1 `FrameCacheManager` Unit & Stress Tests (`crates/rasterizer/tests/frame_cache_test.rs`)
1. **Basic Put and Get:** Insert a frame; assert `get(&key)` returns exact `Arc<RgbaImage>`; verify `metrics.hits == 1`, `metrics.misses == 0`.
2. **Cache Miss:** Query non-existent key; verify returns `None` and `metrics.misses == 1`.
3. **Memory Bounding & Eviction:**
   - Set `max_bytes = 1024 * 1024` (1MB).
   - Insert three 512KB frames (Keys A, B, C).
   - Verify Key A is evicted; Keys B and C remain; `current_bytes <= 1MB`; `metrics.evictions == 1`.
4. **True LRU Eviction Order:**
   - Insert Key A, Key B.
   - Access Key A (touch/get).
   - Insert Key C (triggers eviction).
   - Verify Key B was evicted, Key A and C remain.
5. **Key Replacement & Delta Bytes Accounting:**
   - Insert Key A (size S1); replace Key A with same key (size S2).
   - Verify `current_bytes` reflects S2, entry count is 1, no duplicate entries in LRU queue.
6. **Oversized Single Frame:**
   - Insert frame larger than `max_bytes`.
   - Verify cache does not crash, existing entries are preserved, and `current_bytes` stays valid.
7. **Thread-Safe Concurrent Read/Write Stress:**
   - Spawn 16 parallel threads querying and inserting 10,000 frames concurrently.
   - Verify no deadlocks, no panics, and metrics consistency (`current_bytes <= max_bytes`).
8. **Composition Invalidation:**
   - Insert frames for "Comp1" and "Comp2".
   - Invalidate "Comp1"; verify only "Comp2" frames remain; `current_bytes` adjusted accurately.

### 6.2 `CompositorDaemon` Integration Tests (`crates/renderer/tests/compositor_daemon_test.rs`)
1. **Daemon Spawning & Frame Request:**
   - Spawn daemon; send `RenderFrame` request for `HelloWorld` frame 0.
   - Verify returned frame dimensions, pixels, `cached == false`.
2. **Subsequent Request Cache Hit:**
   - Send identical `RenderFrame` request.
   - Verify `cached == true` and render time is `< 1ms`.
3. **Batch Request Processing:**
   - Send `RenderBatch` for frames 0..10.
   - Verify all 10 frames are produced correctly.
4. **Daemon Graceful Shutdown:**
   - Send `Shutdown`; verify clean join without hanging threads or orphaned FFmpeg processes.

---

## 7. Recommended Implementation Sequence

| Phase | Milestone | Crates Affected | Files to Create / Modify |
|---|---|---|---|
| **1** | `FrameCacheManager` & LRU Engine | `crates/rasterizer` | `crates/rasterizer/src/frame_cache.rs`, `crates/rasterizer/src/lib.rs`, `tests/frame_cache_test.rs` |
| **2** | Compositor Daemon & Worker Pipeline | `crates/renderer` | `crates/renderer/src/compositor/mod.rs`, `crates/renderer/src/compositor/worker.rs`, `crates/renderer/src/lib.rs`, `tests/compositor_daemon_test.rs` |
| **3** | Studio & Player Cache Integration | `crates/player`, `apps/studio` | Connect timeline scrub queries to `FrameCacheManager` |
