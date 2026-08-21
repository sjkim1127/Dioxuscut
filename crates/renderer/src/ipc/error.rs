//! IPC error types and status codes for the Remotion binary IPC protocol.

use thiserror::Error;

/// Standard status codes for Remotion binary IPC packets.
///
/// A status of `0` indicates success (`STATUS_OK`), while non-zero values represent
/// specific error categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum IpcStatusCode {
    /// Request succeeded without errors.
    Ok = 0,
    /// Generic or unclassified error.
    GenericError = 1,
    /// Invalid request payload or malformed command.
    InvalidRequest = 2,
    /// Requested composition ID was not found in registry.
    CompositionNotFound = 3,
    /// Requested frame index is outside composition bounds.
    FrameOutOfBounds = 4,
    /// Rasterization or rendering error occurred.
    RasterError = 5,
    /// Request timed out waiting for execution.
    Timeout = 6,
    /// Request was cancelled before completion.
    Cancelled = 7,
    /// Cache lookup or insertion error.
    CacheError = 8,
    /// Internal daemon server error.
    InternalError = 9,
}

impl IpcStatusCode {
    /// Convert status code to its numeric `u32` representation.
    pub const fn as_u32(&self) -> u32 {
        *self as u32
    }

    /// Check if this status represents success (`0`).
    pub const fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }

    /// Parse a `u32` status code into its corresponding enum variant.
    /// Unrecognized values default to `IpcStatusCode::GenericError`.
    pub const fn from_u32(code: u32) -> Self {
        match code {
            0 => Self::Ok,
            1 => Self::GenericError,
            2 => Self::InvalidRequest,
            3 => Self::CompositionNotFound,
            4 => Self::FrameOutOfBounds,
            5 => Self::RasterError,
            6 => Self::Timeout,
            7 => Self::Cancelled,
            8 => Self::CacheError,
            _ => Self::InternalError,
        }
    }
}

impl From<u32> for IpcStatusCode {
    fn from(code: u32) -> Self {
        Self::from_u32(code)
    }
}

impl From<IpcStatusCode> for u32 {
    fn from(status: IpcStatusCode) -> Self {
        status.as_u32()
    }
}

/// Errors that can occur during binary IPC framing, streaming, encoding, decoding, or RPC.
#[derive(Debug, Error)]
pub enum IpcError {
    /// Underlying I/O stream error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Protocol framing or sequencing violation.
    #[error("Protocol error: {0}")]
    Protocol(String),

    /// Header format error (e.g. invalid UTF-8, missing colons, non-numeric fields).
    #[error("Header parse error: {0}")]
    HeaderParseError(String),

    /// Payload byte length exceeded the configured maximum limit.
    #[error("Payload length {len} exceeds maximum allowed of {max} bytes")]
    PayloadTooLarge {
        /// Actual payload length in bytes.
        len: usize,
        /// Maximum allowed payload length in bytes.
        max: usize,
    },

    /// Response returned a non-zero error status code.
    #[error("Response error (status {status}): {message}")]
    ResponseError {
        /// Error status code.
        status: u32,
        /// Description of the error.
        message: String,
    },

    /// Request timed out while waiting for the daemon response.
    #[error("Request timed out for nonce {0}")]
    RequestTimeout(u64),

    /// IPC connection or channel was closed unexpectedly.
    #[error("IPC connection closed")]
    ConnectionClosed,

    /// JSON serialization or deserialization error.
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    /// Internal error in the IPC runtime or worker pool.
    #[error("Internal IPC error: {0}")]
    Internal(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_status_code_conversions() {
        assert_eq!(IpcStatusCode::Ok.as_u32(), 0);
        assert!(IpcStatusCode::Ok.is_ok());
        assert_eq!(IpcStatusCode::from_u32(0), IpcStatusCode::Ok);
        assert_eq!(IpcStatusCode::from_u32(3), IpcStatusCode::CompositionNotFound);
        assert_eq!(IpcStatusCode::from_u32(999), IpcStatusCode::InternalError);

        let code_u32: u32 = IpcStatusCode::RasterError.into();
        assert_eq!(code_u32, 5);
    }

    #[test]
    fn test_ipc_error_display() {
        let err = IpcError::PayloadTooLarge { len: 500, max: 100 };
        assert!(err.to_string().contains("500"));
        assert!(err.to_string().contains("100"));

        let resp_err = IpcError::ResponseError {
            status: 3,
            message: "Not found".into(),
        };
        assert_eq!(resp_err.to_string(), "Response error (status 3): Not found");
    }
}
