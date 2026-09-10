# Requirement R3: Dynamic Timeline Filmstrip & Waveform Virtualizer Survey & Architectural Specification

## 1. Executive Summary

This report delivers the complete architectural survey and technical specification for **Requirement R3 (Dynamic Timeline Filmstrip & Waveform Virtualizer)** in the Dioxuscut workspace (`crates/player`, `apps/studio`).

In professional desktop and web video editing environments (such as Remotion Studio, Premiere Pro, and Final Cut Pro), the timeline is the primary interactive workspace. A non-virtualized timeline rendering hundreds of DOM elements or un-cached thumbnail images causes extreme memory bloat, dropped UI frames during zooming, and unresponsiveness during timeline scrubbing.

Requirement R3 provides a high-performance, hardware-friendly timeline virtualization engine comprising:
1. **Viewport-Aware Timestamp Slot Calculation (`calculate_timestamp_slots`)**: An ultra-fast, deterministic mathematical slot partitioner that maps timeline zoom level, client viewport width, and scroll offset into exact visible frame intervals and layout coordinates with zero allocation overhead.
2. **Adaptive Ruler Ticks (`calculate_ruler_ticks`)**: Dynamic time ruler generation providing clean division intervals (frames, sub-seconds, seconds, minutes) across a 10,000x continuous zoom range.
3. **Background Asynchronous Frame Thumbnail Generation & LRU Caching (`ThumbnailCache`)**: A thread-safe, memory-bounded LRU cache with an asynchronous worker queue that generates thumbnails without stalling UI interaction or audio playback.
4. **Filmstrip Canvas & Track Layer Rendering**: Virtualized UI components for `<Sequence>`, `<Video>`, and `<Audio>` tracks in Dioxuscut Studio, rendering only the visible window with smooth CSS transform positioning.
5. **Audio Waveform Virtualization (`WaveformPeaks`)**: Fast amplitude downsampling and SVG path generation for audio tracks.

---

## 2. Existing Codebase Survey & Gap Analysis

### 2.1 Current State of `crates/player` and `apps/studio`

#### `crates/player`
- **`src/player.rs`**:
  - Implements the core `<Player>` component wrapping a composition.
  - Manages playback timing via wall-clock elapsed time in `tokio::time::sleep` loops (`frame_at_elapsed`).
  - Provides `PlayerPlaybackState` through Dioxus context.
  - **Gap**: Only handles single-frame composition display; lacks timeline track models, slot calculation, thumbnail caching, and filmstrip components.
- **`src/controls.rs`**:
  - Implements basic play/pause button and HTML `<input type="range">` scrubber.
  - **Gap**: Single linear progress bar without zoom, pan, filmstrip, or track virtualization.
- **`src/native_preview.rs`**:
  - Implements `NativeCompositionPreview` and `SceneView` converting native `Scene` graphs to SVG elements.
  - Includes media synchronization script for HTML `<video>` and `<audio>` DOM elements.

#### `apps/studio`
- **`src/main.rs`**:
  - Provides the desktop editor application shell using `dioxus-desktop`.
  - Grid layout: TopBar (48px), LeftPanel (compositions), Center (Player preview), RightPanel (properties + render queue), Bottom (TimelinePanel, 200px).
  - **`TimelinePanel` (lines 636–728)**:
    - Currently renders a static 1-second ruler (`for sec in 0..=(duration_frames / fps)`) and 3 hardcoded static scene bars ("Scene 1: Title", "Scene 2: Body", "Scene 3: Outro").
    - **Gap**: No zoom controls, no horizontal pan/scroll virtualization, no filmstrip images, no video thumbnails, no audio waveforms, and no connection to composition sequences.

### 2.2 Requirement Matrix & Architectural Gap Analysis

| Feature | Existing Implementation | Requirement for R3 |
|---|---|---|
| **Timestamp Slot Calculation** | None | Viewport-aware `calculate_timestamp_slots` with zoom factor, scroll offset, and client width. |
| **Ruler Subdivision** | Static 1s ticks | Multi-tier adaptive ticks (`calculate_ruler_ticks`): frames, 0.5s, 1s, 5s, 10s, 30s, 1m, 5m. |
| **Thumbnail Generation** | Synchronous export only | Asynchronous background worker generating scaled thumbnail frames. |
| **Thumbnail LRU Cache** | None (only video frame cache in rasterizer) | Dedicated memory-bounded LRU `ThumbnailCache` in `crates/player`. |
| **Filmstrip Track View** | Static colored divs | Virtualized repeating thumbnail strip for Sequence and Video layers. |
| **Audio Waveform View** | None | Downsampled min/max amplitude peaks rendered as SVG waveform bars. |
| **Scrubbing Interactivity** | Native slider input | Smooth drag-and-scrub playhead with timecode HUD and frame-accurate seeking. |

