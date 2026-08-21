//! Remotion binary framing format and protocol data types.
//!
//! Protocol framing wire specification:
//! `remotion_buffer:<nonce>:<len>:<status>:<payload>`
//!
//! - `remotion_buffer:` (16 bytes magic ASCII header prefix)
//! - `<nonce>`: 64-bit unsigned integer ASCII decimal correlation identifier
//! - `<len>`: Payload length in bytes (ASCII decimal)
//! - `<status>`: Execution status code (ASCII decimal, 0 = OK)
//! - `<payload>`: Exactly `<len>` binary bytes

use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// Magic ASCII header prefix separating Remotion binary frames.
pub const BUFFER_PREFIX: &[u8] = b"remotion_buffer:";

/// Standard success status code (0).
pub const STATUS_OK: u32 = 0;

/// Default maximum payload size (128 MB) to prevent unbounded memory allocation.
pub const DEFAULT_MAX_PAYLOAD_BYTES: usize = 128 * 1024 * 1024;

/// A framed binary packet in the Remotion IPC protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryPacket {
    /// Monotonic request/response correlation identifier.
    pub nonce: u64,
    /// Execution status code (0 = success, non-zero = error).
    pub status: u32,
    /// Zero-copy byte slice containing raw RGBA pixel data or JSON payload.
    pub payload: Bytes,
}

impl BinaryPacket {
    /// Create a new binary packet.
    pub fn new(nonce: u64, status: u32, payload: impl Into<Bytes>) -> Self {
        Self {
            nonce,
            status,
            payload: payload.into(),
        }
    }

    /// Create a successful response packet with status `0` (`STATUS_OK`).
    pub fn ok(nonce: u64, payload: impl Into<Bytes>) -> Self {
        Self::new(nonce, STATUS_OK, payload)
    }

    /// Create an error packet with a given status code and UTF-8 error message payload.
    pub fn err(nonce: u64, status: u32, message: impl Into<String>) -> Self {
        let msg = message.into();
        Self::new(nonce, status, Bytes::from(msg.into_bytes()))
    }

    /// Check if the packet represents a successful response (`status == 0`).
    pub fn is_ok(&self) -> bool {
        self.status == STATUS_OK
    }

    /// Check if the packet represents an error (`status != 0`).
    pub fn is_err(&self) -> bool {
        self.status != STATUS_OK
    }

    /// Attempt to view the payload as a UTF-8 string.
    pub fn payload_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.payload)
    }

    /// Attempt to deserialize the payload from JSON.
    pub fn json_payload<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.payload)
    }

    /// Construct a packet from a JSON-serializable value.
    pub fn from_json<T: Serialize>(nonce: u64, status: u32, val: &T) -> Result<Self, serde_json::Error> {
        let bytes = serde_json::to_vec(val)?;
        Ok(Self::new(nonce, status, Bytes::from(bytes)))
    }
}

/// Commands issued from client/UI to the compositor daemon process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum DaemonCommand {
    /// Liveness heartbeat check.
    Ping,
    /// Retrieve metadata for a registered composition.
    GetMetadata {
        /// Unique composition identifier.
        composition_id: String,
    },
    /// List all available registered composition IDs.
    ListCompositions,
    /// Render a single frame to raw RGBA pixel buffer.
    RenderFrame {
        /// Unique composition identifier.
        composition_id: String,
        /// Target frame number.
        frame: u32,
        /// Frame pixel width.
        width: u32,
        /// Frame pixel height.
        height: u32,
        /// Frames per second.
        fps: f64,
        /// Dynamic composition input properties.
        #[serde(default)]
        props: serde_json::Value,
    },
    /// Render a thumbnail frame with downscaling.
    RenderThumbnail {
        /// Unique composition identifier.
        composition_id: String,
        /// Target frame number.
        frame: u32,
        /// Thumbnail pixel width.
        width: u32,
        /// Thumbnail pixel height.
        height: u32,
        /// Frames per second.
        fps: f64,
        /// Dynamic composition input properties.
        #[serde(default)]
        props: serde_json::Value,
    },
    /// Query LRU frame cache metrics (hits, misses, bytes).
    GetCacheStats,
    /// Clear all cached frames in daemon memory.
    ClearCache,
    /// Gracefully shutdown the compositor daemon.
    Shutdown,
}

/// Responses emitted by the compositor daemon for structured commands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum DaemonResponse {
    /// Heartbeat response.
    Pong,
    /// List of registered composition identifiers.
    CompositionsList {
        /// IDs of registered compositions.
        ids: Vec<String>,
    },
    /// Metadata for a registered composition.
    CompositionMetadata {
        /// Composition ID.
        id: String,
        /// Canvas width in pixels.
        width: u32,
        /// Canvas height in pixels.
        height: u32,
        /// Framerate.
        fps: f64,
        /// Total duration in frames.
        duration_in_frames: u32,
    },
    /// Frame cache statistics.
    CacheStats {
        /// Number of frame buffers in memory.
        cached_frames: usize,
        /// Total bytes occupied by cached frames.
        cached_bytes: usize,
        /// Total cache hit queries.
        hits: u64,
        /// Total cache miss queries.
        misses: u64,
    },
    /// Generic success response.
    Success,
    /// Explicit error response with error code and description.
    Error {
        /// Error status code.
        code: u32,
        /// Error message.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_packet_constructors() {
        let p_ok = BinaryPacket::ok(42, Bytes::from_static(b"hello"));
        assert_eq!(p_ok.nonce, 42);
        assert_eq!(p_ok.status, STATUS_OK);
        assert!(p_ok.is_ok());
        assert!(!p_ok.is_err());
        assert_eq!(p_ok.payload_str().unwrap(), "hello");

        let p_err = BinaryPacket::err(101, 3, "Composition not found");
        assert_eq!(p_err.nonce, 101);
        assert_eq!(p_err.status, 3);
        assert!(p_err.is_err());
        assert_eq!(p_err.payload_str().unwrap(), "Composition not found");
    }

    #[test]
    fn test_daemon_command_response_serde() {
        let cmd = DaemonCommand::RenderFrame {
            composition_id: "Comp1".into(),
            frame: 10,
            width: 1920,
            height: 1080,
            fps: 30.0,
            props: serde_json::json!({ "text": "Hello" }),
        };

        let packet = BinaryPacket::from_json(1, STATUS_OK, &cmd).unwrap();
        let decoded_cmd: DaemonCommand = packet.json_payload().unwrap();
        assert_eq!(cmd, decoded_cmd);

        let resp = DaemonResponse::Pong;
        let resp_packet = BinaryPacket::from_json(2, STATUS_OK, &resp).unwrap();
        let decoded_resp: DaemonResponse = resp_packet.json_payload().unwrap();
        assert_eq!(resp, decoded_resp);
    }
}
