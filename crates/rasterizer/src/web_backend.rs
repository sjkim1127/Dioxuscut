//! Browser-backed frame transport for Three.js and other web compositions.

use crate::backend::{BackendCapabilities, FrameConfig, RasterError, RasterizerBackend};
use crate::scene::Scene;
use crate::web::{
    WebFrameRequest, WebFrameResponse, WebWorkerMessage, WEB_WORKER_PROTOCOL_VERSION,
};
use base64::Engine;
use image::RgbaImage;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

pub struct BrowserFrameBackend {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
}

impl BrowserFrameBackend {
    pub fn new(
        node: impl AsRef<std::ffi::OsStr>,
        worker: impl AsRef<std::path::Path>,
        url: impl Into<String>,
    ) -> Result<Self, RasterError> {
        let mut child = Command::new(node)
            .arg(worker.as_ref())
            .arg(format!("--url={}", url.into()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| RasterError::Init("browser worker stdin unavailable".into()))?;
        let mut stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| RasterError::Init("browser worker stdout unavailable".into()))?,
        );
        let mut line = String::new();
        stdout.read_line(&mut line)?;
        match serde_json::from_str::<WebWorkerMessage>(&line) {
            Ok(WebWorkerMessage::Ready { protocol }) if protocol == WEB_WORKER_PROTOCOL_VERSION => {
            }
            Ok(WebWorkerMessage::Ready { protocol }) => {
                return Err(RasterError::Init(format!(
                    "unsupported browser worker protocol {protocol}"
                )))
            }
            Ok(_) => {
                return Err(RasterError::Init(
                    "browser worker did not become ready".into(),
                ))
            }
            Err(error) => {
                return Err(RasterError::Init(format!(
                    "invalid browser worker handshake: {error}"
                )))
            }
        }
        Ok(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(stdout),
        })
    }
    pub fn render_web_frame(&self, request: &WebFrameRequest) -> Result<RgbaImage, RasterError> {
        let encoded =
            serde_json::to_string(&WebWorkerMessage::Render(request.clone())).map_err(|e| {
                RasterError::Frame {
                    frame: request.frame,
                    reason: e.to_string(),
                }
            })?;
        let mut stdin = self
            .stdin
            .lock()
            .map_err(|_| RasterError::Init("browser worker stdin lock poisoned".into()))?;
        writeln!(stdin, "{encoded}")?;
        stdin.flush()?;
        drop(stdin);
        let mut stdout = self
            .stdout
            .lock()
            .map_err(|_| RasterError::Init("browser worker stdout lock poisoned".into()))?;
        let mut line = String::new();
        stdout.read_line(&mut line)?;
        match serde_json::from_str::<WebWorkerMessage>(&line) {
            Ok(WebWorkerMessage::Frame(WebFrameResponse {
                frame,
                width,
                height,
                rgba_base64,
            })) if frame == request.frame => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(rgba_base64)
                    .map_err(|e| RasterError::Frame {
                        frame,
                        reason: e.to_string(),
                    })?;
                RgbaImage::from_raw(width, height, bytes).ok_or_else(|| RasterError::Frame {
                    frame,
                    reason: "RGBA payload length does not match dimensions".into(),
                })
            }
            Ok(WebWorkerMessage::Error { frame, message }) => Err(RasterError::Frame {
                frame: frame.unwrap_or(request.frame),
                reason: message,
            }),
            Ok(other) => Err(RasterError::Frame {
                frame: request.frame,
                reason: format!("unexpected worker response: {other:?}"),
            }),
            Err(error) => Err(RasterError::Frame {
                frame: request.frame,
                reason: format!("invalid worker response: {error}"),
            }),
        }
    }
}
impl Drop for BrowserFrameBackend {
    fn drop(&mut self) {
        if let Ok(mut stdin) = self.stdin.lock() {
            let _ = writeln!(stdin, "{{\"type\":\"shutdown\"}}");
            let _ = stdin.flush();
        }
        if let Ok(mut child) = self.child.lock() {
            let _ = child.wait();
        }
    }
}
impl RasterizerBackend for BrowserFrameBackend {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            native_scene: false,
            browser_runtime: true,
            gpu_accelerated: true,
            supports_streaming: false,
        }
    }
    fn render_frame(&self, _scene: &Scene, config: &FrameConfig) -> Result<RgbaImage, RasterError> {
        self.render_web_frame(&WebFrameRequest {
            frame: config.frame,
            fps: config.fps,
            width: config.width,
            height: config.height,
            props: serde_json::Value::Null,
        })
    }
}
