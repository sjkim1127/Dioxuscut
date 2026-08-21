# Project: Dioxuscut High-Performance Remotion-Equivalent Compositor Architecture

## Architecture
This project implements the high-performance Remotion-equivalent compositor architecture in Dioxuscut, comprising:
1. **LRU Frame Cache & Compositor Pipeline (`crates/rasterizer`, `crates/renderer`)**: Memory-bounded LRU frame caching (`FrameCacheManager`), persistent daemon compositor (`CompositorDaemon`), warm decoder reuse, thread-safe concurrent queries, cache hit metrics.
2. **Zero-Copy Binary IPC Protocol (`crates/renderer`, `crates/cli`, `apps/studio`)**: `remotion_buffer:<nonce>:<len>:<status>:<payload>` streaming packet codec, chunked streaming, asynchronous response correlation via monotonic nonces, raw RGBA pixel byte stream transport, CLI daemon subcommand.
3. **Dynamic Timeline Filmstrip & Waveform Virtualizer (`crates/player`, `apps/studio`)**: Viewport-aware slot partitioning (`calculate_timestamp_slots`), adaptive multi-tier dynamic ruler (`calculate_ruler_ticks`), background asynchronous thumbnail generation with LRU caching (`ThumbnailCache`), audio waveform peaks virtualization, and Studio UI integration.

```
+------------------------------------------------------------------------------------------------+
|                                      apps/studio (UI)                                          |
|  +------------------------------------------------------------------------------------------+  |
|  | TimelinePanel: Virtualized Filmstrip, Multi-tier Ruler, Waveform Peaks, Zoom/Scroll      |  |
|  +------------------------------------------------------------------------------------------+  |
|          |                                                            |                        |
|          v                                                            v                        |
|  crates/player (Virtualizer)                              crates/renderer (DaemonClient)       |
|  - calculate_timestamp_slots                              - BinaryPacket framing               |
|  - calculate_ruler_ticks                                  - Asynchronous Nonce Correlation     |
|  - ThumbnailCache & Generator                             - Zero-copy Bytes transport          |
+----------|------------------------------------------------------------|------------------------+
           |                                                            | stdio / socket / IPC
           |                                                            v
           |                                                crates/renderer (CompositorDaemon)   |
           |                                                - IPC Server (BinaryIpcCodec)        |
           |                                                - Warm Font & Video Decoders         |
           |                                                - CompositionRegistry                |
           |                                                            |
           +------------------------------------------------------------+
                                        |
                                        v
                            crates/rasterizer
                            - FrameCacheManager (Memory-bounded LRU)
                            - CacheMetrics (hits, misses, evictions, bytes)
                            - TinySkiaBackend (CPU Rasterizer)
```

## Feature Inventory
| # | Feature | Description | Milestone | Source |
|---|---------|-------------|-----------|--------|
| 1 | Memory-bounded LRU Frame Cache (`FrameCacheManager`) | Thread-safe in-memory LRU cache indexed by `FrameCacheKey` with byte-budget eviction | M1 | R1 |
| 2 | Cache Metrics & Hit Ratio Tracking | Atomic tracking of hits, misses, evictions, bytes, and hit ratio | M1 | R1 |
| 3 | Binary Packet Framing Protocol | `remotion_buffer:<nonce>:<len>:<status>:<payload>` framing format & parser | M2 | R2 |
| 4 | Streaming Codec & Chunked Decoder/Encoder | `tokio_util::codec` `BinaryIpcCodec`, `StreamDecoder`, `StreamEncoder`, `make_streamer` | M2 | R2 |
| 5 | Asynchronous Nonce Correlation & Error Signaling | Monotonic nonces with status code signaling and out-of-order response matching | M2 | R2 |
| 6 | Persistent Compositor Daemon Pipeline | Long-running daemon managing warm renderers, decoders, and composition registry | M3 | R3 (Compositor) |
| 7 | CLI Daemon Command & Stdio Isolation | `dioxuscut daemon` supporting stdio, socket, and port with strict stderr logging | M3 | R2 / R3 |
| 8 | Viewport-Aware Timestamp Slots Calculation | `calculate_timestamp_slots` mapping zoom, client width, scroll, overscan to slots | M4 | R3 |
| 9 | Adaptive Multi-tier Timeline Ruler | `calculate_ruler_ticks` computing adaptive tick intervals across 10,000x zoom range | M4 | R3 |
| 10 | Background Thumbnail Generator & LRU Cache | Async thumbnail generation with LRU caching and Dioxus reactive updates | M4 | R3 |
| 11 | Audio Waveform Peaks Virtualization | Min/max amplitude downsampling and SVG path generation | M4 | R3 |
| 12 | Studio Virtualized Filmstrip & Timeline UI | Dynamic `<FilmstripView>`, `<TimelinePanel>`, zoom controls, track lanes | M5 | R3 |
| 13 | Full E2E Integration, Verification & Stress Hardening | End-to-end integration tests, clippy, formatting, adversarial stress testing | M6 | Acceptance |

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| M1 | Frame Cache & Rasterizer Foundation | `crates/rasterizer`: `FrameCacheManager`, `FrameCacheKey`, `CacheMetrics`, tests | none | PLANNED |
| M2 | Zero-Copy Binary IPC Protocol | `crates/renderer`: `BinaryPacket`, `BinaryIpcCodec`, `StreamEncoder`, `StreamDecoder`, tests | none | PLANNED |
| M3 | Compositor Daemon & CLI Daemon | `crates/renderer`, `crates/cli`: `CompositorDaemon`, `DaemonClient`, `DaemonServer`, `dioxuscut daemon` | M1, M2 | PLANNED |
| M4 | Timeline Virtualizer & Filmstrip Engine | `crates/player`: `slots.rs`, `ruler.rs`, `thumbnail_cache.rs`, `waveform.rs`, `filmstrip.rs` | M1 | PLANNED |
| M5 | Studio Timeline & UI Integration | `apps/studio`: Timeline UI, zoom controls, filmstrip rendering, IPC preview | M3, M4 | PLANNED |
| M6 | E2E Testing, Adversarial Verification & Hardening | Opaque-box E2E suite (Tiers 1-4), adversarial stress tests (Tier 5), audit | M1-M5 | PLANNED |

