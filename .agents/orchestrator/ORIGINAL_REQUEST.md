# Original User Request

## 2026-07-21T13:13:28Z

Automate the complete Dioxuscut CLI headless rendering pipeline, from spawning the web app server to frame extraction via Headless Chrome and final MP4 encoding with FFmpeg.

Working directory: /Users/sjkim1127/Dioxuscut

### Requirements
- R1: Automated Web Server Lifecycle (`dx serve` or embedded HTTP static server)
- R2: Frame Extraction via Headless Chrome (`headless_chrome`, `window.DIOXUSCUT_FRAME`)
- R3: FFmpeg MP4 Encoding & Cleanup
- R4: Comprehensive CLI Command Interface (`dioxuscut render` flags)

### Acceptance Criteria
- Running `cargo run -p dioxuscut-cli -- render -c HelloWorld -p data.json -o output.mp4 --width 1280 --height 720 --fps 30 --duration 60` produces a valid, playable `output.mp4`.
- No manual background server or manual intervention is required during the rendering process.
- Temporary frame files are cleaned up after encoding completes.
- Informative logging is provided via `tracing-subscriber` detailing server launch, frame capture progress, and FFmpeg encoding.
