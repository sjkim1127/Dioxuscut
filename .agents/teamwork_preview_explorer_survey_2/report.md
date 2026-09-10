# Requirement R2: Zero-Copy Binary IPC Protocol Survey & Architectural Specification

## 1. Executive Summary

This report provides a comprehensive architectural survey and detailed implementation design for **Requirement R2 (Zero-Copy Binary IPC Protocol)** in the Dioxuscut workspace.

In modern high-performance video tooling architectures (exemplified by Remotion's Rust compositor daemon `packages/compositor/rust`), video editing UI processes (e.g. Dioxus Studio desktop app) offload timeline frame rasterization, video decoding, and thumbnail generation to a background worker daemon. Communicating millions of raw RGBA pixel bytes (e.g., 8.3 MB per 1080p frame, 33.2 MB per 4K frame) over JSON or Base64 introduces devastating memory bloat, CPU serialization overhead, and timeline scrubbing lag.

The **Remotion-equivalent Zero-Copy Binary Streaming IPC Protocol** solves this by establishing a chunk-framed, asynchronous, multiplexed binary protocol over standard I/O streams (`stdin`/`stdout`), Unix Domain Sockets / named pipes, or TCP sockets using the wire framing:
```text
remotion_buffer:<nonce>:<len>:<status>:<payload>
```

This report maps out the complete protocol framing, streaming codecs (`StreamDecoder` / `StreamEncoder` / `BinaryIpcCodec`), async I/O integration, asynchronous request correlation via nonces, error handling, CLI daemon subcommand (`dioxuscut daemon`), and UI/Studio client integration.

---

## 2. Existing Codebase Survey & Gap Analysis

### 2.1 Current State of Crates

| Crate | Existing Responsibilities | Current Communication / I/O Model | Gaps for R2 |
|---|---|---|---|
| `crates/renderer` | Static axum HTTP server (`server.rs`), FFmpeg PNG stitching CLI wrapper (`encode.rs`), RenderConfig (`render_frames.rs`). | HTTP server for static web assets; child process spawning for ffmpeg CLI. | No binary IPC protocol, no streaming codec, no daemon/client RPC layer. |
| `crates/cli` | Clap CLI parser (`main.rs`, `lib.rs`), validation, Rhai script execution, direct in-process render dispatch. | Standalone CLI (`dioxuscut render ...`), writes directly to ffmpeg pipe or disk. | Lacks a long-running `daemon` subcommand, stdio/socket IPC listener, or IPC worker dispatch. |
| `apps/studio` | Desktop editor UI (`dioxus-desktop`), composition catalogue, player preview, properties panel, in-process render queue. | In-process asynchronous task spawning calling `dioxuscut-cli` directly; player renders Scene->SVG on UI thread. | UI thread handles scene generation; lacks IPC client to stream raw frames from daemon; scrubbing can stall UI. |
| `crates/rasterizer` | Scene IR, `TinySkiaBackend` (CPU), `WgpuBackend` (GPU), FFmpeg pipe streaming (`render.rs`), `VideoFrameCache`, `ImageCache`. | Direct Rust function calls (`backend.render_frame(...)`), Rayon parallel pipeline. | Frame cache is localized to single render runs; lacks IPC exposure for single-frame queries. |
| `crates/player` | `<Player>` playback component, timeline time tracking, `NativeCompositionPreview` (Scene -> SVG adapter). | SVG DOM generation inside Dioxus virtual DOM. | No binary frame receiver / raw texture buffer updater. |

### 2.2 Workspace Dependencies & Capability Analysis

- `tokio = { version = "1", features = ["full"] }` is present in workspace dependencies.
- `bytes = "1"` is already present in `Cargo.lock` (used transitively by axum/reqwest/tokio).
- `tokio-util = { version = "0.7", features = ["codec"] }` is present in `Cargo.lock` (used transitively by axum/tower).
- `image = "0.25"` is present for RGBA image buffer representation.
- `serde` and `serde_json` are present for command serialization.

---

## 3. Remotion Binary Streaming IPC Protocol Specification

### 3.1 Packet Wire Format

The Remotion binary streaming IPC protocol frames all messages (both commands and binary data responses) using an ASCII-prefixed header followed by raw payload bytes:

```text
+------------------+---------+---+-------+---+----------+---+-----------------------+
| remotion_buffer: | <nonce> | : | <len> | : | <status> | : | <payload: len bytes>  |
+------------------+---------+---+-------+---+----------+---+-----------------------+
```

#### Field Specifications:

1. **Magic Header Prefix (`b"remotion_buffer:"`)**:
   - Fixed 16-byte ASCII string: `remotion_buffer:` (`0x72 0x65 0x6d 0x6f 0x74 0x69 0x6f 0x6e 0x5f 0x62 0x75 0x66 0x66 0x65 0x72 0x3a`).
   - Acts as a synchronization boundary. If log messages or foreign bytes precede the packet, the parser can seek forward to `remotion_buffer:` to resynchronize without dropping the connection.

2. **Nonce / Correlation ID (`<nonce>`)**:
   - ASCII decimal integer (e.g. `1`, `42`, `18446744073709551615`).
   - Represents a unique 64-bit unsigned integer request identifier (`u64`).
   - The daemon copies the `<nonce>` from incoming requests into its response packet.
   - Allows asynchronous, concurrent, and out-of-order response processing over a single multiplexed channel without head-of-line blocking.

3. **Payload Length (`<len>`)**:
   - ASCII decimal integer representing the exact byte count of the subsequent binary payload (e.g. `0`, `124`, `8294400`).
   - Allows `<payload>` to contain arbitrary binary bytes (including colons `:`, nulls `\0`, newlines `\n`, control characters) without escaping or framing corruption.

4. **Status Code (`<status>`)**:
   - ASCII decimal integer representing the execution result.
   - `0` (`STATUS_OK`): Indicates successful execution. Payload contains the requested data (raw RGBA buffer or JSON response).
   - Non-zero (`STATUS_ERR_*`): Indicates an error. Payload contains a UTF-8 JSON or text description of the error.

5. **Header Delimiters (`:`)**:
   - ASCII colon `b':'` separates `remotion_buffer`, `nonce`, `len`, and `status`.
   - The header ends with the 4th colon (`:`), immediately after which the binary payload begins.

6. **Payload (`<payload>`)**:
   - Exactly `<len>` raw bytes.
   - No trailing newline, carriage return, or padding bytes. The next packet's `remotion_buffer:` begins immediately at byte offset `header_length + len`.

---

## 4. Binary Packet Framing & Streaming Parser Design

### 4.1 Architecture Diagram

```text
           ┌────────────────────────────────────────────────────────┐
           │                  Async Byte Stream                     │
           │        (tokio::io::AsyncRead / AsyncWrite)             │
           └───────────────────────────┬────────────────────────────┘
                                       │
                        ┌──────────────┴──────────────┐
                        │                             │
                        ▼                             ▼
        ┌───────────────────────────────┐  ┌───────────────────────────────┐
        │        StreamDecoder          │  │        StreamEncoder          │
        │      (BinaryIpcCodec)         │  │      (BinaryIpcCodec)         │
        │                               │  │                               │
        │ • Buffer scanning             │  │ • Format ASCII header         │
        │ • Resynchronization           │  │ • Append raw payload bytes    │
        │ • Zero-copy split to Bytes    │  │ • Vectored / zero-copy write  │
        └───────────────┬───────────────┘  └──────────────▲────────────────┘
                        │                             │
                        ▼                             │
        ┌─────────────────────────────────────────────────────────────┐
        │                       BinaryPacket                          │
        │ { nonce: u64, status: u32, payload: bytes::Bytes }          │
        └─────────────────────────────────────────────────────────────┘
```

### 4.2 StreamDecoder (Framing Parser) Implementation Logic

The streaming decoder must handle all network/pipe realities:
- Chunk fragmentation: A header or payload split across multiple TCP/pipe read calls.
- Packet coalescence: Multiple complete packets arriving in a single `read()` call.
- Garbage / Log resilience: Non-protocol bytes (e.g. stray stdout text) discarded until `remotion_buffer:` is found.

#### Decoding State Machine:
```rust
use bytes::{Buf, Bytes, BytesMut};
use tokio_util::codec::Decoder;

pub const BUFFER_PREFIX: &[u8] = b"remotion_buffer:";
pub const STATUS_OK: u32 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryPacket {
    pub nonce: u64,
    pub status: u32,
    pub payload: Bytes,
}

pub struct BinaryIpcCodec {
    max_payload_bytes: usize, // e.g. 128 MB default limit for safety
}

impl Decoder for BinaryIpcCodec {
    type Item = BinaryPacket;
    type Error = IpcError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        loop {
            if src.len() < BUFFER_PREFIX.len() {
                return Ok(None);
            }

            // 1. Locate magic prefix
            let prefix_pos = match find_subsequence(src, BUFFER_PREFIX) {
                Some(pos) => pos,
                None => {
                    // Retain the last prefix_len - 1 bytes in case the prefix is split across reads
                    let keep = BUFFER_PREFIX.len() - 1;
                    if src.len() > keep {
                        src.advance(src.len() - keep);
                    }
                    return Ok(None);
                }
            };

            // Discard any garbage bytes before the prefix
            if prefix_pos > 0 {
                src.advance(prefix_pos);
            }

            // 2. Scan for colons separating header fields
            // Format: remotion_buffer:<nonce>:<len>:<status>:
            let after_prefix = &src[BUFFER_PREFIX.len()..];
            let mut colon_offsets = Vec::with_capacity(3);
            for (idx, &byte) in after_prefix.iter().enumerate() {
                if byte == b':' {
                    colon_offsets.push(BUFFER_PREFIX.len() + idx);
                    if colon_offsets.len() == 3 {
                        break;
                    }
                }
            }

            if colon_offsets.len() < 3 {
                // Header incomplete; await more data
                return Ok(None);
            }

            let c1 = colon_offsets[0]; // End of nonce
            let c2 = colon_offsets[1]; // End of len
            let c3 = colon_offsets[2]; // End of status (end of header)

            let nonce_str = std::str::from_utf8(&src[BUFFER_PREFIX.len()..c1])
                .map_err(|_| IpcError::HeaderParseError("Invalid UTF-8 in nonce".into()))?;
            let len_str = std::str::from_utf8(&src[c1 + 1..c2])
                .map_err(|_| IpcError::HeaderParseError("Invalid UTF-8 in len".into()))?;
            let status_str = std::str::from_utf8(&src[c2 + 1..c3])
                .map_err(|_| IpcError::HeaderParseError("Invalid UTF-8 in status".into()))?;

            let nonce = match nonce_str.parse::<u64>() {
                Ok(n) => n,
                Err(_) => {
                    // Invalid header data; skip prefix to resync
                    src.advance(BUFFER_PREFIX.len());
                    continue;
                }
            };

            let len = match len_str.parse::<usize>() {
                Ok(l) => l,
                Err(_) => {
                    src.advance(BUFFER_PREFIX.len());
                    continue;
                }
            };

            let status = match status_str.parse::<u32>() {
                Ok(s) => s,
                Err(_) => {
                    src.advance(BUFFER_PREFIX.len());
                    continue;
                }
            };

            if len > self.max_payload_bytes {
                return Err(IpcError::PayloadTooLarge {
                    len,
                    max: self.max_payload_bytes,
                });
            }

            let header_len = c3 + 1;
            let total_len = header_len + len;

            if src.len() < total_len {
                // Have full header, but incomplete payload. Reserve capacity and wait.
                src.reserve(total_len - src.len());
                return Ok(None);
            }

            // 3. Extract header and slice payload zero-copy
            src.advance(header_len);
            let payload = src.split_to(len).freeze();

            return Ok(Some(BinaryPacket {
                nonce,
                status,
                payload,
            }));
        }
    }
}
```

### 4.3 StreamEncoder Implementation

```rust
use tokio_util::codec::Encoder;

impl Encoder<BinaryPacket> for BinaryIpcCodec {
    type Error = IpcError;

    fn encode(&mut self, item: BinaryPacket, dst: &mut BytesMut) -> Result<(), Self::Error> {
        use std::io::Write;
        let mut header_buf = [0u8; 64];
        let mut cursor = std::io::Cursor::new(&mut header_buf[..]);
        write!(
            cursor,
            "remotion_buffer:{}:{}:{}:",
            item.nonce,
            item.payload.len(),
            item.status
        )
        .map_err(IpcError::Io)?;

        let header_len = cursor.position() as usize;
        let header_bytes = &header_buf[..header_len];

        dst.reserve(header_len + item.payload.len());
        dst.extend_from_slice(header_bytes);
        dst.extend_from_slice(&item.payload);
        Ok(())
    }
}
```

### 4.4 `make_streamer` Helper Function

```rust
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::Framed;

/// Constructs a bidirectional packet stream/sink over any asynchronous I/O channel.
pub fn make_streamer<T>(io: T) -> Framed<T, BinaryIpcCodec>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    Framed::new(io, BinaryIpcCodec::default())
}
```

---

## 5. Transport of Raw RGBA Pixel Frames vs JSON Commands

### 5.1 Command Protocol Design

Commands between the client and daemon are serialized as JSON inside the packet payload for flexibility and schema evolution, while frame pixel buffers are transported as raw, uncompressed binary data with zero copying.

```text
Request (Client -> Daemon):
remotion_buffer:<nonce>:<len>:0:{"type":"RenderFrame","composition_id":"HelloWorld","frame":42,"width":1920,"height":1080,"fps":30.0}

Response (Daemon -> Client - Cache Miss -> Rendered Frame):
remotion_buffer:<nonce>:8294400:0:<8,294,400 raw RGBA bytes>

Response (Daemon -> Client - Error):
remotion_buffer:<nonce>:45:3:{"error":"Composition 'Unknown' not found"}
```

### 5.2 Command Enums & Message Types

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum DaemonCommand {
    Ping,
    GetMetadata {
        composition_id: String,
    },
    ListCompositions,
    RenderFrame {
        composition_id: String,
        frame: u32,
        width: u32,
        height: u32,
        fps: f64,
        props: serde_json::Value,
    },
    RenderThumbnail {
        composition_id: String,
        frame: u32,
        width: u32,
        height: u32,
        fps: f64,
        props: serde_json::Value,
    },
    GetCacheStats,
    ClearCache,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum DaemonResponse {
    Pong,
    CompositionsList {
        ids: Vec<String>,
    },
    CompositionMetadata {
        id: String,
        width: u32,
        height: u32,
        fps: f64,
        duration_in_frames: u32,
    },
    CacheStats {
        cached_frames: usize,
        cached_bytes: usize,
        hits: u64,
        misses: u64,
    },
    Success,
    Error {
        code: u32,
        message: String,
    },
}
```

### 5.3 Zero-Copy Invariant Analysis

1. **Daemon Side**:
   - `TinySkiaBackend::render_frame()` yields `image::RgbaImage`.
   - `RgbaImage::into_raw()` takes ownership of `Vec<u8>` without copying.
   - `Bytes::from(vec)` creates zero-copy `Bytes` buffer.
   - `BinaryIpcCodec::encode` writes packet header and sends buffer chunks directly to the OS socket/pipe buffer.

2. **Client Side**:
   - `StreamDecoder` reads socket buffer into `BytesMut`.
   - `split_to(len).freeze()` produces `Bytes` slice pointing to existing allocation.
   - For UI display: `RgbaImage::from_raw(width, height, bytes.to_vec())` or direct GPU texture upload `wgpu::Queue::write_texture(...)` passing `&bytes[..]`.

---

## 6. Asynchronous Response Correlation & Nonce Management

### 6.1 Concurrency Model

When scrubbing the timeline or rendering filmstrip thumbnails, the client issues dozens of concurrent requests. Because frames vary in complexity and cache residency (cache hit vs miss), responses can arrive out-of-order.

```text
Client                                             Daemon (Worker Pool)
  │                                                         │
  │─── Request (nonce: 1, frame: 10) ──────────────────────>│ (Cache hit: returns in 0.2ms)
  │─── Request (nonce: 2, frame: 11) ──────────────────────>│ (Cache miss: renders in 15ms)
  │─── Request (nonce: 3, frame: 12) ──────────────────────>│ (Cache miss: renders in 16ms)
  │                                                         │
  │<── Response (nonce: 1, status: 0, 8.3MB RGBA) ──────────│ [Responds 1st]
  │<── Response (nonce: 3, status: 0, 8.3MB RGBA) ──────────│ [Responds 2nd]
  │<── Response (nonce: 2, status: 0, 8.3MB RGBA) ──────────│ [Responds 3rd]
  ▼                                                         ▼
```

### 6.2 DaemonClient Design

```rust
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use bytes::Bytes;

pub struct DaemonClient {
    next_nonce: AtomicU64,
    outgoing_tx: mpsc::Sender<BinaryPacket>,
    pending_map: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<BinaryPacket, IpcError>>>>>,
}

impl DaemonClient {
    pub fn new<R, W>(reader: R, mut writer: W) -> Self
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W: tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<BinaryPacket>(64);
        let pending_map = Arc::new(Mutex::new(HashMap::<u64, oneshot::Sender<Result<BinaryPacket, IpcError>>>::new()));

        // Writer task
        let mut framed_writer = tokio_util::codec::FramedWrite::new(writer, BinaryIpcCodec::default());
        tokio::spawn(async move {
            use futures::SinkExt;
            while let Some(packet) = outgoing_rx.recv().await {
                if let Err(e) = framed_writer.send(packet).await {
                    tracing::error!("Failed to write IPC packet: {e}");
                    break;
                }
            }
        });

        // Reader task
        let mut framed_reader = tokio_util::codec::FramedRead::new(reader, BinaryIpcCodec::default());
        let pending_clone = Arc::clone(&pending_map);
        tokio::spawn(async move {
            use futures::StreamExt;
            while let Some(res) = framed_reader.next().await {
                match res {
                    Ok(packet) => {
                        let mut map = pending_clone.lock().await;
                        if let Some(tx) = map.remove(&packet.nonce) {
                            let _ = tx.send(Ok(packet));
                        } else {
                            tracing::warn!("Received unsolicited or timed-out packet nonce: {}", packet.nonce);
                        }
                    }
                    Err(e) => {
                        tracing::error!("IPC reader error: {e}");
                        let mut map = pending_clone.lock().await;
                        for (_, tx) in map.drain() {
                            let _ = tx.send(Err(IpcError::ConnectionClosed));
                        }
                        break;
                    }
                }
            }
        });

        Self {
            next_nonce: AtomicU64::new(1),
            outgoing_tx,
            pending_map,
        }
    }

    pub async fn send_request(&self, command: DaemonCommand) -> Result<BinaryPacket, IpcError> {
        let nonce = self.next_nonce.fetch_add(1, Ordering::Relaxed);
        let json_bytes = serde_json::to_vec(&command).map_err(IpcError::Json)?;
        let packet = BinaryPacket {
            nonce,
            status: STATUS_OK,
            payload: Bytes::from(json_bytes),
        };

        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending_map.lock().await;
            map.insert(nonce, tx);
        }

        self.outgoing_tx.send(packet).await.map_err(|_| IpcError::ConnectionClosed)?;

        let response_packet = rx.await.map_err(|_| IpcError::ConnectionClosed)??;
        if response_packet.status != STATUS_OK {
            let error_msg = String::from_utf8_lossy(&response_packet.payload).to_string();
            return Err(IpcError::ResponseError {
                status: response_packet.status,
                message: error_msg,
            });
        }

        Ok(response_packet)
    }

    pub async fn render_frame(
        &self,
        composition_id: String,
        frame: u32,
        width: u32,
        height: u32,
        fps: f64,
        props: serde_json::Value,
    ) -> Result<Bytes, IpcError> {
        let cmd = DaemonCommand::RenderFrame {
            composition_id,
            frame,
            width,
            height,
            fps,
            props,
        };
        let packet = self.send_request(cmd).await?;
        let expected_len = (width as usize) * (height as usize) * 4;
        if packet.payload.len() != expected_len {
            return Err(IpcError::Protocol(format!(
                "Invalid RGBA payload length: expected {expected_len}, got {}",
                packet.payload.len()
            )));
        }
        Ok(packet.payload)
    }
}
```

---

## 7. Error Code Signaling & Status Codes

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum IpcStatusCode {
    Ok = 0,
    GenericError = 1,
    InvalidRequest = 2,
    CompositionNotFound = 3,
    FrameOutOfBounds = 4,
    RasterError = 5,
    Timeout = 6,
    Cancelled = 7,
    CacheError = 8,
    InternalError = 9,
}

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Header parse error: {0}")]
    HeaderParseError(String),

    #[error("Payload length {len} exceeds maximum of {max} bytes")]
    PayloadTooLarge { len: usize, max: usize },

    #[error("Response error (status {status}): {message}")]
    ResponseError { status: u32, message: String },

    #[error("Request timed out for nonce {0}")]
    RequestTimeout(u64),

    #[error("IPC connection closed")]
    ConnectionClosed,

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}
```