## Interface Contracts

### `crates/rasterizer` (`FrameCacheManager`)
```rust
pub struct FrameCacheConfig {
    pub max_bytes: usize, // e.g. 512MB default
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FrameCacheKey {
    pub composition_id: String,
    pub frame: u64,
    pub width: u32,
    pub height: u32,
    pub props_hash: u64,
}

pub struct FrameCacheManager {
    // Thread-safe RwLock + Atomic metrics
    pub fn new(config: FrameCacheConfig) -> Self;
    pub fn get(&self, key: &FrameCacheKey) -> Option<Arc<image::RgbaImage>>;
    pub fn insert(&self, key: FrameCacheKey, frame: Arc<image::RgbaImage>);
    pub fn get_or_render<F>(&self, key: FrameCacheKey, render_fn: F) -> Result<Arc<image::RgbaImage>, RasterError>
        where F: FnOnce() -> Result<image::RgbaImage, RasterError>;
    pub fn metrics(&self) -> CacheMetrics;
    pub fn clear(&self);
}
```

### `crates/renderer` (`BinaryPacket` & IPC Protocol)
```rust
// Packet Header: "remotion_buffer:<nonce>:<len>:<status>:<payload>"
pub struct BinaryPacket {
    pub nonce: u64,
    pub status: u32, // 0 = OK, non-zero = error code
    pub payload: bytes::Bytes,
}

pub struct BinaryIpcCodec {
    pub max_payload_bytes: usize,
}

impl tokio_util::codec::Decoder for BinaryIpcCodec {
    type Item = BinaryPacket;
    type Error = IpcError;
    // decodes streaming chunks, supports resync
}

impl tokio_util::codec::Encoder<BinaryPacket> for BinaryIpcCodec {
    type Error = IpcError;
    // encodes to remotion_buffer format
}
```

### `crates/player` (`calculate_timestamp_slots` & Virtualizer)
```rust
pub struct TimelineViewport {
    pub duration_in_frames: u64,
    pub fps: f64,
    pub zoom_factor: f64,
    pub scroll_left_px: f64,
    pub client_width_px: f64,
    pub target_slot_width_px: f64,
    pub overscan_px: f64,
}

pub struct TimestampSlot {
    pub index: usize,
    pub start_frame: u64,
    pub end_frame: u64,
    pub start_time_secs: f64,
    pub x_position_px: f64,
    pub width_px: f64,
}

pub fn calculate_timestamp_slots(viewport: &TimelineViewport) -> VirtualizedTimelineSlots;
pub fn calculate_ruler_ticks(viewport: &TimelineViewport) -> Vec<RulerTick>;
```

## Code Layout
- `crates/rasterizer/src/frame_cache.rs`: In-memory LRU FrameCacheManager, metrics, and unit tests
- `crates/renderer/src/ipc/`: Binary IPC protocol, codec, streaming parser, client, server, and tests
- `crates/renderer/src/compositor/`: Compositor daemon, request router, warm pipeline manager, and tests
- `crates/cli/src/main.rs` & `commands/daemon.rs`: CLI daemon subcommand handler
- `crates/player/src/virtualizer/`: Virtualized slots, adaptive ruler, thumbnail cache, waveform, filmstrip
- `apps/studio/src/timeline/`: Modular studio timeline panel, track lanes, waveform view, zoom controls
- `tests/e2e/`: End-to-end integration and Remotion-parity test suites
