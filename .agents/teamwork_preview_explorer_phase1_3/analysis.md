# Analysis Report: System Environment, FFmpeg Integration, & E2E Test Harness Requirements

**Author**: Explorer 3 (Phase 1 Exploration)  
**Target Project**: Dioxuscut (`/Users/sjkim1127/Dioxuscut`)  
**Date**: 2026-07-21  

---

## Executive Summary

This investigation evaluated the host environment, FFmpeg command-line configuration for H.264 MP4 encoding from PNG frame sequences, and designed a comprehensive 4-Tier E2E testing framework to validate the acceptance criteria for the `dioxuscut-cli` rendering pipeline (`cargo run -p dioxuscut-cli -- render -c HelloWorld -p data.json -o output.mp4 --width 1280 --height 720 --fps 30 --duration 60`).

Key findings:
1. **System Environment**: FFmpeg 8.1.1 (with `libx264`, `libx265`, `yuv420p`, `videotoolbox`, `neon`), `ffprobe` 8.1.1, Google Chrome 150.0 (located at `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`), Dioxus CLI 0.6.1 (`dx`), and Rust 1.97.0 are all installed and operational on the host system.
2. **FFmpeg Integration**: Standardizing the frame pattern to `frame_%06d.png` (or `frame-%06d.png`), enforcing output scaling (`-s 1280x720`), adding `-movflags +faststart`, `-preset fast`, and capturing `stderr` on subprocess failure will ensure robust, high-quality MP4 generation.
3. **E2E Test Suite**: A 4-Tier test strategy covering unit parsing (Tier 1), synthetic frame encoding (Tier 2), web server + headless browser lifecycle (Tier 3), and CLI binary end-to-end acceptance testing (Tier 4) guarantees full pipeline coverage.

---

## 1. System Environment & Tools Verification

Terminal checks were executed to confirm binary locations, versioning, and feature support:

| Tool | Status | Installed Version / Location | Notes |
|---|---|---|---|
| **FFmpeg** | **Available** | `8.1.1` (`/opt/homebrew/Cellar/ffmpeg/8.1.1/bin/ffmpeg`) | Includes `libx264`, `libx265`, `libsvtav1`, `libvmaf`, `neon`, `videotoolbox` |
| **FFprobe** | **Available** | `8.1.1` (`/opt/homebrew/Cellar/ffmpeg/8.1.1/bin/ffprobe`) | Useful for automated E2E video metadata assertions |
| **Google Chrome** | **Available** | `150.0.7871.129` (`/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`) | Auto-discovered by `headless_chrome` Rust crate on macOS |
| **Dioxus CLI** | **Available** | `dioxus 0.6.1 (c2952a7)` (`/Users/sjkim1127/.cargo/bin/dx`) | Ready for `dx serve` web application compilation |
| **Rust / Cargo** | **Available** | `cargo 1.97.0`, `rustc 1.97.0` | Workspace builds cleanly |

### Environment Observations & Constraints
- **Headless Chrome Binary Resolution**: `google-chrome` and `chromium` standard CLI aliases are not in system `PATH`, but Google Chrome is installed at the default macOS application bundle location (`/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`). The `headless_chrome` crate automatically detects this path when initializing browser instances on macOS.
- **FFmpeg Execution**: `ffmpeg` and `ffprobe` are present in standard `PATH` (`/opt/homebrew/bin/ffmpeg`), allowing direct process execution via `tokio::process::Command::new("ffmpeg")`.

---

## 2. FFmpeg MP4 Encoding Parameters & Command Flag Specification

To encode sequential PNG screenshots (`frame_%06d.png`) into a high-quality, universally compatible H.264 MP4 video, FFmpeg must be invoked with precise flags.

### 2.1 Canonical FFmpeg Command

```bash
ffmpeg -y \
  -framerate 30 \
  -i "/tmp/dioxuscut_frames/frame_%06d.png" \
  -c:v libx264 \
  -crf 18 \
  -preset fast \
  -pix_fmt yuv420p \
  -s 1280x720 \
  -movflags +faststart \
  "output.mp4"
```