---

## 8. Async I/O Integration: CLI Subcommand & Daemon Server

### 8.1 CLI Subcommand (`crates/cli`)

Extend `crates/cli/src/lib.rs`:
```rust
#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum Commands {
    /// Render a registered Rust composition or Rhai script to a media file.
    Render { ... },

    /// Run the persistent compositor daemon process with binary IPC protocol.
    Daemon {
        /// Standard I/O mode (reads binary packets from stdin, writes to stdout).
        #[arg(long, default_value_t = true)]
        stdio: bool,

        /// Path to Unix domain socket (Unix) or named pipe (Windows).
        #[arg(long)]
        socket: Option<PathBuf>,

        /// TCP port to bind IPC listener (e.g. 9510).
        #[arg(long)]
        port: Option<u16>,

        /// Maximum frame cache size in megabytes.
        #[arg(long, default_value_t = 512)]
        cache_size_mb: usize,

        /// Number of parallel worker threads.
        #[arg(long)]
        concurrency: Option<usize>,
    },
}
```

### 8.2 Standard I/O Isolation Rule

When running in `stdio` mode (`dioxuscut daemon --stdio`), **all stdout writes must be strictly reserved for binary IPC packets**.
All tracing logs (`tracing_subscriber`) must be configured to emit to `std::io::stderr`:
```rust
tracing_subscriber::fmt()
    .with_writer(std::io::stderr)
    .with_env_filter("info,dioxuscut_renderer=debug")
    .init();
```
This guarantees that stdout is a clean, unpolluted binary pipe.