---

## 3. Viewport-Aware Timestamp Slot Calculation (`calculate_timestamp_slots`)

### 3.1 Mathematical Model & Coordinate System

The timeline coordinate system is parameterized by:
- `timeline_duration_frames: u32`: Total composition length in frames.
- `fps: f64`: Frames per second (e.g. 30.0, 60.0, 24.0, 29.97).
- `zoom_factor: f64`: Continuous zoom multiplier where `1.0` is the default scale (e.g. 100 pixels per second).
- `scroll_left_px: f64`: Horizontal scroll offset of the viewport in pixels.
- `client_width_px: f64`: Visible client width of the timeline container in pixels.
- `target_slot_width_px: f64`: Desired width per thumbnail slot in pixels (default: 80.0 px).

```text
Timeline Coordinates (Pixels):
0px ───────────────────────────────────────────────────────────► Total Width (px)
       │◄─── scroll_left ───►│◄── client_width ──►│
       ├─────────────────────┼────────────────────┼─────────────────────────┤
       │ (Scrolled Off Left) │  VISIBLE VIEWPORT  │ (Scrolled Off Right)    │
       │  [Overscan Buffer]  │  [Active Slots]    │  [Overscan Buffer]      │
```

#### Derived Quantities:
1. **Base Pixel Density**:
   $$\text{base\_px\_per\_sec} = 100.0\text{ px/s}$$
   $$\text{pixels\_per\_second} = \text{base\_px\_per\_sec} \times \text{zoom\_factor}$$
   $$\text{pixels\_per\_frame} = \frac{\text{pixels\_per\_second}}{\text{fps}}$$
2. **Total Timeline Width**:
   $$\text{total\_width\_px} = \text{timeline\_duration\_frames} \times \text{pixels\_per\_frame}$$
3. **Slot Width & Frame Interval**:
   - Each thumbnail slot occupies a nominal pixel width $W_{\text{slot}} = \text{target\_slot\_width\_px}$.
   - Number of frames per slot:
     $$\text{frames\_per\_slot} = \max\left(1, \left\lfloor \frac{W_{\text{slot}}}{\text{pixels\_per\_frame}} \right\rfloor \right)$$
   - Actual slot pixel width:
     $$W_{\text{actual}} = \text{frames\_per\_slot} \times \text{pixels\_per\_frame}$$
4. **Visible Slot Index Range with Overscan**:
   - Overscan buffer (e.g. 1 viewport width on each side) prevents blank gaps during high-speed touchpad panning.
   $$\text{visible\_min\_x} = \max(0.0, \text{scroll\_left\_px} - \text{overscan\_px})$$
   $$\text{visible\_max\_x} = \min(\text{total\_width\_px}, \text{scroll\_left\_px} + \text{client\_width\_px} + \text{overscan\_px})$$
   $$\text{start\_slot\_idx} = \left\lfloor \frac{\text{visible\_min\_x}}{W_{\text{actual}}} \right\rfloor$$
   $$\text{end\_slot\_idx} = \min\left(\text{total\_slots}, \left\lceil \frac{\text{visible\_max\_x}}{W_{\text{actual}}} \right\rceil\right)$$

### 3.2 Struct Definitions & API Specification

