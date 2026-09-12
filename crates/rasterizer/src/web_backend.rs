//! Browser-backed frame transport for Three.js and other web compositions.

use crate::backend::{BackendCapabilities, FrameConfig, RasterError, RasterizerBackend};
use crate::frame_cache::{CacheMetrics, FrameCacheKey, FrameCacheManager};
use crate::scene::Scene;
use crate::web::{
    WebFrameRequest, WebFrameResponse, WebTimelineClip, WebWorkerMessage,
    WEB_WORKER_PROTOCOL_VERSION,
};
use base64::Engine;
use image::RgbaImage;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

struct BrowserWorker {
    child: Mutex<Child>,
    compositions: Vec<String>,
    /// A worker processes one request at a time; protect the full
    /// write/read transaction so concurrent host threads cannot steal replies.
    request: Mutex<()>,
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
}

pub struct BrowserFrameBackend {
    workers: Vec<BrowserWorker>,
    next_worker: AtomicUsize,
    composition: Mutex<Option<String>>,
    assets: Mutex<Vec<String>>,
    timeline: Mutex<Vec<WebTimelineClip>>,
    image_format: Option<String>,
    jpeg_quality: Option<u8>,
    props: Mutex<serde_json::Value>,
    cache: FrameCacheManager,
}

impl BrowserFrameBackend {
    pub fn new(
        node: impl AsRef<std::ffi::OsStr>,
        worker: impl AsRef<std::path::Path>,
        url: impl Into<String>,
    ) -> Result<Self, RasterError> {
        Self::with_concurrency(node, worker, url, 1)
    }

