# Project: Dioxuscut CLI Headless Rendering Pipeline Automation

## Architecture
Dioxuscut is a video creation framework written in Rust using Dioxus.
The CLI headless rendering pipeline automates frame extraction and video compilation:
1. **Server Manager**: Spawns web server (`dx serve` or embedded HTTP static server for Dioxus web app), binds to available port, polls health check endpoint until ready, manages clean process termination.
2. **Browser Renderer**: Connects via `headless_chrome` (Chrome DevTools Protocol), navigates to server URL, sets `window.DIOXUSCUT_FRAME`, waits for rendering/DOM updates, captures high-res PNG screenshots into temporary directory.
3. **Media Compiler**: Invokes FFmpeg CLI to encode PNG sequence into H.264 MP4 with specified FPS and resolution, cleans up temp frame directory.
4. **CLI Entrypoint**: Implements `dioxuscut-cli` with `dioxuscut render` command using `clap` and structured logging via `tracing-subscriber`.

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| 1 | Web Server Lifecycle | Web server spawning (`dx serve` / static HTTP server), port allocation, health checking, termination | None | DONE |
| 2 | Headless Chrome Frame Extractor | `headless_chrome` browser automation, `window.DIOXUSCUT_FRAME` injection, DOM wait, PNG capture | M1 | DONE |
| 3 | FFmpeg MP4 Encoding & Cleanup | FFmpeg command execution, FPS/resolution formatting, temp frame directory management & cleanup | M2 | IN_PROGRESS |
| 4 | CLI Command & E2E Integration | `dioxuscut render` CLI flags (`-c`, `-p`, `-o`, `--width`, `--height`, `--fps`, `--duration`), tracing logs, E2E validation | M1, M2, M3 | IN_PROGRESS |

## Interface Contracts
### CLI ↔ Renderer
- Command: `dioxuscut render -c <composition> -p <props.json> -o <output.mp4> --width <w> --height <h> --fps <fps> --duration <duration_in_frames>`
- Config Struct:
  ```rust
  pub struct RenderConfig {
      pub composition: String,
      pub props_path: PathBuf,
      pub output_path: PathBuf,
      pub width: u32,
      pub height: u32,
      pub fps: u32,
      pub duration_frames: u32,
  }
  ```

### Renderer ↔ Web App Browser Interface
- JS Global Variable: `window.DIOXUSCUT_FRAME = <frame_index: usize>`
- Screenshot format: PNG files named `frame_%06d.png` in temporary directory.

## Code Layout
- `crates/cli`: CLI binary crate `dioxuscut-cli` (`src/main.rs`, command line parsing, logging setup)
- `crates/renderer`: Rendering pipeline crate `dioxuscut-renderer` (`src/lib.rs`, `src/server.rs`, `src/browser.rs`, `src/ffmpeg.rs`)
- `crates/core`: Core types and compositions (`crates/core`)
- `apps/example`: Web application entry point for Dioxus web app rendering
