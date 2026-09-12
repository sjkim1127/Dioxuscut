//! Browser-backed frame transport for Three.js and other web compositions.

use crate::backend::{BackendCapabilities, FrameConfig, RasterError, RasterizerBackend};
use crate::frame_cache::{CacheMetrics, FrameCacheConfig, FrameCacheKey, FrameCacheManager};
use crate::scene::Scene;
use crate::web::{
    WebFrameRequest, WebFrameResponse, WebTimelineClip, WebWorkerMessage,
    WEB_WORKER_PROTOCOL_VERSION,
};
use base64::Engine;
use image::RgbaImage;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

struct BrowserWorker {
    process: Mutex<WorkerProcess>,
    node: OsString,
    worker: PathBuf,
    url: String,
    timeout: Duration,
    compositions: Vec<String>,
    /// A worker processes one request at a time; protect the full
    /// write/read transaction so concurrent host threads cannot steal replies.
    request: Mutex<()>,
}

struct WorkerProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: Receiver<std::io::Result<String>>,
}

pub struct BrowserFrameBackend {
    workers: Vec<BrowserWorker>,
    next_worker: AtomicUsize,
    composition: Mutex<Option<String>>,
    assets: Mutex<Vec<String>>,
    timeline: Mutex<Vec<WebTimelineClip>>,
    image_format: Option<String>,
    jpeg_quality: Option<u8>,
    transparent: bool,
    transport_retries: usize,
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
            transparent: std::env::var("DIOXUSCUT_BROWSER_TRANSPARENT")
                .ok()
                .is_some_and(|value| {
                    matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes"
                    )
                }),
            transport_retries: 1,
            props: Mutex::new(serde_json::json!({})),
            cache: FrameCacheManager::default(),
        })
    }
}

