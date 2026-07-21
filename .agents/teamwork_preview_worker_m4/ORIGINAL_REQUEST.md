## 2026-07-21T13:33:40Z
<USER_REQUEST>
You are Implementation Worker M4 for Dioxuscut project.
Your working directory is /Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_worker_m4.
Please create your working directory if needed and produce your handoff report at /Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_worker_m4/handoff.md.

Task (Milestone 4: CLI Interface & Full Pipeline Integration):
Implement Requirement R4 and full end-to-end integration in `crates/cli`:
1. Implement `dioxuscut-cli` CLI binary command interface using `clap`:
   - Command `dioxuscut render` accepting `-c, --composition`, `-p, --props`, `-o, --output`, `--width`, `--height`, `--fps`, `--duration`.
2. Set up `tracing-subscriber` logging format (`tracing_subscriber::fmt::init()` or `EnvFilter`) detailing:
   - Web server launch and health check readiness.
   - Headless Chrome launch & frame extraction progress (`Captured frame N/M`).
   - FFmpeg encoding launch & output completion.
   - Temporary frame directory cleanup.
3. Wire the full automated rendering pipeline:
   - Spawn web server -> Extract PNG frames via Headless Chrome -> Encode MP4 via FFmpeg -> Clean up temp frames -> Terminate server.
4. Test running `cargo run -p dioxuscut-cli -- render -c HelloWorld -p data.json -o output.mp4 --width 1280 --height 720 --fps 30 --duration 60` and verify it generates valid, playable `output.mp4`.
5. Run `cargo test --workspace` and `cargo check --workspace` to ensure 100% tests pass.

MANDATORY INTEGRITY WARNING:
DO NOT CHEAT. All implementations must be genuine. DO NOT hardcode test results, create dummy/facade implementations, or circumvent the intended task. A Forensic Auditor will independently verify your work. Integrity violations WILL be detected and your work WILL be rejected.

Attach report paths in your handoff.
</USER_REQUEST>