```rust
use serde::{Deserialize, Serialize};

/// Viewport configuration defining visible timeline geometry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimelineViewport {
    /// Total duration of the composition or track in frames.
    pub duration_in_frames: u32,
    /// Playback rate in frames per second.
    pub fps: f64,
    /// Timeline horizontal zoom multiplier (1.0 = standard 100px/sec).
    pub zoom_factor: f64,
    /// Current horizontal scroll offset in pixels (0.0 = timeline start).
    pub scroll_left_px: f64,
    /// Width of the visible timeline client container in pixels.
    pub client_width_px: f64,
    /// Target nominal pixel width for each thumbnail slot (e.g. 80.0 px).
    pub target_slot_width_px: f64,
    /// Additional buffer in pixels on left and right for seamless scrolling.
    pub overscan_px: f64,
}

impl Default for TimelineViewport {
    fn default() -> Self {
        Self {
            duration_in_frames: 300,
            fps: 30.0,
            zoom_factor: 1.0,
            scroll_left_px: 0.0,
            client_width_px: 1200.0,
            target_slot_width_px: 80.0,
            overscan_px: 200.0,
        }
    }
}

/// A computed thumbnail or filmstrip slot on the virtualized timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimestampSlot {
    /// 0-indexed sequential slot number.
    pub slot_index: usize,
    /// Representative frame index to sample for thumbnail rasterization.
    pub frame: u32,
    /// Exact timeline time in seconds corresponding to this frame.
    pub timestamp_seconds: f64,
    /// Left offset in pixels from the timeline origin (x = 0).
    pub x_offset_px: f64,
    /// Computed pixel width of this slot.
    pub width_px: f64,
    /// Duration of this slot in frames.
    pub duration_in_frames: u32,
    /// Whether this slot lies inside the active visible viewport (excluding overscan).
    pub is_in_viewport: bool,
}

/// Output of the viewport virtualization calculation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VirtualizedTimelineSlots {
    /// Computed slots to render in the virtual DOM / canvas.
    pub slots: Vec<TimestampSlot>,
    /// Total virtual scrollable width of the timeline in pixels.
    pub total_width_px: f64,
    /// Total number of slots across the entire composition.
    pub total_slots: usize,
    /// Pixels per frame at the current zoom level.
    pub pixels_per_frame: f64,
    /// Pixels per second at the current zoom level.
    pub pixels_per_second: f64,
    /// Number of frames represented by each standard slot.
    pub frames_per_slot: u32,
}
```

### 3.3 Implementation of `calculate_timestamp_slots`

```rust
/// Calculates the set of visible and buffered timestamp slots for timeline filmstrip virtualization.
pub fn calculate_timestamp_slots(viewport: &TimelineViewport) -> VirtualizedTimelineSlots {
    if viewport.duration_in_frames == 0 {
        return VirtualizedTimelineSlots {
            slots: Vec::new(),
            total_width_px: 0.0,
            total_slots: 0,
            pixels_per_frame: 0.0,
            pixels_per_second: 0.0,
            frames_per_slot: 0,
        };
    }

    let safe_fps = if viewport.fps.is_finite() && viewport.fps > 0.0 {
        viewport.fps
    } else {
        30.0
    };

    let safe_zoom = if viewport.zoom_factor.is_finite() && viewport.zoom_factor > 0.0 {
        viewport.zoom_factor.clamp(0.001, 1000.0)
    } else {
        1.0
    };

    let safe_client_width = viewport.client_width_px.max(0.0);
    let safe_scroll_left = viewport.scroll_left_px.max(0.0);
    let safe_target_slot_width = viewport.target_slot_width_px.clamp(16.0, 1920.0);
    let safe_overscan = viewport.overscan_px.max(0.0);

    const BASE_PX_PER_SEC: f64 = 100.0;
    let pixels_per_second = BASE_PX_PER_SEC * safe_zoom;
    let pixels_per_frame = pixels_per_second / safe_fps;
    let total_width_px = viewport.duration_in_frames as f64 * pixels_per_frame;

    // Number of frames per slot
    let frames_per_slot = ((safe_target_slot_width / pixels_per_frame).floor() as u32).max(1);
    let slot_width_px = frames_per_slot as f64 * pixels_per_frame;

    let total_slots = ((viewport.duration_in_frames as f64) / (frames_per_slot as f64)).ceil() as usize;

    if total_slots == 0 || slot_width_px <= 0.0 {
        return VirtualizedTimelineSlots {
            slots: Vec::new(),
            total_width_px,
            total_slots: 0,
            pixels_per_frame,
            pixels_per_second,
            frames_per_slot,
        };
    }

    // Viewport bounds
    let viewport_min_x = safe_scroll_left;
    let viewport_max_x = safe_scroll_left + safe_client_width;

    // Buffer range including overscan
    let buffer_min_x = (viewport_min_x - safe_overscan).max(0.0);
    let buffer_max_x = (viewport_max_x + safe_overscan).min(total_width_px);

    let start_slot = ((buffer_min_x / slot_width_px).floor() as usize).min(total_slots.saturating_sub(1));
    let end_slot = ((buffer_max_x / slot_width_px).ceil() as usize).min(total_slots);

    let mut slots = Vec::with_capacity(end_slot.saturating_sub(start_slot) + 1);

    for idx in start_slot..end_slot {
        let frame_start = (idx as u32) * frames_per_slot;
        if frame_start >= viewport.duration_in_frames {
            break;
        }

        let remaining_frames = viewport.duration_in_frames - frame_start;
        let slot_frames = remaining_frames.min(frames_per_slot);
        let actual_width_px = slot_frames as f64 * pixels_per_frame;
        let x_offset_px = frame_start as f64 * pixels_per_frame;

        let slot_min_x = x_offset_px;
        let slot_max_x = x_offset_px + actual_width_px;
        let is_in_viewport = slot_max_x >= viewport_min_x && slot_min_x <= viewport_max_x;

        // Sample frame at mid-slot or start of slot
        let sample_frame = frame_start + (slot_frames / 2);
        let sample_frame = sample_frame.min(viewport.duration_in_frames - 1);
        let timestamp_seconds = sample_frame as f64 / safe_fps;

        slots.push(TimestampSlot {
            slot_index: idx,
            frame: sample_frame,
            timestamp_seconds,
            x_offset_px,
            width_px: actual_width_px,
            duration_in_frames: slot_frames,
            is_in_viewport,
        });
    }

    VirtualizedTimelineSlots {
        slots,
        total_width_px,
        total_slots,
        pixels_per_frame,
        pixels_per_second,
        frames_per_slot,
    }
}
```

