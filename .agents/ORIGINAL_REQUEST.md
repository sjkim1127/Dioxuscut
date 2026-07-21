# Original User Request

## 2026-07-21T13:13:18Z

Automate the complete Dioxuscut CLI headless rendering pipeline, from spawning the web app server to frame extraction via Headless Chrome and final MP4 encoding with FFmpeg.

Working directory: /Users/sjkim1127/Dioxuscut
Integrity mode: development

## Requirements

### R1. Automated Web Server Lifecycle
The CLI must automatically spawn the Dioxus web server (`dx serve` or an embedded HTTP static server) on an available local port, wait for health check readiness before capturing, and cleanly terminate the server process upon completion.

### R2. Frame Extraction via Headless Chrome
The renderer must connect to the local server using `headless_chrome`, iterate through frames `0..duration_in_frames` by setting `window.DIOXUSCUT_FRAME` in JS, wait for DOM re-renders, and write high-resolution PNG screenshots to a temporary directory.

### R3. FFmpeg MP4 Encoding & Cleanup
The renderer must automatically execute FFmpeg to encode the sequential PNG screenshots into an H.264 MP4 video file using the specified FPS and resolution, and subsequently remove the temporary frame directory.

### R4. Comprehensive CLI Command Interface
The `dioxuscut-cli` tool must provide a user-friendly CLI (`dioxuscut render`) taking `--composition`, `--props` (path to JSON input data), `--output`, `--width`, `--height`, `--fps`, and `--duration` flags.

## Acceptance Criteria

### Automated End-to-End Pipeline
- [ ] Running `cargo run -p dioxuscut-cli -- render -c HelloWorld -p data.json -o output.mp4 --width 1280 --height 720 --fps 30 --duration 60` produces a valid, playable `output.mp4`.
- [ ] No manual background server or manual intervention is required during the rendering process.
- [ ] Temporary frame files are cleaned up after encoding completes.
- [ ] Informative logging is provided via `tracing-subscriber` detailing server launch, frame capture progress, and FFmpeg encoding.
