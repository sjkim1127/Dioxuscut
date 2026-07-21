# Dioxuscut CLI Headless Rendering Pipeline — Orchestrator Plan

## Executive Summary
This project automates the Dioxuscut CLI headless rendering pipeline. The pipeline spans web app server spawning, headless browser frame capture via `headless_chrome` + `window.DIOXUSCUT_FRAME`, MP4 encoding via FFmpeg, and a CLI command (`dioxuscut render`).

## Architecture & Track Structure

### Implementation Track
- **M1: Automated Web Server Lifecycle (`dioxuscut-renderer` / `dioxuscut-cli`)**
  - Spawn web server (`dx serve` or embedded HTTP static server for dioxus web app).
  - Port auto-selection / dynamic port assignment.
  - Health check polling (`/` or readiness endpoint) before starting render.
  - Clean process termination on finish or error.

- **M2: Frame Extraction via Headless Chrome (`dioxuscut-renderer`)**
  - Launch Headless Chrome instance via `headless_chrome` crate.
  - Navigate to web app URL.
  - Iteratively set `window.DIOXUSCUT_FRAME = frame_idx` in browser context.
  - Wait for DOM re-render / frame ready signal.
  - Capture high-resolution PNG screenshot for each frame `0..duration_in_frames`.
  - Save PNGs sequentially into a temporary directory (e.g., `frame_0000.png`).

- **M3: FFmpeg MP4 Encoding & Cleanup (`dioxuscut-renderer` / `dioxuscut-media` or CLI)**
  - Execute FFmpeg process to stitch sequential PNG frames into H.264 MP4 output (`-i frame_%04d.png -c:v libx264 -pix_fmt yuv420p -r <fps> <output.mp4>`).
  - Handle custom resolution, FPS, output file path.
  - Safely remove temporary directory upon completion or failure cleanup.

- **M4: Comprehensive CLI Interface (`dioxuscut-cli`)**
  - Provide `dioxuscut render` command using `clap`.
  - Arguments: `--composition` (`-c`), `--props` (`-p`), `--output` (`-o`), `--width`, `--height`, `--fps`, `--duration`.
  - Integrate tracing/logging via `tracing-subscriber` detailing server launch, frame capture progress, and FFmpeg encoding.

### E2E Testing Track
- Build opaque-box E2E test suite covering:
  - Tier 1: Feature coverage (CLI parameter parsing, server spawning, single frame render, full video render).
  - Tier 2: Boundary & corner cases (invalid composition name, missing props file, invalid resolution, 0 fps/duration, cleanup on interrupt/error).
  - Tier 3: Cross-feature combinations (custom props + custom resolution + high fps + non-standard output path).
  - Tier 4: Real-world application scenario (`HelloWorld` composition rendering with actual `data.json` producing playable MP4).
- Publish `TEST_READY.md`.

## Execution Workflow
1. **Exploration**: Spawn Explorers to analyze existing `crates/` (`cli`, `renderer`, `media`, `core`, `player`), `apps/` (`studio`, `example`), `Cargo.toml`, dependencies, and system tools (`ffmpeg`, `dx`, Chrome/chromium).
2. **E2E Test Infra**: Spawn worker/sub-orchestrator to write test suite and create `TEST_READY.md`.
3. **Milestone Execution**: Run Explorer -> Worker -> Reviewers -> Challengers -> Forensic Auditor loop for each milestone.
4. **Verification & Audit**: Run end-to-end integration test (`cargo run -p dioxuscut-cli -- render ...`) and conduct Forensic Audit for zero integrity violations.
