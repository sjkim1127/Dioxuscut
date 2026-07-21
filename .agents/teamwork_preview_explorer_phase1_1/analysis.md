# Dioxuscut Phase 1 Workspace & Crate Analysis Report

## Executive Summary
This report presents a comprehensive investigation of the Dioxuscut Rust workspace (`/Users/sjkim1127/Dioxuscut`), examining workspace crates, dependency structure, existing implementations in `crates/cli` and `crates/renderer`, buildability verification, and architectural recommendations for Milestones M1 through M4.

The existing workspace compiles cleanly (`cargo check --workspace --all-targets`) and passes all 19 existing tests (`cargo test --workspace`). The baseline rendering and encoding pipeline is scaffolded in `crates/cli` and `crates/renderer`, but key components—such as dynamic web server lifecycle management (M1), robust frame rendering synchronization (M2), FFmpeg validation (M3), and end-to-end integration (M4)—contain stubs or hardcoded assumptions that require expansion.

---

## 1. Workspace & Crate Structure Overview

### Workspace Cargo Configuration (`Cargo.toml`)
- **Resolver**: Edition 2021, Resolver 2.
- **Workspace Members** (9 packages):
  1. `crates/animation` (`dioxuscut-animation`) — Math primitives (`interpolate`, `spring`, `easing`, color lerp).
  2. `crates/core` (`dioxuscut-core`) — Core Dioxus components (`Composition`, `Sequence`, `AbsoluteFill`, `Freeze`) and hooks (`use_current_frame`, `use_video_config`, `use_input_props`).
  3. `crates/media` (`dioxuscut-media`) — Video/audio/image components (`<Video>`, `<Audio>`, `<Img>`).
  4. `crates/player` (`dioxuscut-player`) — Desktop/Web UI `<Player>` component and playback controls.
  5. `crates/renderer` (`dioxuscut-renderer`) — Headless frame extraction and video encoding.
  6. `crates/transitions` (`dioxuscut-transitions`) — Visual transition components (`<Fade>`, `<Slide>`).
  7. `crates/cli` (`dioxuscut-cli`) — Command-line interface executable (`dioxuscut`).
  8. `apps/studio` (`studio`) — Desktop studio application.
  9. `apps/example` (`example`) — Sample web video composition app.

---

## 2. Dependency Audit & Workspace Dependencies Evaluation

### Current Workspace Dependencies (`Cargo.toml` lines 22–55)
- **UI & Runtime**: `dioxus` (0.6), `dioxus-web` (0.6), `dioxus-desktop` (0.6), `tokio` (1, full), `futures` (0.3).
- **Serialization & Error Handling**: `serde` (1), `serde_json` (1), `anyhow` (1), `thiserror` (1).
- **Logging & Math**: `tracing` (0.1), `tracing-subscriber` (0.3), `ordered-float` (4), `num-traits` (0.2).
- **Internal Crates**: Re-exported via path references in `[workspace.dependencies]`.

### Dependency Findings & Recommendations
1. **`headless_chrome`**:
   - *Current status*: Listed directly in `crates/renderer/Cargo.toml:18` as `headless_chrome = "1.0.12"`.
   - *Recommendation*: Move to `[workspace.dependencies]` in root `Cargo.toml` to unify version management across the workspace.

2. **`clap`**:
   - *Current status*: Listed directly in `crates/cli/Cargo.toml:17` as `clap = { version = "4.4", features = ["derive"] }`.
   - *Recommendation*: Move to `[workspace.dependencies]` in root `Cargo.toml`.

3. **Web Server & HTTP Client Crates (`axum`, `tower-http`, `reqwest`)**:
   - *Current status*: **Missing**. Neither `axum`, `tower-http`, `warp`, `hyper`, nor `reqwest` are included in any `Cargo.toml`.
   - *Need*: Milestone 1 requires web server spawning, ephemeral port allocation, and health check polling before headless chrome navigation.
   - *Recommendation*: Add `axum` (0.7), `tower-http` (0.5 with `fs` feature), and `reqwest` (0.11/0.12) or `tokio::net::TcpStream` for health checking to `[workspace.dependencies]`.

---

## 3. Analysis of Existing Implementations and Stubs

### A. CLI Binary (`crates/cli/src/main.rs`)
- **Implementation**:
  - Uses `clap::Parser` to parse subcommands (`dioxuscut render`).
  - Initializes `tracing_subscriber` with env filter `"info,dioxuscut_renderer=debug"`.
  - Reads input props from a JSON file if provided via `--props`.
  - Sets local environment variable `DIOXUSCUT_PROPS` in the CLI process.
- **Stubs & Deficiencies**:
  - **Hardcoded URL**: Line 81: `let url = "http://localhost:8080".to_string();`. Assumes a web server is already running manually on port 8080.
  - **Environment Variable Isolation**: Setting `std::env::set_var("DIOXUSCUT_PROPS", ...)` inside the CLI process does not propagate to an external child process (like `dx serve`) unless passed explicitly during sub-process spawning.
  - **No Signal Handling / Server Lifecycle**: No setup for spawning, health check polling, or graceful server teardown.