### 8.3 Daemon Request Dispatcher Engine

```rust
use dioxuscut_composition::CompositionRegistry;
use dioxuscut_rasterizer::{FrameCacheManager, TinySkiaBackend, FrameConfig};
use std::sync::Arc;

pub struct DaemonEngine {
    registry: Arc<CompositionRegistry>,
    cache: Arc<FrameCacheManager>,
    backend: Arc<TinySkiaBackend>,
}

impl DaemonEngine {
    pub async fn handle_packet(&self, packet: BinaryPacket) -> BinaryPacket {
        let command: DaemonCommand = match serde_json::from_slice(&packet.payload) {
            Ok(cmd) => cmd,
            Err(err) => {
                return BinaryPacket {
                    nonce: packet.nonce,
                    status: IpcStatusCode::InvalidRequest as u32,
                    payload: Bytes::from(format!("Invalid command JSON: {err}")),
                };
            }
        };

        match command {
            DaemonCommand::Ping => {
                let res = serde_json::to_vec(&DaemonResponse::Pong).unwrap();
                BinaryPacket {
                    nonce: packet.nonce,
                    status: STATUS_OK,
                    payload: Bytes::from(res),
                }
            }
            DaemonCommand::ListCompositions => {
                let ids = self.registry.ids().into_iter().map(String::from).collect();
                let res = serde_json::to_vec(&DaemonResponse::CompositionsList { ids }).unwrap();
                BinaryPacket {
                    nonce: packet.nonce,
                    status: STATUS_OK,
                    payload: Bytes::from(res),
                }
            }
            DaemonCommand::RenderFrame {
                composition_id,
                frame,
                width,
                height,
                fps,
                props,
            } => {
                // Check FrameCacheManager first (LRU Cache Hit)
                if let Some(cached_rgba) = self.cache.get(&composition_id, frame, width, height) {
                    return BinaryPacket {
                        nonce: packet.nonce,
                        status: STATUS_OK,
                        payload: cached_rgba,
                    };
                }

                // Cache miss: Render frame
                let composition = match self.registry.get(&composition_id) {
                    Ok(c) => c,
                    Err(e) => {
                        return BinaryPacket {
                            nonce: packet.nonce,
                            status: IpcStatusCode::CompositionNotFound as u32,
                            payload: Bytes::from(e.to_string()),
                        };
                    }
                };

                let context = dioxuscut_composition::NativeCompositionContext {
                    width,
                    height,
                    fps,
                    duration_in_frames: 1000,
                };

                let prepared = match composition.prepare(&props, context) {
                    Ok(p) => p,
                    Err(e) => {
                        return BinaryPacket {
                            nonce: packet.nonce,
                            status: IpcStatusCode::RasterError as u32,
                            payload: Bytes::from(e.to_string()),
                        };
                    }
                };

                let scene = match prepared.render(frame) {
                    Ok(s) => s,
                    Err(e) => {
                        return BinaryPacket {
                            nonce: packet.nonce,
                            status: IpcStatusCode::RasterError as u32,
                            payload: Bytes::from(e.to_string()),
                        };
                    }
                };

                let frame_cfg = FrameConfig::new(width, height, frame, fps);
                let rendered_img = match self.backend.render_frame(&scene, &frame_cfg) {
                    Ok(img) => img,
                    Err(e) => {
                        return BinaryPacket {
                            nonce: packet.nonce,
                            status: IpcStatusCode::RasterError as u32,
                            payload: Bytes::from(e.to_string()),
                        };
                    }
                };

                let raw_rgba = Bytes::from(rendered_img.into_raw());
                // Insert into LRU frame cache
                self.cache.insert(
                    &composition_id,
                    frame,
                    width,
                    height,
                    raw_rgba.clone(),
                );

                BinaryPacket {
                    nonce: packet.nonce,
                    status: STATUS_OK,
                    payload: raw_rgba,
                }
            }
            _ => BinaryPacket {
                nonce: packet.nonce,
                status: IpcStatusCode::InvalidRequest as u32,
                payload: Bytes::from("Command not implemented yet"),
            },
        }
    }
}
```

