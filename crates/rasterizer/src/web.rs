//! Protocol types for browser-backed composition workers.

use serde::{Deserialize, Serialize};

/// Deterministic request sent to a browser-backed renderer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebFrameRequest {
    pub frame: u32,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub props: serde_json::Value,
}

/// Result returned by a browser-backed renderer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebFrameResponse {
    pub frame: u32,
    pub rgba_base64: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebWorkerMessage {
    Ready { protocol: u32 },
    Render(WebFrameRequest),
    Frame(WebFrameResponse),
    Error { frame: Option<u32>, message: String },
    Shutdown,
}

pub const WEB_WORKER_PROTOCOL_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_request_round_trips_as_json() {
        let message = WebWorkerMessage::Render(WebFrameRequest {
            frame: 12,
            fps: 30.0,
            width: 1280,
            height: 720,
            props: serde_json::json!({"seed": 7}),
        });
        let json = serde_json::to_string(&message).unwrap();
        assert_eq!(
            serde_json::from_str::<WebWorkerMessage>(&json).unwrap(),
            message
        );
    }

    #[test]
    fn protocol_ready_message_is_stable() {
        let json = serde_json::to_string(&WebWorkerMessage::Ready {
            protocol: WEB_WORKER_PROTOCOL_VERSION,
        })
        .unwrap();
        assert_eq!(json, r#"{"type":"ready","protocol":1}"#);
    }
}
