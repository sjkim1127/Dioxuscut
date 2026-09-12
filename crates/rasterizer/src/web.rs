//! Protocol types for browser-backed composition workers.

use serde::{Deserialize, Serialize};

/// Deterministic request sent to a browser-backed renderer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebTimelineClip {
    pub id: String,
    pub composition: String,
    pub start: u32,
    pub duration: u32,
    #[serde(default)]
    pub props: serde_json::Value,
}

/// Deterministic request sent to a browser-backed renderer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebFrameRequest {
    /// Composition selected by the host. Older workers may ignore this field.
    #[serde(default)]
    pub composition: Option<String>,
    pub frame: u32,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub props: serde_json::Value,
    /// Host-provided local or URL assets that the browser composition may preload.
    #[serde(default)]
    pub assets: Vec<String>,
    #[serde(default)]
    pub timeline: Vec<WebTimelineClip>,
    /// Browser screenshot transport. Defaults to PNG for lossless compatibility.
    #[serde(default)]
    pub image_format: Option<String>,
    #[serde(default)]
    pub jpeg_quality: Option<u8>,
}

/// Result returned by a browser-backed renderer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebFrameResponse {
    pub frame: u32,
    pub width: u32,
    pub height: u32,
    /// Preferred transport: the browser's encoded PNG screenshot.
    #[serde(default)]
    pub png_base64: Option<String>,
    #[serde(default)]
    pub jpeg_base64: Option<String>,
    /// Legacy transport retained for older workers.
    #[serde(default)]
    pub rgba_base64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebWorkerMessage {
    Ready {
        protocol: u32,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        compositions: Vec<String>,
    },
    Render(WebFrameRequest),
    Frame(WebFrameResponse),
    Error {
        frame: Option<u32>,
        message: String,
    },
    Shutdown,
}

pub const WEB_WORKER_PROTOCOL_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_request_round_trips_as_json() {
        let message = WebWorkerMessage::Render(WebFrameRequest {
            composition: Some("demo".into()),
            frame: 12,
            fps: 30.0,
            width: 1280,
            height: 720,
            props: serde_json::json!({"seed": 7}),
            assets: vec![],
            timeline: vec![],
            image_format: None,
            jpeg_quality: None,
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
            compositions: vec![],
        })
        .unwrap();
        assert_eq!(json, r#"{"type":"ready","protocol":1}"#);
    }

    #[test]
    fn frame_response_accepts_legacy_rgba_transport() {
        let message = r#"{"type":"frame","frame":3,"width":1,"height":1,"rgba_base64":"AQIDBA=="}"#;
        let parsed: WebWorkerMessage = serde_json::from_str(message).unwrap();
        assert!(matches!(parsed, WebWorkerMessage::Frame(response)
            if response.png_base64.is_none() && response.rgba_base64.as_deref() == Some("AQIDBA==")));
    }
}