impl BrowserWorker {
    fn spawn(
        node: &impl AsRef<std::ffi::OsStr>,
        worker: &Path,
        url: &str,
    ) -> Result<Self, RasterError> {
        let node = node.as_ref().to_os_string();
        let worker = worker.to_path_buf();
        let url = url.to_string();
        let timeout = Duration::from_millis(
            std::env::var("DIOXUSCUT_BROWSER_FRAME_TIMEOUT_MS")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|value: &u64| *value > 0)
                .unwrap_or(30_000),
        );
        let (process, compositions) = Self::spawn_process(&node, &worker, &url)?;
        Ok(Self {
            process: Mutex::new(process),
            node,
            worker,
            url,
            timeout,
            compositions,
            request: Mutex::new(()),
        })
    }

    fn spawn_process(
        node: &OsString,
        worker: &Path,
        url: &str,
    ) -> Result<(WorkerProcess, Vec<String>), RasterError> {
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
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| RasterError::Init("browser worker stdout unavailable".into()))?;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let done = line.is_err();
                if sender.send(line).is_err() || done {
                    break;
                }
            }
        });
        let line = receiver
            .recv_timeout(Duration::from_secs(30))
            .map_err(|error| {
                RasterError::Init(format!("browser worker handshake timeout: {error}"))
            })??;
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
        Ok((
            WorkerProcess {
                child,
                stdin,
                stdout: receiver,
            },
            compositions,
        ))
    }

    fn restart(&self) -> Result<(), RasterError> {
        let mut process = self
            .process
            .lock()
            .map_err(|_| RasterError::Init("browser worker process lock poisoned".into()))?;
        let _ = process.child.kill();
        let _ = process.child.wait();
        let (replacement, _) = Self::spawn_process(&self.node, &self.worker, &self.url)?;
        *process = replacement;
        Ok(())
    }

    fn request_line(&self, encoded: &str) -> Result<String, RasterError> {
        let mut process = self
            .process
            .lock()
            .map_err(|_| RasterError::Init("browser worker process lock poisoned".into()))?;
        writeln!(process.stdin, "{encoded}")?;
        process.stdin.flush()?;
        match process.stdout.recv_timeout(self.timeout) {
            Ok(line) => Ok(line?),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(RasterError::Frame {
                frame: 0,
                reason: format!(
                    "browser worker response timed out after {}ms",
                    self.timeout.as_millis()
                ),
            }),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(RasterError::Frame {
                frame: 0,
                reason: "browser worker exited without a response".into(),
            }),
        }
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

    /// Configure alpha-preserving PNG screenshots for this browser backend.
    ///
    /// This is equivalent to setting `DIOXUSCUT_BROWSER_TRANSPARENT=1`, but is
    /// preferable for embedders that do not use process-wide environment state.
    pub fn with_transparent(mut self, transparent: bool) -> Self {
        self.transparent = transparent;
        self
    }

    /// Configure the browser screenshot format for this backend.
    ///
    /// `png` preserves lossless output and alpha; `jpeg` can reduce transport
    /// time and memory for opaque video previews. Invalid values are ignored
    /// and leave the current format unchanged.
    pub fn with_image_format(mut self, format: impl Into<String>) -> Self {
        let format = format.into().to_ascii_lowercase();
        if matches!(format.as_str(), "png" | "jpeg") {
            self.image_format = Some(format);
        }
        self
    }

    /// Configure JPEG quality for this backend (1..=100).
    pub fn with_jpeg_quality(mut self, quality: u8) -> Self {
        if (1..=100).contains(&quality) {
            self.jpeg_quality = Some(quality);
        }
        self
    }

    /// Configure how many times a failed browser transport request is retried.
    /// A retry restarts the affected worker before resending the frame.
    pub fn with_transport_retries(mut self, retries: usize) -> Self {
        self.transport_retries = retries;
        self
    }

    /// Configure the per-frame browser worker response timeout.
    pub fn with_frame_timeout(mut self, timeout: Duration) -> Self {
        if !timeout.is_zero() {
            for worker in &mut self.workers {
                worker.timeout = timeout;
            }
        }
        self
    }

    /// Configure the decoded browser-frame cache budget in bytes.
    ///
    /// The default is 512 MiB. Embedders can lower it when the browser worker
    /// shares memory with a Tauri or Dioxus application.
    pub fn with_frame_cache_bytes(mut self, max_bytes: usize) -> Self {
        self.cache = FrameCacheManager::new(FrameCacheConfig::with_max_bytes(max_bytes));
        self
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
        let composition = request.composition.clone().or_else(|| {
            self.composition
                .lock()
                .ok()
                .and_then(|composition| composition.clone())
        });
        let cache_inputs = serde_json::json!({
            "props": &request.props,
            "assets": &request.assets,
            "timeline": &request.timeline,
            "image_format": &request.image_format,
            "jpeg_quality": request.jpeg_quality,
            "transparent": request.transparent,
        });
        let cache_key = FrameCacheKey::from_props(
            composition.as_deref().unwrap_or("browser"),
            request.frame as u64,
            request.width,
            request.height,
            &cache_inputs,
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
        let mut line = None;
        let mut last_error = None;
        for attempt in 0..=self.transport_retries {
            match worker.request_line(&encoded) {
                Ok(response) => {
                    line = Some(response);
                    break;
                }
                Err(error) if attempt < self.transport_retries => {
                    tracing::warn!(
                        frame = request.frame,
                        attempt = attempt + 1,
                        error = %error,
                        "browser worker transport failed; restarting"
                    );
                    worker
                        .restart()
                        .map_err(|restart_error| RasterError::Frame {
                            frame: request.frame,
                            reason: format!("{error}; worker restart failed: {restart_error}"),
                        })?;
                    last_error = Some(error);
                }
                Err(error) => last_error = Some(error),
            }
        }
        let line = line.ok_or_else(|| RasterError::Frame {
            frame: request.frame,
            reason: format!(
                "browser worker transport failed: {}",
                last_error.expect("retry loop records an error")
            ),
        })?;
        tracing::debug!(frame = request.frame, "browser frame request sent");
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
                if let Some(encoded) = png_base64 {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .map_err(|e| RasterError::Frame {
                            frame,
                            reason: e.to_string(),
                        })?;
                    let image =
                        image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
                            .map_err(|e| RasterError::Frame {
                                frame,
                                reason: e.to_string(),
                            })?;
                    if image.width() != width || image.height() != height {
                        return Err(RasterError::Frame {
                            frame,
                            reason: format!(
                                "image payload dimensions {}x{} do not match response {}x{}",
                                image.width(),
                                image.height(),
                                width,
                                height
                            ),
                        });
                    }
                    Ok(image.into_rgba8())
                } else if let Some(encoded) = jpeg_base64 {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .map_err(|e| RasterError::Frame {
                            frame,
                            reason: e.to_string(),
                        })?;
                    let image =
                        image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg)
                            .map_err(|e| RasterError::Frame {
                                frame,
                                reason: e.to_string(),
                            })?;
                    if image.width() != width || image.height() != height {
                        return Err(RasterError::Frame {
                            frame,
                            reason: format!(
                                "image payload dimensions {}x{} do not match response {}x{}",
                                image.width(),
                                image.height(),
                                width,
                                height
                            ),
                        });
                    }
                    Ok(image.into_rgba8())
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
            if let Ok(mut process) = worker.process.lock() {
                let _ = writeln!(process.stdin, "{{\"type\":\"shutdown\"}}");
                let _ = process.stdin.flush();
                let _ = process.child.try_wait();
                if process.child.try_wait().ok().flatten().is_none() {
                    let _ = process.child.kill();
                }
                let _ = process.child.wait();
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
            transparent: self.transparent,
            props,
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn transport_failure_restarts_worker_and_retries_frame() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dioxuscut-browser-restart-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        let marker = root.join("first-request-seen");
        let script = root.join("worker.sh");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' '{{\"type\":\"ready\",\"protocol\":1}}'\nread request || exit 0\nif [ ! -f '{marker}' ]; then touch '{marker}'; exit 0; fi\nprintf '%s\\n' '{{\"type\":\"frame\",\"frame\":3,\"width\":1,\"height\":1,\"rgba_base64\":\"AQIDBA==\"}}'\n",
                marker = marker.display()
            ),
        )
        .unwrap();

        let backend = BrowserFrameBackend::new("/bin/sh", &script, "http://unused")
            .unwrap()
            .with_image_format("JPEG")
            .with_jpeg_quality(80)
            .with_transport_retries(2)
            .with_frame_timeout(Duration::from_millis(500))
            .with_transparent(true);
        assert_eq!(backend.image_format.as_deref(), Some("jpeg"));
        assert_eq!(backend.jpeg_quality, Some(80));
        assert_eq!(backend.transport_retries, 2);
        assert_eq!(backend.workers[0].timeout, Duration::from_millis(500));
        assert!(backend.transparent);
        let image = backend
            .render_web_frame(&WebFrameRequest {
                composition: Some("test".into()),
                frame: 3,
                fps: 30.0,
                width: 1,
                height: 1,
                props: serde_json::json!({}),
                assets: vec![],
                timeline: vec![],
                image_format: None,
                jpeg_quality: None,
                transparent: false,
            })
            .unwrap();
        assert_eq!(image.as_raw(), &[1, 2, 3, 4]);
        drop(backend);
        let _ = fs::remove_dir_all(root);
    }
}