---

## 9. Integration with Studio & Player (`apps/studio`, `crates/player`)

### 9.1 Timeline Filmstrip & Scrubbing Pipeline

1. When Studio launches, it creates a `DaemonClient` connecting to an embedded or child daemon process.
2. When the user scrubs the timeline:
   - UI sends `RenderFrame` request with current timeline frame.
   - If frame is in daemon's `FrameCacheManager`, it responds in <1ms over binary IPC.
   - UI receives raw `Bytes` and updates the preview canvas instantly.
3. When the timeline view zoom/scroll changes:
   - `calculate_timestamp_slots` computes visible thumbnail timestamps (e.g. frames 0, 15, 30, 45).
   - UI spawns async requests `RenderThumbnail` for each missing slot.
   - Each thumbnail arrives with its correlated `nonce`, updating the filmstrip strip asynchronously without any UI frame drops.

---

## 10. Type and Trait Mapping Matrix

| Type / Trait | Crate | Purpose |
|---|---|---|
| `BinaryPacket` | `dioxuscut-renderer::ipc` | Raw framed packet `{ nonce: u64, status: u32, payload: Bytes }`. |
| `BinaryIpcCodec` | `dioxuscut-renderer::ipc` | Tokio `Encoder` + `Decoder` for `remotion_buffer:<nonce>:<len>:<status>:<payload>`. |
| `StreamDecoder` / `StreamEncoder` | `dioxuscut-renderer::ipc` | Streaming chunk parser and packet serializer. |
| `make_streamer` | `dioxuscut-renderer::ipc` | Wraps `AsyncRead + AsyncWrite` into `Framed<T, BinaryIpcCodec>`. |
| `DaemonCommand` / `DaemonResponse` | `dioxuscut-renderer::ipc` | JSON-RPC command set for metadata, cache, and frame requests. |
| `DaemonClient` | `dioxuscut-renderer::ipc` | Client RPC handle with async nonce correlation map. |
| `DaemonServer` / `DaemonEngine` | `dioxuscut-renderer::ipc` | Server request router dispatching to `CompositionRegistry` + `FrameCacheManager`. |
| `IpcStatusCode` / `IpcError` | `dioxuscut-renderer::ipc` | Wire status codes and typed Rust error enum. |

