## 2026-07-21T13:27:07Z
Task (Milestone 3: FFmpeg MP4 Encoding & Cleanup):
Implement Requirement R3 in `crates/renderer/src/encode.rs` and `crates/renderer/src/lib.rs`:
1. Implement FFmpeg CLI process invocation for encoding sequential PNG screenshots (`frame_%06d.png`) into H.264 MP4:
   - Command: `ffmpeg -y -framerate <fps> -i <frames_dir>/frame_%06d.png -c:v libx264 -crf 18 -preset fast -pix_fmt yuv420p -s <width>x<height> -movflags +faststart <output_mp4>`
   - Log FFmpeg invocation and capture stderr output with `tracing::info!` / `tracing::error!`.
   - Implement temporary frame directory removal upon completion (or cleanup helper).
2. Expose `pub fn encode_mp4(...)` and `pub fn cleanup_frames(...)` in `crates/renderer`.
3. Add unit test in `encode.rs` testing FFmpeg command construction or mock/synthetic frame encoding.
4. Verify `cargo check -p dioxuscut-renderer` and `cargo test -p dioxuscut-renderer`.