    pub fn with_concurrency(
        node: impl AsRef<std::ffi::OsStr>,
        worker: impl AsRef<std::path::Path>,
        url: impl Into<String>,
        concurrency: usize,
    ) -> Result<Self, RasterError> {
        if concurrency == 0 {
            return Err(RasterError::Init(
                "browser worker concurrency must be greater than zero".into(),
            ));
        }
        let url = url.into();
        let mut workers = Vec::with_capacity(concurrency);
        for _ in 0..concurrency {
            workers.push(BrowserWorker::spawn(&node, worker.as_ref(), &url)?);
        }
        Ok(Self {
            workers,
            next_worker: AtomicUsize::new(0),
            composition: Mutex::new(None),
            assets: Mutex::new(
                std::env::var_os("DIOXUSCUT_BROWSER_ASSETS")
                    .map(|value| {
                        std::env::split_paths(&value)
                            .map(|path| path.to_string_lossy().into_owned())
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
            timeline: Mutex::new(
                std::env::var("DIOXUSCUT_BROWSER_TIMELINE")
                    .ok()
                    .and_then(|value| serde_json::from_str(&value).ok())
                    .unwrap_or_default(),
            ),
            image_format: std::env::var("DIOXUSCUT_BROWSER_IMAGE_FORMAT")
                .ok()
                .filter(|v| v == "png" || v == "jpeg"),
            jpeg_quality: std::env::var("DIOXUSCUT_BROWSER_JPEG_QUALITY")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v: &u8| (1..=100).contains(v)),
            props: Mutex::new(serde_json::json!({})),
            cache: FrameCacheManager::default(),
        })
    }
}

impl BrowserWorker {
    fn spawn(
        node: &impl AsRef<std::ffi::OsStr>,
        worker: &std::path::Path,
        url: &str,
    ) -> Result<Self, RasterError> {
        let mut child = Command::new(node)
            .arg(worker)
            .arg(format!("--url={url}"))
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
        let compositions = match serde_json::from_str::<WebWorkerMessage>(&line) {
            Ok(WebWorkerMessage::Ready {
                protocol,
                compositions,
            }) if protocol == WEB_WORKER_PROTOCOL_VERSION => compositions,
            Ok(WebWorkerMessage::Ready { protocol, .. }) => {
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
        };
        Ok(Self {
            child: Mutex::new(child),
            compositions,
            request: Mutex::new(()),
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(stdout),
        })
    }
}

impl BrowserFrameBackend {
    /// Return composition IDs advertised by the first browser worker.
    pub fn compositions(&self) -> Vec<String> {
        self.workers
            .first()
            .map(|worker| worker.compositions.clone())
            .unwrap_or_default()
    }

    /// Configure assets that browser compositions should preload before frames.
    pub fn set_assets(&self, assets: Vec<String>) -> Result<(), RasterError> {
        *self
            .assets
            .lock()
            .map_err(|_| RasterError::Init("browser assets lock poisoned".into()))? = assets;
        Ok(())
    }

    /// Configure the project timeline forwarded to browser compositions.
    pub fn set_timeline(&self, timeline: Vec<WebTimelineClip>) -> Result<(), RasterError> {
        *self
            .timeline
            .lock()
            .map_err(|_| RasterError::Init("browser timeline lock poisoned".into()))? = timeline;
        Ok(())
    }

    /// Number of persistent browser workers available for frame rendering.
    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// Select the browser-side composition for subsequent frame requests.
    pub fn set_composition(&self, composition: impl Into<String>) -> Result<(), RasterError> {
        *self
            .composition
            .lock()
            .map_err(|_| RasterError::Init("browser composition lock poisoned".into()))? =
            Some(composition.into());
        Ok(())
    }

    pub fn set_props(&self, props: serde_json::Value) -> Result<(), RasterError> {
        *self
            .props
            .lock()
            .map_err(|_| RasterError::Init("browser worker props lock poisoned".into()))? = props;
        Ok(())
    }

    /// Return cache counters for host UIs and performance telemetry.
    pub fn cache_metrics(&self) -> CacheMetrics {
        self.cache.metrics()
    }
    pub fn render_web_frame(&self, request: &WebFrameRequest) -> Result<RgbaImage, RasterError> {
        let composition = self
            .composition
            .lock()
            .map_err(|_| RasterError::Init("browser composition lock poisoned".into()))?
            .clone()
            .unwrap_or_else(|| "browser".into());
        let cache_key = FrameCacheKey::from_props(
            &composition,
            request.frame as u64,
            request.width,
            request.height,
            &request.props,
        );
        if let Some(image) = self.cache.get(&cache_key) {
            return Ok((*image).clone());
        }
        let encoded =
            serde_json::to_string(&WebWorkerMessage::Render(request.clone())).map_err(|e| {
                RasterError::Frame {
                    frame: request.frame,
                    reason: e.to_string(),
                }
            })?;
        let worker =
            &self.workers[self.next_worker.fetch_add(1, Ordering::Relaxed) % self.workers.len()];
        let _request = worker
            .request
            .lock()
            .map_err(|_| RasterError::Init("browser worker request lock poisoned".into()))?;
        let mut stdin = worker
            .stdin
            .lock()
            .map_err(|_| RasterError::Init("browser worker stdin lock poisoned".into()))?;
        writeln!(stdin, "{encoded}")?;
        stdin.flush()?;
        drop(stdin);
        tracing::debug!(frame = request.frame, "browser frame request sent");
        let mut stdout = worker
            .stdout
            .lock()
            .map_err(|_| RasterError::Init("browser worker stdout lock poisoned".into()))?;
        let mut line = String::new();
        stdout.read_line(&mut line)?;
        tracing::debug!(
            frame = request.frame,
            bytes = line.len(),
            "browser frame response received"
        );
        let result = match serde_json::from_str::<WebWorkerMessage>(&line) {
            Ok(WebWorkerMessage::Frame(WebFrameResponse {
                frame,
                width,
                height,
                png_base64,
                jpeg_base64,
                rgba_base64,
            })) if frame == request.frame => {
                if let Some(encoded) = png_base64.or(jpeg_base64) {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .map_err(|e| RasterError::Frame {
                            frame,
                            reason: e.to_string(),
                        })?;
                    image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
                        .map(|image| image.into_rgba8())
                        .map_err(|e| RasterError::Frame {
                            frame,
                            reason: e.to_string(),
                        })
                } else if let Some(encoded) = rgba_base64 {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .map_err(|e| RasterError::Frame {
                            frame,
                            reason: e.to_string(),
                        })?;
                    RgbaImage::from_raw(width, height, bytes).ok_or_else(|| RasterError::Frame {
                        frame,
                        reason: "RGBA payload length does not match dimensions".into(),
                    })
                } else {
                    Err(RasterError::Frame {
                        frame,
                        reason: "frame response has no image payload".into(),
                    })
                }
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
        };
        if let Ok(ref image) = result {
            self.cache.insert(cache_key, Arc::new(image.clone()));
        }
        result
    }
}
impl Drop for BrowserFrameBackend {
    fn drop(&mut self) {
        for worker in &self.workers {
            if let Ok(mut stdin) = worker.stdin.lock() {
                let _ = writeln!(stdin, "{{\"type\":\"shutdown\"}}");
                let _ = stdin.flush();
            }
            if let Ok(mut child) = worker.child.lock() {
                let _ = child.try_wait();
                if child.try_wait().ok().flatten().is_none() {
                    let _ = child.kill();
                }
                let _ = child.wait();
            }
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
        let props = self
            .props
            .lock()
            .map_err(|_| RasterError::Init("browser worker props lock poisoned".into()))?
            .clone();
        self.render_web_frame(&WebFrameRequest {
            composition: self
                .composition
                .lock()
                .map_err(|_| RasterError::Init("browser composition lock poisoned".into()))?
                .clone(),
            frame: config.frame,
            fps: config.fps,
            width: config.width,
            height: config.height,
            assets: self
                .assets
                .lock()
                .map_err(|_| RasterError::Init("browser assets lock poisoned".into()))?
                .clone(),
            timeline: self
                .timeline
                .lock()
                .map_err(|_| RasterError::Init("browser timeline lock poisoned".into()))?
                .clone(),
            image_format: self.image_format.clone(),
            jpeg_quality: self.jpeg_quality,
            props,
        })
    }
}