### B. Frame Renderer (`crates/renderer/src/render_frames.rs`)
- **Implementation**:
  - Defines `RenderConfig` (url, output_dir, width, height, fps, duration_in_frames, frame_range, concurrency).
  - Uses `headless_chrome::Browser` to navigate to `config.url`.
  - Iterates over `frame_range`, sets JS global `window.DIOXUSCUT_FRAME = frame`, captures PNG screenshots, writes files to `output_dir/frame-{frame:06}.png`.
- **Stubs & Deficiencies**:
  - **Frame Naming Inconsistency**: Saves frames as `frame-{frame:06}.png` (dash delimiter), whereas `encode.rs` and `PROJECT.md` expect `frame-%06d.png` (or `frame_%06d.png` with underscore).
  - **Arbitrary Fixed Sleep**: Uses fixed `sleep(Duration::from_millis(30))` for DOM rendering instead of deterministic rendering completion signals (`requestAnimationFrame` / DOM mutation check).
  - **Sequential Single-Tab Rendering**: `concurrency` field is present in `RenderConfig` but ignored; frame capture runs sequentially in a single tab.

### C. Media Compiler / Video Encoder (`crates/renderer/src/encode.rs`)
- **Implementation**:
  - Defines `EncodeConfig` (h264 default with crf=18, codec="libx264", pixel_format="yuv420p").
  - Spawns `ffmpeg` subprocess using `tokio::process::Command`.
- **Stubs & Deficiencies**:
  - **Missing Pre-flight Check**: Does not verify if `ffmpeg` is installed or accessible in `PATH` before attempting process execution.
  - **Error Output Truncation**: Does not capture `stderr` from FFmpeg when execution fails, making debugging difficult.

---

## 4. Buildability & Test Verification Results

### Build Verification Command
```bash
cargo check --workspace --all-targets
```
- **Result**: **SUCCESS** (0 errors, 3 harmless warnings regarding dead code/unused imports).
- **Execution Time**: ~1.77s.

### Test Verification Command
```bash
cargo test --workspace
```
- **Result**: **SUCCESS** (All 19 tests passed).
  - `dioxuscut-animation`: 16 unit tests passed, 3 doc-tests passed.
  - Other crates (`core`, `media`, `player`, `renderer`, `transitions`, `cli`, `example`, `studio`): Compiled test targets cleanly with 0 failures.

---

## 5. Recommendations for M1–M4 Crate Organization

### M1: Web Server Lifecycle
- **Target Crate**: `crates/renderer` (module `src/server.rs`) or standalone helper in `dioxuscut-renderer`.
- **Key Deliverables**:
  1. `ServerManager` struct that binds to an available ephemeral port (`std::net::TcpListener::bind("127.0.0.1:0")`).
  2. Support dual modes:
     - **Static Mode**: Embedded `axum` HTTP server serving pre-built Dioxus web app static assets.
     - **CLI Mode**: Subprocess runner executing `dx serve --port <port>`.
  3. Health check poller: repeated TCP connection attempt / HTTP GET `/` with timeout (e.g. 10s timeout, 100ms interval).
  4. RAII Drop guard to ensure child server processes are killed on exit or panics.

### M2: Headless Chrome Frame Extractor
- **Target Crate**: `crates/renderer` (modules `src/browser.rs`, `src/render_frames.rs`).
- **Key Deliverables**:
  1. Standardize frame file pattern to `frame_%06d.png`.
  2. Implement JS frame injection contract:
     - Set `window.DIOXUSCUT_FRAME = frame_index`.
     - Evaluate JS signal to wait for DOM update / layout completion.
  3. Implement multi-tab parallel frame rendering when `concurrency > 1`.

### M3: FFmpeg MP4 Encoding & Cleanup
- **Target Crate**: `crates/renderer` (module `src/encode.rs`).
- **Key Deliverables**:
  1. Add pre-flight `ffmpeg -version` check with clear error message if FFmpeg is missing.
  2. Capture stderr output from FFmpeg subprocess for detailed error reporting on encoding failure.
  3. Automatic cleanup of temporary PNG directory upon successful video creation.

### M4: CLI Command & E2E Integration
- **Target Crate**: `crates/cli` (`dioxuscut-cli`).
- **Key Deliverables**:
  1. Integrate `clap` CLI flags (`-c`, `-p`, `-o`, `--width`, `--height`, `--fps`, `--duration`).
  2. Connect end-to-end pipeline: `ServerManager::start()` → `render_frames()` → `encode_frames()` → `ServerManager::stop()`.
  3. Centralize dependencies (`clap`, `headless_chrome`, `axum`, `tower-http`, `reqwest`) in root `Cargo.toml`.