### 3.4 Adaptive Timeline Ruler Ticks (`calculate_ruler_ticks`)

A professional timeline requires clean, readable time marks that dynamically adjust their granularity (e.g. 1 frame, 5 frames, 10 frames, 1 sec, 5 sec, 10 sec, 30 sec, 1 min, 5 min) depending on the zoom level so ticks never overlap.

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RulerTick {
    pub frame: u32,
    pub timestamp_seconds: f64,
    pub x_offset_px: f64,
    pub is_major: bool,
    pub label: Option<String>,
}

/// Computes ruler tick intervals based on zoom factor and client viewport.
pub fn calculate_ruler_ticks(
    viewport: &TimelineViewport,
    min_tick_spacing_px: f64,
) -> Vec<RulerTick> {
    if viewport.duration_in_frames == 0 {
        return Vec::new();
    }

    let safe_fps = if viewport.fps.is_finite() && viewport.fps > 0.0 { viewport.fps } else { 30.0 };
    let safe_zoom = if viewport.zoom_factor.is_finite() && viewport.zoom_factor > 0.0 {
        viewport.zoom_factor.clamp(0.001, 1000.0)
    } else { 1.0 };

    let px_per_sec = 100.0 * safe_zoom;
    let px_per_frame = px_per_sec / safe_fps;
    let total_width_px = viewport.duration_in_frames as f64 * px_per_frame;

    let safe_min_spacing = min_tick_spacing_px.max(30.0);

    // Standard tick step candidates in seconds
    let step_candidates_sec: &[f64] = &[
        1.0 / safe_fps,       // 1 frame
        5.0 / safe_fps,       // 5 frames
        10.0 / safe_fps,      // 10 frames
        0.5,                  // 0.5s
        1.0,                  // 1.0s
        2.0,                  // 2.0s
        5.0,                  // 5.0s
        10.0,                 // 10.0s
        30.0,                 // 30.0s
        60.0,                 // 1 min
        300.0,                // 5 min
        600.0,                // 10 min
    ];

    let chosen_step_sec = step_candidates_sec
        .iter()
        .copied()
        .find(|&step| step * px_per_sec >= safe_min_spacing)
        .unwrap_or(600.0);

    let step_frames = ((chosen_step_sec * safe_fps).round() as u32).max(1);

    let scroll_min_x = (viewport.scroll_left_px - viewport.overscan_px).max(0.0);
    let scroll_max_x = (viewport.scroll_left_px + viewport.client_width_px + viewport.overscan_px).min(total_width_px);

    let start_frame = ((scroll_min_x / px_per_frame).floor() as u32).min(viewport.duration_in_frames);
    let end_frame = ((scroll_max_x / px_per_frame).ceil() as u32).min(viewport.duration_in_frames);

    let aligned_start = (start_frame / step_frames) * step_frames;

    let mut ticks = Vec::new();
    let mut curr_frame = aligned_start;

    while curr_frame <= end_frame {
        if curr_frame <= viewport.duration_in_frames {
            let x_offset_px = curr_frame as f64 * px_per_frame;
            let time_secs = curr_frame as f64 / safe_fps;
            let is_major = (curr_frame % (step_frames * 5)) == 0 || curr_frame == 0;
            let label = format_timecode(time_secs, safe_fps, chosen_step_sec);

            ticks.push(RulerTick {
                frame: curr_frame,
                timestamp_seconds: time_secs,
                x_offset_px,
                is_major,
                label: Some(label),
            });
        }
        curr_frame = curr_frame.saturating_add(step_frames);
        if step_frames == 0 { break; }
    }

    ticks
}