---

## 11. Comprehensive Test Suite Mapping

To ensure 100% test coverage and compliance with all quality gates, the test matrix for Requirement R2 comprises:

### 11.1 Unit Tests (`crates/renderer/src/ipc/tests.rs` or `crates/renderer/tests/`)
1. `test_packet_encode_decode_roundtrip`: Encode packet with known nonce, status, payload; decode back; assert exact equality.
2. `test_chunked_streaming_byte_by_byte`: Feed encoded packet byte-by-byte into `BinaryIpcCodec::decode`; verify packet is produced only when complete.
3. `test_chunked_streaming_arbitrary_splits`: Split a 1000-byte packet at arbitrary offsets (1, 7, 33, 128 bytes); verify lossless reconstruction.
4. `test_packet_coalescence`: Place 5 concatenated packets into a single `BytesMut`; verify decoder returns all 5 in sequence.
5. `test_noise_resynchronization`: Prepend random ASCII/binary garbage before `remotion_buffer:`; verify decoder discards garbage and decodes valid packet.
6. `test_invalid_header_rejection`: Malformed non-numeric nonce/len/status properly handled without panics.
7. `test_payload_too_large_rejection`: Payload exceeding configured `max_payload_bytes` returns `IpcError::PayloadTooLarge`.
8. `test_zero_length_payload`: Packet with `<len> = 0` correctly parsed and emitted with empty `Bytes`.
9. `test_status_codes_and_error_propagation`: Verify error status codes (1..9) and error message extraction.

