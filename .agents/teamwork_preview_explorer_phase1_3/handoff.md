# Handoff Report: System Environment, FFmpeg Integration, & Test Harness Planning

**Agent**: Explorer 3 (Phase 1 Exploration)  
**Working Directory**: `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_explorer_phase1_3`  
**Date**: 2026-07-21  

---

## 1. Observation

Direct system and codebase observations gathered during terminal and file inspection:

1. **System Tools Verification**:
   - `ffmpeg -version`: Output `ffmpeg version 8.1.1 Copyright (c) 2000-2026 the FFmpeg developers`, built with `--enable-libx264 --enable-videotoolbox --enable-neon`. Path: `/opt/homebrew/Cellar/ffmpeg/8.1.1/bin/ffmpeg`.
   - `ffprobe -version`: Output `ffprobe version 8.1.1`.
   - Google Chrome binary check: Binary found at `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`. Version: `Google Chrome 150.0.7871.129`. `google-chrome` and `chromium` standard CLI aliases are not in system `PATH`.
   - `dx --version`: Output `dioxus 0.6.1 (c2952a7)`. Binary located at `/Users/sjkim1127/.cargo/bin/dx`.
   - `cargo --version` / `rustc --version`: `cargo 1.97.0`, `rustc 1.97.0`.

2. **Existing Renderer & CLI Code Inspection**:
   - `crates/renderer/Cargo.toml`: Line 18 defines `headless_chrome = "1.0.12"`.
   - `crates/renderer/src/encode.rs`:
     - Lines 41–44: `let input_pattern = config.frames_dir.join("frame-%06d.png").to_string_lossy().to_string();`
     - Lines 57–66: `ffmpeg` CLI arguments: `["-y", "-framerate", &config.fps.to_string(), "-i", &input_pattern, "-c:v", &config.codec, "-crf", &config.crf.to_string(), "-pix_fmt", &config.pixel_format, config.output.to_str().unwrap_or("output.mp4")]`.
   - `crates/renderer/src/render_frames.rs`: Line 111 generates filenames using `frame-{frame:06}.png`.
   - `crates/cli/src/main.rs`:
     - Line 81: `let url = "http://localhost:8080".to_string();`
     - Line 83: `let out_dir = std::env::temp_dir().join("dioxuscut_render_frames");`

---

## 2. Logic Chain

1. **Host Environment Verification**:
   - Terminal check results (Observation 1) confirm that FFmpeg 8.1.1 (with `libx264`), `ffprobe` 8.1.1, Google Chrome 150.0, Dioxus CLI 0.6.1 (`dx`), and Cargo/Rustc 1.97.0 are present and operational.
   - On macOS, `headless_chrome` automatically checks `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`. Because Google Chrome 150.0 is present at this path, `headless_chrome` can connect to Chrome in headless mode without extra configuration.

2. **FFmpeg Command Optimization**:
   - `encode.rs` and `render_frames.rs` currently use `frame-%06d.png` (hyphen), whereas standard specifications require `frame_%06d.png` (underscore). Aligning this pattern ensures consistent frame file discovery.
   - For high-quality MP4 video output from PNG sequences:
     - `-framerate <fps>` before `-i` ensures exact 1:1 frame rate mapping.
     - `-c:v libx264 -crf 18 -preset fast` ensures visually lossless H.264 video quality at fast encoding speeds.
     - `-pix_fmt yuv420p` is required for standard MP4 playback in Apple QuickTime and web browsers.
     - `-s <width>x<height>` guarantees video output resolution matches requested `--width` and `--height`.
     - `-movflags +faststart` optimizes MP4 metadata layout for web streaming.

3. **E2E Testing Harness Strategy**:
   - To validate the acceptance criteria (`cargo run -p dioxuscut-cli -- render -c HelloWorld -p data.json -o output.mp4 --width 1280 --height 720 --fps 30 --duration 60`), tests must cover all components:
     - **Tier 1**: Unit tests for CLI parsing and default argument handling in `dioxuscut-cli`.
     - **Tier 2**: Integration test generating synthetic PNG frames in a temporary directory, invoking `encode_frames()`, and using `ffprobe` to verify output MP4 codec (`h264`), resolution (`1280x720`), duration (`2.0s`), and frame rate (`30fps`).
     - **Tier 3**: Subsystem test for local web server lifecycle (dynamic port binding, health polling, shutdown) and Headless Chrome DOM rendering (`window.DIOXUSCUT_FRAME`).
     - **Tier 4**: Complete E2E CLI test executing the full `cargo run -p dioxuscut-cli -- render` command, verifying zero exit code, log output via `tracing`, valid MP4 output via `ffprobe`, and automatic cleanup of temporary frame directories.

---

## 3. Caveats

- **Read-Only Scope**: In accordance with Explorer constraints, no production code modifications were made. Implementation of the recommended changes to `encode.rs` and test harness files will be performed in subsequent phases.
- **Port Allocation**: Server lifecycle design assumes binding to dynamic free ports (`127.0.0.1:0`) to prevent port conflicts during concurrent test runs.

---

## 4. Conclusion

- **System Environment**: FFmpeg 8.1.1, `ffprobe` 8.1.1, Google Chrome 150.0, `dx` CLI 0.6.1, and Rust 1.97.0 are verified and available.
- **FFmpeg Flags**: Detailed command-line flags (`-y`, `-framerate`, `-i frame_%06d.png`, `-c:v libx264`, `-crf 18`, `-preset fast`, `-pix_fmt yuv420p`, `-s 1280x720`, `-movflags +faststart`) are documented and audited against existing `encode.rs`.
- **E2E Test Plan**: 4-Tier test strategy fully mapped to acceptance criteria.
- **Artifacts**:
  - Full analysis report: `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_explorer_phase1_3/analysis.md`
  - Handoff report: `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_explorer_phase1_3/handoff.md`

---

## 5. Verification Method

To independently verify the observations and analysis:

1. **Verify Tool Availability Commands**:
   ```bash
   ffmpeg -version
   ffprobe -version
   "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --version
   dx --version
   cargo check --workspace
   ```
2. **Inspect Reports**:
   - `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_explorer_phase1_3/analysis.md`
   - `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_explorer_phase1_3/handoff.md`