fn format_timecode(time_secs: f64, fps: f64, step_sec: f64) -> String {
    let total_secs = time_secs.floor() as u64;
    let mins = total_secs / 60;
    let secs = total_secs % 60;
    if step_sec < 1.0 {
        let frame_in_sec = ((time_secs - total_secs as f64) * fps).round() as u64;
        format!("{mins:02}:{secs:02}:{frame_in_sec:02}")
    } else {
        format!("{mins:02}:{secs:02}")
    }
}
```

---

## 4. Background Asynchronous Thumbnail Generation & LRU Caching

### 4.1 Architecture Diagram

```text
  ┌─────────────────────────────────────────────────────────────┐
  │                 Dioxus Studio UI Thread                     │
  │                                                             │
  │  • Renders virtualized Filmstrip Slots                      │
  │  • Queries ThumbnailCache for visible slots                 │
  │  • If Miss: sends AsyncThumbnailRequest to Channel          │
  └──────────────────────────────┬──────────────────────────────┘
                                 │ mpsc::channel(128)
                                 ▼
  ┌─────────────────────────────────────────────────────────────┐
  │            Background Thumbnail Worker Service              │
  │                                                             │
  │  • Prioritizes visible slots over overscan slots            │
  │  • Drops stale requests if user scrubs past them            │
  │  • Renders still frame via TinySkia / Daemon Compositor IPC │
  │  • Scales down to thumbnail dimensions (e.g. 160x90)        │
  │  • Encodes to PNG / RGBA Data URL                           │
  │  • Stores into ThumbnailCache                               │
  │  • Signals UI update via Dioxus reactive signal             │
  └──────────────────────────────┬──────────────────────────────┘
                                 │
                                 ▼
  ┌─────────────────────────────────────────────────────────────┐
  │             Memory-Bounded LRU ThumbnailCache               │
  │                                                             │
  │  Key: (composition_id, frame, width, height)                │
  │  Value: Arc<ThumbnailData> (raw RGBA or Base64 Data URL)    │
  │  Capacity: 256 items (or 64 MB max memory)                  │
  └─────────────────────────────────────────────────────────────┘
```

### 4.2 `ThumbnailCache` Implementation

```rust
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ThumbnailKey {
    pub composition_id: String,
    pub frame: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct ThumbnailItem {
    pub key: ThumbnailKey,
    pub rgba_bytes: Arc<Vec<u8>>,
    pub data_url: Option<String>,
    pub byte_size: usize,
}

#[derive(Default)]
struct CacheInner {
    map: HashMap<ThumbnailKey, Arc<ThumbnailItem>>,
    order: VecDeque<ThumbnailKey>,
    current_bytes: usize,
    hits: u64,
    misses: u64,
}

#[derive(Clone)]
pub struct ThumbnailCache {
    inner: Arc<Mutex<CacheInner>>,
    max_bytes: usize,
    max_entries: usize,
}

impl Default for ThumbnailCache {
    fn default() -> Self {
        Self::new(64 * 1024 * 1024, 512) // 64 MB or 512 thumbnails
    }
}

impl ThumbnailCache {
    pub fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(CacheInner::default())),
            max_bytes,
            max_entries,
        }
    }

    pub fn get(&self, key: &ThumbnailKey) -> Option<Arc<ThumbnailItem>> {
        let mut inner = self.inner.lock().ok()?;
        if let Some(item) = inner.map.get(key).cloned() {
            inner.hits += 1;
            // Move to back of LRU queue
            if let Some(pos) = inner.order.iter().position(|k| k == key) {
                inner.order.remove(pos);
                inner.order.push_back(key.clone());
            }
            Some(item)
        } else {
            inner.misses += 1;
            None
        }
    }

    pub fn insert(&self, item: ThumbnailItem) {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };

        let key = item.key.clone();
        let bytes = item.byte_size;

        if bytes > self.max_bytes {
            return;
        }

        // If replacing existing key, deduct old size
        if let Some(old) = inner.map.remove(&key) {
            inner.current_bytes = inner.current_bytes.saturating_sub(old.byte_size);
            if let Some(pos) = inner.order.iter().position(|k| k == &key) {
                inner.order.remove(pos);
            }
        }

        // Evict LRU entries if capacity exceeded
        while (inner.current_bytes + bytes > self.max_bytes || inner.map.len() >= self.max_entries)
            && !inner.order.is_empty()
        {
            if let Some(evicted_key) = inner.order.pop_front() {
                if let Some(evicted) = inner.map.remove(&evicted_key) {
                    inner.current_bytes = inner.current_bytes.saturating_sub(evicted.byte_size);
                }
            }
        }

        inner.current_bytes += bytes;
        inner.order.push_back(key.clone());
        inner.map.insert(key, Arc::new(item));
    }

    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.map.clear();
            inner.order.clear();
            inner.current_bytes = 0;
        }
    }

    pub fn stats(&self) -> (usize, usize, u64, u64) {
        if let Ok(inner) = self.inner.lock() {
            (inner.map.len(), inner.current_bytes, inner.hits, inner.misses)
        } else {
            (0, 0, 0, 0)
        }
    }
}
```

### 4.3 Background Worker & Generation Logic

```rust
use dioxuscut_composition::{CompositionRegistry, NativeCompositionContext};
use dioxuscut_rasterizer::{FrameConfig, TinySkiaBackend};
use image::imageops::FilterType;