### 11.2 Integration & Async Tests (`crates/renderer/tests/ipc_integration.rs`)
1. `test_async_duplex_pipe_ping_pong`: Create `tokio::io::duplex` pair; run client and server tasks; verify `Ping` -> `Pong`.
2. `test_concurrent_multiplexed_nonces`: Client fires 50 concurrent requests in parallel tasks with random artificial delays on server; verify 100% of responses map to correct caller.
3. `test_zero_copy_raw_rgba_transfer`: Client requests 1080p frame (8.3 MB); daemon returns synthetic gradient; client verifies pixel contents.
4. `test_daemon_cache_hit_performance`: Repeated frame requests return cached buffers immediately with hit counter increment.
5. `test_request_timeout`: Server does not respond; client timeout fires and cleans up pending nonce.

### 11.3 E2E Subprocess Tests (`crates/cli/tests/e2e_daemon_ipc.rs`)
1. `test_e2e_daemon_stdio_subprocess`: Spawn `dioxuscut daemon --stdio` subprocess; communicate over child stdin/stdout using `DaemonClient`; query composition metadata and frame 0.

---

## 12. Implementation Plan & Architectural Recommendations

1. **Step 1: Workspace Dependencies (`Cargo.toml`)**:
   - Add `bytes = "1"` and `tokio-util = { version = "0.7", features = ["codec"] }` to `[workspace.dependencies]` and `crates/renderer/Cargo.toml`.
2. **Step 2: Binary IPC Protocol Module (`crates/renderer/src/ipc/`)**:
   - Implement `protocol.rs`, `codec.rs`, `client.rs`, `server.rs`, `error.rs`.
   - Export `make_streamer`, `BinaryPacket`, `BinaryIpcCodec`, `DaemonClient`, `DaemonServer`.
3. **Step 3: CLI Subcommand (`crates/cli/src/main.rs`, `crates/cli/src/lib.rs`)**:
   - Add `Commands::Daemon` subcommand with `--stdio`, `--socket`, `--port` flags.
   - Implement daemon server loop connecting `CompositionRegistry` and `FrameCacheManager`.
4. **Step 4: Studio Integration (`apps/studio/src/`)**:
   - Connect Studio preview & timeline thumbnail virtualizer to `DaemonClient`.
5. **Step 5: Full Test Suite Execution**:
   - Verify `cargo check`, `cargo clippy`, `cargo test`, `cargo fmt`.