### 2.2 Flag-by-Flag Deep Dive

1. **`-y` (Overwrite Output)**
   - Overwrites the target output file without asking for user confirmation. Required for unattended CLI automation.
2. **`-framerate <fps>` (Input Framerate)**
   - Placed **before** `-i`. Specifies the rate at which FFmpeg interprets the incoming image sequence (e.g., 30 fps = 1 frame per 33.33ms).
   - *Crucial*: Placing `-r` after `-i` without `-framerate` before `-i` causes FFmpeg to default input frame rate to 25 fps and drop/duplicate frames. `-framerate` before `-i` ensures exact 1:1 image-to-frame mapping.
3. **`-i <pattern>` (Input Sequence Pattern)**
   - Standard printf sequence pattern matching zero-padded 6-digit integers: `/path/to/frame_%06d.png`.
   - Files matching `frame_000000.png`, `frame_000001.png`, ..., `frame_000059.png` will be processed sequentially.
4. **`-c:v libx264` (Video Codec)**
   - Uses the H.264 / AVC video codec via `libx264`. H.264 in an MP4 container ensures universal playback compatibility across HTML5 `<video>`, macOS QuickTime, iOS, Android, and web browsers.
5. **`-crf <value>` (Constant Rate Factor quality control)**
   - Controls quality vs bitrate (range 0–51, where lower is better quality).
   - **CRF 18** is visually lossless for 2D/UI animations and text rendering. (Default libx264 CRF is 23; CRF 18 provides crisp text and line rendering without excessive file size).
6. **`-preset <preset_name>` (Encoding Speed/Efficiency Tradeoff)**
   - Options: `ultrafast`, `superfast`, `veryfast`, `faster`, `fast`, `medium`, `slow`, `slower`, `veryslow`.
   - **`fast`** or **`medium`** provides an optimal balance between execution speed and output compression efficiency for automated rendering tasks.
7. **`-pix_fmt yuv420p` (Pixel Format / Chroma Subsampling)**
   - PNG screenshots use 8-bit RGBA or RGB24 (`gbrp` / `yuv444p` in raw conversion).
   - **`yuv420p`** (Planar YUV 4:2:0) is **strictly mandatory** for standard video players (Apple QuickTime, iOS Safari, Chrome HTML5 player). Non-4:2:0 formats fail to play or show black screens in QuickTime.
   - *Note*: `yuv420p` requires width and height to be even numbers (divisible by 2).
8. **`-s <width>x<height>` (Target Output Resolution)**
   - Forces output resolution (e.g. `-s 1280x720`). Ensures output video dimensions match requested `--width` and `--height` regardless of browser display scaling or Retina device pixel ratio.
9. **`-movflags +faststart` (Progressive Download / Web Optimization)**
   - Relocates the MP4 `moov` atom (metadata index) from the end of the file to the beginning. Allows web players and browsers to start video playback immediately before downloading the full file.

### 2.3 Existing `encode.rs` Audit vs Target Requirements

In `crates/renderer/src/encode.rs`:
```rust
// Existing implementation:
let status = tokio::process::Command::new("ffmpeg")
    .args([
        "-y",
        "-framerate", &config.fps.to_string(),
        "-i",         &input_pattern, // currently pattern is frame-%06d.png
        "-c:v",       &config.codec,
        "-crf",       &config.crf.to_string(),
        "-pix_fmt",   &config.pixel_format,
        config.output.to_str().unwrap_or("output.mp4"),
    ])
```

