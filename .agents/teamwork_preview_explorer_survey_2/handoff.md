# Handoff Report: Explorer 2 (Binary IPC Protocol Specialist)

## 1. Observation

1. **Workspace Crates & Dependencies (`Cargo.toml`, `crates/renderer/Cargo.toml`, `crates/cli/Cargo.toml`, `apps/studio/Cargo.toml`)**:
   - `Cargo.toml`: Lines 38-46 declare `tokio = { version = "1", features = ["full"] }`, `futures = "0.3"`, `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`.
   - `Cargo.lock`: Lines 584 and 6139 confirm `bytes 1.11.1` and `tokio-util 0.7.18` are already resolved in the dependency tree.
   - `crates/renderer/Cargo.toml`: Currently declares dependencies on `serde`, `serde_json`, `tokio`, `tracing`, `anyhow`, `thiserror`, `axum`, `tower-http`, `reqwest`. It lacks explicit `bytes` and `tokio-util` dependency declarations.

2. **Existing Renderer & CLI Communication (`crates/renderer/src/lib.rs`, `crates/cli/src/main.rs`, `apps/studio/src/main.rs`)**:
   - `crates/renderer/src/lib.rs`: Lines 13-21 expose `encode`, `render_frames`, `server`. It does not contain an IPC module or streaming codec.
   - `crates/cli/src/main.rs`: Lines 14-63 only handle `Commands::Render`. No `daemon` subcommand exists.
   - `apps/studio/src/main.rs`: Lines 388-420 execute rendering in-process by spawning a local async task calling `dioxuscut_cli::execute_render_command_with_registry_and_control`. The preview in lines 261-269 uses `NativeCompositionPreview` (Scene -> SVG) on the main UI thread.

3. **Existing Frame Rendering & Buffers (`crates/rasterizer/src/backend.rs`, `crates/rasterizer/src/render.rs`)**:
   - `crates/rasterizer/src/backend.rs`: Lines 58-61 define `RasterizerBackend::render_frame(&self, scene: &Scene, config: &FrameConfig) -> Result<RgbaImage, RasterError>`.
   - `crates/rasterizer/src/render.rs`: Lines 518-530 in `render_to_ffmpeg_pipe_fallible` convert `image::RgbaImage` into raw RGBA bytes via `img.into_raw()` (a `Vec<u8>`) and stream to FFmpeg stdin via `stdin.write_all(&rgba)`.

4. **Remotion Binary Protocol Specification Requirement (`ORIGINAL_REQUEST.md`)**:
   - `ORIGINAL_REQUEST.md`: Lines 19-24 specify:
     ```text
     ### R2. Zero-Copy Binary IPC Protocol (`crates/renderer`, `crates/cli`, `apps/studio`)
     Implement Remotion's binary streaming IPC protocol (`remotion_buffer:<nonce>:<len>:<status>:<payload>`):
     - Binary packet framing and framing parser (`make_streamer` / `StreamEncoder` & `StreamDecoder`) supporting chunked streaming.
     - Fast transport of raw RGBA pixel frames and JSON commands between backend render processes and UI clients without serialization overhead.
     - Error code signaling and asynchronous response correlation via unique message nonces.
     ```

## 2. Logic Chain

1. **Premise 1 (From Observation 1 & 4)**: The protocol requires framing raw binary buffers and JSON messages using `remotion_buffer:<nonce>:<len>:<status>:<payload>`. To implement this in Tokio's ecosystem without memory copying or reallocations, `bytes::BytesMut` and `tokio_util::codec::{Decoder, Encoder, Framed}` are the standard Rust idioms.
2. **Premise 2 (From Observation 2 & 3)**: Frame rendering already produces contiguous `RgbaImage` data (`Vec<u8>` of size `width * height * 4`). In an IPC architecture, `Vec<u8>` can be wrapped directly into `bytes::Bytes` with zero-copy overhead (`Bytes::from(rendered.into_raw())`).
3. **Premise 3 (From Observation 4)**: Real-time UI timeline scrubbing and thumbnail virtualizers generate concurrent, out-of-order frame requests. A monotonic `AtomicU64` request nonce coupled with a client-side `HashMap<u64, oneshot::Sender<BinaryPacket>>` allows fully asynchronous, multiplexed request/response correlation over a single duplex stream (stdio, pipe, or socket) with zero head-of-line blocking.
4. **Premise 4 (From Observation 2)**: Adding `dioxuscut-renderer::ipc` (housing `BinaryPacket`, `BinaryIpcCodec`, `StreamDecoder`, `StreamEncoder`, `make_streamer`, `DaemonClient`, `DaemonServer`) and adding a `Commands::Daemon` subcommand to `dioxuscut-cli` will provide the standalone worker daemon needed by Studio and CLI tools.

## 3. Caveats

- Standard I/O mode (`dioxuscut daemon --stdio`) requires all logging via `tracing_subscriber` to be redirected strictly to `stderr` (`.with_writer(std::io::stderr)`). If any log output were written to `stdout`, it would corrupt the binary stream.
- In memory-constrained environments, streaming very high resolution uncompressed frames (e.g. 4K at 33MB per frame) requires the codec's `max_payload_bytes` limit to be configured appropriately (defaulting to 128MB).
- No caveats regarding architectural feasibility; the required crates (`bytes`, `tokio-util`) are already present in `Cargo.lock`.

## 4. Conclusion

Requirement R2 is fully analyzed and designed with an end-to-end architecture:
1. **Module Layout**: Place binary IPC protocol and codecs in `crates/renderer/src/ipc/` (`protocol.rs`, `codec.rs`, `client.rs`, `server.rs`, `error.rs`).
2. **Framing & Codec**: Implement `BinaryIpcCodec` with `remotion_buffer:<nonce>:<len>:<status>:<payload>` format, resynchronization support, and zero-copy `Bytes` slicing.
3. **Async Integration**: Expose `make_streamer`, `DaemonClient` (with async nonce correlation), and `DaemonServer` (with command router for `RenderFrame`, `RenderThumbnail`, `GetMetadata`, etc.).
4. **CLI Daemon**: Add `dioxuscut daemon [--stdio] [--socket <PATH>] [--port <PORT>]` to `crates/cli`.
5. **Detailed Documentation**: Detailed technical specifications, code designs, type signatures, and test matrices are recorded in `/Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_explorer_survey_2/report.md`.

## 5. Verification Method

1. **Inspect Survey Report**:
   ```bash
   view_file /Users/sjkim1127/Dioxuscut/.agents/teamwork_preview_explorer_survey_2/report.md
   ```
2. **Independent Workspace Quality Checks**:
   ```bash
   cargo check --locked --workspace --all-targets --all-features
   cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
   cargo test --locked --workspace --all-features
   cargo fmt --all -- --check
   ```
3. **Invalidation Conditions**:
   - If the protocol prefix differs from `remotion_buffer:` or the header field ordering `<nonce>:<len>:<status>:` is modified without updating the streaming decoder.
   - If tracing subscriber in stdio daemon mode writes to `stdout` instead of `stderr`.