pub struct ThumbnailGenerator {
    registry: Arc<CompositionRegistry>,
    backend: Arc<TinySkiaBackend>,
}

impl ThumbnailGenerator {
    pub fn new(registry: Arc<CompositionRegistry>) -> Self {
        Self {
            registry,
            backend: Arc::new(TinySkiaBackend::new()),
        }
    }

    pub fn generate(
        &self,
        composition_id: &str,
        frame: u32,
        thumb_width: u32,
        thumb_height: u32,
        fps: f64,
        props: &serde_json::Value,
    ) -> Result<ThumbnailItem, String> {
        let composition = self.registry.get(composition_id).map_err(|e| e.to_string())?;

        let context = NativeCompositionContext {
            width: thumb_width * 2,  // Render at 2x resolution for retina crispness
            height: thumb_height * 2,
            fps,
            duration_in_frames: 1000,
        };

        let prepared = composition.prepare(props, context).map_err(|e| e.to_string())?;
        let scene = prepared.render(frame).map_err(|e| e.to_string())?;

        let frame_cfg = FrameConfig::new(context.width, context.height, frame, fps);
        let full_img = self.backend.render_frame(&scene, &frame_cfg).map_err(|e| e.to_string())?;

        // Resize down to target thumbnail dimensions
        let resized = image::imageops::resize(&full_img, thumb_width, thumb_height, FilterType::Triangle);
        let rgba_raw = resized.into_raw();
        let byte_size = rgba_raw.len();

        let key = ThumbnailKey {
            composition_id: composition_id.to_string(),
            frame,
            width: thumb_width,
            height: thumb_height,
        };

        // Convert to data URL for seamless web/desktop HTML img rendering
        let mut png_bytes = std::io::Cursor::new(Vec::new());
        let _ = image::DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(thumb_width, thumb_height, rgba_raw.clone()).unwrap()
        ).write_to(&mut png_bytes, image::ImageFormat::Png);

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(png_bytes.into_inner());
        let data_url = format!("data:image/png;base64,{b64}");

        Ok(ThumbnailItem {
            key,
            rgba_bytes: Arc::new(rgba_raw),
            data_url: Some(data_url),
            byte_size,
        })
    }
}
```

---

## 5. Filmstrip Canvas & Track Layer Rendering in Dioxuscut Studio

### 5.1 Dioxus Studio Timeline Component Architecture

```text
TimelinePanel (Dioxus Component)
 ├── TimelineHeaderControls (Zoom In/Out buttons, Zoom Slider, Timecode Display, Snap Toggle)
 ├── TimelineScrollContainer (Handles scroll event, syncs scroll_left signal)
 │    ├── TimelineRulerView (Renders RulerTicks, Playhead Marker, Cursor Line)
 │    └── TimelineTrackList
 │         ├── VideoTrackLane
 │         │    └── SequenceClipItem (from_frame .. to_frame)
 │         │         └── FilmstripView (Repeats ThumbnailSlot cards virtualized)
 │         ├── AudioTrackLane
 │         │    └── AudioClipItem
 │         │         └── WaveformView (Renders downsampled amplitude SVG path)
 │         └── CaptionTrackLane
 └── ScrubberPlayhead (Absolute positioned needle synced to current_frame)
```

### 5.2 Filmstrip UI Component Specification

```rust
use dioxus::prelude::*;
use crate::virtualizer::{calculate_timestamp_slots, TimelineViewport, ThumbnailCache, ThumbnailKey};

#[derive(Props, Clone, PartialEq)]
pub struct FilmstripViewProps {
    pub composition_id: String,
    pub track_from_frame: u32,
    pub track_duration_frames: u32,
    pub viewport: TimelineViewport,
    pub cache: ThumbnailCache,
    pub thumb_height: u32,
}