**Identified gaps in `encode.rs`**:
- **Filename Pattern Consistency**: `encode.rs` uses `frame-%06d.png` (hyphen) whereas project spec and `render_frames.rs` / CLI prompt use `frame_%06d.png` (underscore). Pattern should be parameterized or standardized to `frame_%06d.png`.
- **Missing Resolution Scaling**: Width/Height parameters are not passed to FFmpeg (`-s {width}x{height}` or `-vf scale=...`).
- **Missing Optimization Flags**: `-movflags +faststart` and `-preset fast` are missing.
- **Subprocess Error Feedback**: Current implementation only checks `status.success()`. If FFmpeg fails, `stderr` is lost. Capturing `stderr` via `.output().await` provides actionable error messages in logs.

---

## 3. 4-Tier E2E Testing Framework Plan

To ensure the CLI command requirement (`cargo run -p dioxuscut-cli -- render -c HelloWorld -p data.json -o output.mp4 --width 1280 --height 720 --fps 30 --duration 60`) is reliably tested and validated, we define a 4-Tier testing strategy.

```
+-----------------------------------------------------------------------+
| Tier 4: Full E2E CLI Acceptance Test (cargo run -p dioxuscut-cli ...)  |
+-----------------------------------------------------------------------+
                                   |
+-----------------------------------------------------------------------+
| Tier 3: Web Server Lifecycle & Headless Chrome Capture Integration    |
+-----------------------------------------------------------------------+
                                   |
+-----------------------------------------------------------------------+
| Tier 2: Synthetic Frame Sequence FFmpeg Encoding Integration         |
+-----------------------------------------------------------------------+
                                   |
+-----------------------------------------------------------------------+
| Tier 1: Unit & Component Parser Tests (Clap CLI, Config Structs)      |
+-----------------------------------------------------------------------+
```

### 3.1 Tier 1: Unit & Component Tests (Fast, Isolated, Pure Rust)
- **Objective**: Verify CLI flag parsing, default parameter fallbacks, and configuration struct initialization without launching external processes.
- **Test Locations**: `crates/cli/src/main.rs` (unit tests module), `crates/renderer/src/render_frames.rs`, `crates/renderer/src/encode.rs`.
- **Test Cases**:
  1. **CLI Argument Parsing**: Verify `Cli::try_parse_from(["dioxuscut", "render", "-c", "HelloWorld", "-p", "data.json", "-o", "output.mp4", "--width", "1280", "--height", "720", "--fps", "30", "--duration", "60"])` populates all fields accurately.
  2. **Default CLI Flags**: Verify running `dioxuscut render -c Demo` defaults to `out.mp4`, 1920x1080, 30.0 fps, 150 duration frames.
  3. **Config Struct Builders**: Verify `RenderConfig::new()` and `EncodeConfig::h264()` generate expected path strings and frame ranges (`0..=59` for 60 duration frames).

### 3.2 Tier 2: Synthetic Pipeline & FFmpeg Encoding Tests (Medium Speed)
- **Objective**: Test FFmpeg process execution, image sequence ingestion, MP4 container generation, and frame directory cleanup without needing a web server or browser.
- **Test Location**: `crates/renderer/tests/encode_test.rs`.
- **Test Process**:
  1. Create a temporary folder using `tempfile::TempDir`.
  2. Generate 60 synthetic PNG images (`frame_000000.png` .. `frame_000059.png`) with simple RGB color patterns (using `image` crate or raw PNG encoder).
  3. Invoke `encode_frames(&EncodeConfig)`.
  4. **Validation via FFprobe**:
     - Run `ffprobe -v error -select_streams v:0 -show_entries stream=codec_name,width,height,r_frame_rate,duration -of json output.mp4`.
     - Assert `codec_name == "h264"`.
     - Assert `width == 1280` and `height == 720`.
     - Assert `duration` is ~2.0 seconds (`60 / 30`).
  5. Assert clean removal of temporary PNG directory.

### 3.3 Tier 3: Subsystem Integration Tests (Server Lifecycle & Headless Browser)
- **Objective**: Validate the automated web server lifecycle (spawn, port selection, health check, termination) and Headless Chrome DOM rendering (`window.DIOXUSCUT_FRAME`).
- **Test Location**: `crates/renderer/tests/browser_server_test.rs`.
- **Test Process**:
  1. **Server Lifecycle**: Spawn a local HTTP server on `127.0.0.1:0` (ephemeral free port), serve static HTML/JS with a mock `window.DIOXUSCUT_FRAME` receiver.
  2. **Health Check**: Poll `http://127.0.0.1:<port>/` until HTTP 200 OK is returned (with a 5-second timeout).
  3. **Browser Automation**: Launch `headless_chrome::Browser`, navigate to port, iterate `window.DIOXUSCUT_FRAME = 0..10`, capture screenshot bytes.
  4. **PNG Integrity Assertions**: Inspect captured bytes; verify PNG magic bytes (`\x89PNG\r\n\x1a\n`) and image header dimensions (1280x720).
  5. **Server Teardown**: Send shutdown signal to server process; assert port is released.

### 3.4 Tier 4: Full End-to-End CLI Pipeline Acceptance Test
- **Objective**: Directly validate the exact prompt acceptance criteria end-to-end.
- **Command**:
  ```bash
  cargo run -p dioxuscut-cli -- render -c HelloWorld -p data.json -o output.mp4 --width 1280 --height 720 --fps 30 --duration 60
  ```
- **Test Location**: `crates/cli/tests/e2e_render_test.rs`.
- **Automated Test Steps & Assertions**:
  1. **Fixture Setup**: Write a test input `data.json` file containing sample composition props.
  2. **Subprocess Run**: Execute `cargo run -p dioxuscut-cli -- render ...` using `std::process::Command` or `assert_cmd`.
  3. **Exit Code Assertion**: Assert process exit status `0`.
  4. **Log Tracing Assertion**: Assert stdout/stderr contains `tracing` subscriber log entries:
     - `Starting render for composition 'HelloWorld'`
     - `Server listening on http://127.0.0.1:<port>`
     - `Rendering frame 0..59`
     - `Encoding ... → output.mp4`
     - `Encode complete`
  5. **Output File Assertions**:
     - Assert `output.mp4` exists in file system.
     - Assert file size > 0 bytes.
     - Execute `ffprobe` to verify codec = `h264`, width = 1280, height = 720, duration = 2.0s, fps = 30.
  6. **Temp Artifact Cleanup Assertion**: Verify temporary frame output directory (`/tmp/dioxuscut_render_frames_*`) no longer exists.

---

## 4. Recommendations for Implementation & E2E Setup

### 4.1 FFmpeg Module Enhancements (`crates/renderer/src/encode.rs`)
1. **Standardize Frame Filename Pattern**:
   Change input pattern in `EncodeConfig` to `frame_%06d.png` (matching `render_frames.rs`).
2. **Add Width & Height Scaling**:
   Extend `EncodeConfig` struct to include `width: Option<u32>` and `height: Option<u32>`. When present, pass `-s {width}x{height}` to FFmpeg arguments.
3. **Add Faststart & Preset Flags**:
   Include `-movflags +faststart` and `-preset fast` in default `EncodeConfig::h264()`.
4. **Enhanced Error Logging**:
   Capture `output.stderr` when FFmpeg fails and return `RenderError::Encode(String::from_utf8_lossy(&output.stderr).to_string())`.

### 4.2 E2E Test Suite Setup (`crates/cli/tests/`)
1. **Add `assert_cmd` & `predicates` Dev-Dependencies**:
   Add `assert_cmd` to `crates/cli/Cargo.toml` under `[dev-dependencies]` for clean CLI process assertions.
2. **Automated `ffprobe` Helper in Tests**:
   Implement a helper function `verify_mp4_metadata(path: &Path, expected_width: u32, expected_height: u32, expected_duration: f64)` in test suite.
3. **RAII Temporary Directory Guard**:
   Use `tempfile::Builder::new().prefix("dioxuscut_frame_").tempdir()?` in `dioxuscut-renderer` to guarantee temp frame directory cleanup even if rendering panics or fails mid-way.

---

*Report compiled by Explorer 3.*