#[component]
pub fn FilmstripView(props: FilmstripViewProps) -> Element {
    let virtualized = calculate_timestamp_slots(&props.viewport);
    let cache = props.cache.clone();
    let comp_id = props.composition_id.clone();
    let thumb_height = props.thumb_height;

    rsx! {
        div {
            class: "dioxuscut-filmstrip-container",
            style: "
                display: flex;
                position: absolute;
                top: 0; bottom: 0; left: 0; right: 0;
                overflow: hidden;
                pointer-events: none;
            ",
            for slot in virtualized.slots {
                {
                    let key = ThumbnailKey {
                        composition_id: comp_id.clone(),
                        frame: slot.frame,
                        width: (slot.width_px.round() as u32).max(16),
                        height: thumb_height,
                    };
                    let cached_thumb = cache.get(&key);

                    rsx! {
                        div {
                            key: "slot-{slot.slot_index}",
                            style: "
                                position: absolute;
                                left: {slot.x_offset_px}px;
                                width: {slot.width_px}px;
                                height: 100%;
                                box-sizing: border-box;
                                border-right: 1px solid rgba(255,255,255,0.08);
                                overflow: hidden;
                                background: #161622;
                            ",
                            if let Some(thumb) = cached_thumb {
                                if let Some(ref data_url) = thumb.data_url {
                                    img {
                                        src: "{data_url}",
                                        style: "width: 100%; height: 100%; object-fit: cover; display: block;",
                                    }
                                }
                            } else {
                                // Skeleton placeholder while thumbnail renders in background
                                div {
                                    style: "
                                        width: 100%; height: 100%;
                                        display: flex; align-items: center; justify-content: center;
                                        background: rgba(255,255,255,0.03);
                                        color: rgba(255,255,255,0.25);
                                        font-size: 9px; font-family: monospace;
                                    ",
                                    "{slot.frame}f"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
```

---

## 6. Audio Waveform Representation & Virtualization

### 6.1 Peak Downsampling Algorithm

Audio waveforms require fast visualization without parsing millions of PCM samples on every render tick. We compute downsampled **min/max amplitude pairs** per pixel slot.

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaveformPeaks {
    /// Pairs of [min, max] amplitude values normalized in [-1.0, 1.0].
    pub peaks: Vec<(f32, f32)>,
    /// Number of audio PCM samples per peak bucket.
    pub samples_per_pixel: usize,
    /// Total duration in seconds.
    pub duration_seconds: f64,
}

impl WaveformPeaks {
    /// Extract min/max peaks from raw PCM audio channel samples.
    pub fn from_pcm_samples(samples: &[f32], target_peak_count: usize) -> Self {
        if samples.is_empty() || target_peak_count == 0 {
            return Self {
                peaks: Vec::new(),
                samples_per_pixel: 1,
                duration_seconds: 0.0,
            };
        }

        let samples_per_bucket = (samples.len() / target_peak_count).max(1);
        let mut peaks = Vec::with_capacity(target_peak_count);

        for chunk in samples.chunks(samples_per_bucket) {
            let mut min_val = 0.0f32;
            let mut max_val = 0.0f32;
            for &s in chunk {
                if s < min_val { min_val = s; }
                if s > max_val { max_val = s; }
            }
            peaks.push((min_val.clamp(-1.0, 0.0), max_val.clamp(0.0, 1.0)));
        }

        Self {
            peaks,
            samples_per_pixel: samples_per_bucket,
            duration_seconds: (samples.len() as f64) / 44100.0,
        }
    }

    /// Generates SVG path data `d="..."` for mirrored waveform bar rendering.
    pub fn to_svg_path(&self, width_px: f64, height_px: f64) -> String {
        if self.peaks.is_empty() {
            return String::new();
        }

        let mid_y = height_px * 0.5;
        let half_h = height_px * 0.45;
        let count = self.peaks.len();
        let dx = width_px / (count as f64);

        let mut path = String::with_capacity(count * 32);

        // Top half (positive peaks)
        for (i, &(_min, max)) in self.peaks.iter().enumerate() {
            let x = (i as f64) * dx;
            let y = mid_y - (max as f64 * half_h);
            if i == 0 {
                path.push_str(&format!("M {x:.1} {y:.1} "));
            } else {
                path.push_str(&format!("L {x:.1} {y:.1} "));
            }
        }

        // Bottom half (negative peaks in reverse order)
        for (i, &(min, _max)) in self.peaks.iter().enumerate().rev() {
            let x = (i as f64) * dx;
            let y = mid_y - (min as f64 * half_h);
            path.push_str(&format!("L {x:.1} {y:.1} "));
        }

        path.push('Z');
        path
    }
}
```

---

## 7. Type, Trait, and Module Architecture

### 7.1 Proposed Module Tree

```text
crates/player/
├── Cargo.toml
└── src/
    ├── lib.rs                   # Re-exports virtualizer types, Player, Controls
    ├── player.rs                # Core Player component
    ├── controls.rs              # Basic scrubber controls
    ├── native_preview.rs        # Native Scene -> SVG preview
    └── virtualizer/             # NEW MODULE for Requirement R3
        ├── mod.rs               # Public re-exports
        ├── slots.rs             # calculate_timestamp_slots, TimelineViewport, TimestampSlot
        ├── ruler.rs             # calculate_ruler_ticks, RulerTick
        ├── thumbnail_cache.rs   # ThumbnailCache, ThumbnailKey, ThumbnailItem
        ├── thumbnail_worker.rs  # ThumbnailGenerator, Async worker queue
        ├── waveform.rs          # WaveformPeaks, SVG generator
        └── filmstrip.rs         # FilmstripView Dioxus component

apps/studio/
└── src/
    ├── main.rs                  # Studio desktop app shell
    └── timeline/                # NEW MODULAR TIMELINE COMPONENTS
        ├── mod.rs
        ├── panel.rs             # Full TimelinePanel with zoom, pan, playhead
        ├── ruler_bar.rs         # Interactive ruler bar with timecodes
        ├── track_lane.rs        # Multi-track lanes (Video, Audio, Sequences)
        ├── clip_view.rs         # Sequence & Video clips with FilmstripView integration
        └── waveform_view.rs     # Audio track with WaveformPeaks rendering
```

---

## 8. Automated Testing & Verification Plan

### 8.1 Unit Tests to Implement

1. **`slots::tests` (`crates/player/src/virtualizer/slots.rs`)**:
   - `test_slots_standard_zoom_returns_continuous_partitions`: Verify slots cover visible width without gaps or overlaps.
   - `test_slots_extreme_zoom_in_and_out`: Test `zoom_factor = 0.001` (entire composition in 1 slot) and `zoom_factor = 100.0` (1 frame per slot).
   - `test_slots_overscan_buffering`: Verify `overscan_px` generates left and right buffer slots marked with `is_in_viewport = false`.
   - `test_slots_zero_duration_and_non_finite_inputs`: Verify safe handling of `duration = 0`, `fps = 0.0`, `zoom = -5.0`, `scroll_left = NaN`.
   - `test_slots_boundary_clamping_at_timeline_end`: Verify the final slot terminates exactly at `duration_in_frames - 1`.

2. **`ruler::tests` (`crates/player/src/virtualizer/ruler.rs`)**:
   - `test_ruler_adaptive_tick_intervals`: Check step transitions from frames -> 1s -> 5s -> 1m.
   - `test_ruler_timecode_formatting`: Validate formatting (`00:01:15` for sub-second, `01:30` for minutes).

3. **`thumbnail_cache::tests` (`crates/player/src/virtualizer/thumbnail_cache.rs`)**:
   - `test_thumbnail_cache_lru_eviction_on_entry_limit`: Insert 10 items into 5-item cache; ensure oldest 5 are evicted.
   - `test_thumbnail_cache_lru_eviction_on_byte_limit`: Insert large items; ensure byte capacity is respected.
   - `test_thumbnail_cache_hit_and_miss_metrics`: Validate hit/miss counter increments.

4. **`waveform::tests` (`crates/player/src/virtualizer/waveform.rs`)**:
   - `test_waveform_peak_extraction`: Verify min/max buckets match sine wave peak amplitudes.
   - `test_waveform_svg_path_generation`: Verify closed SVG path starts with `M` and ends with `Z`.

### 8.2 Verification Commands

```bash
# 1. Workspace compilation across all targets
cargo check --locked --workspace --all-targets --all-features

# 2. Workspace Clippy zero-warning enforcement
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

# 3. Comprehensive test execution across all crates
cargo test --locked --workspace --all-features

# 4. Code formatting verification
cargo fmt --all -- --check
```

---

## 9. Conclusion & Next Steps

This survey confirms that Requirement R3 can be implemented cleanly by introducing a dedicated `virtualizer` module inside `crates/player` (pure computational math, LRU cache, and modular Dioxus UI components) and updating `apps/studio`'s `TimelinePanel` to replace static placeholder tracks with virtualized, zoomable, filmstrip-backed track lanes.

All required types, algorithms, and test cases are specified and ready for implementation.
