//! Host-neutral project and render-job contracts.
//!
//! Dioxus, Tauri, CLI, and Python integrations should exchange these models
//! instead of depending on one another's UI state.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
#[cfg(not(target_arch = "wasm32"))]
use std::io::Read;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub version: u32,
    pub composition: String,
    pub settings: ProjectSettings,
    #[serde(default = "default_props")]
    pub props: serde_json::Value,
    #[serde(default)]
    pub assets: Vec<AssetRef>,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

fn default_props() -> serde_json::Value {
    serde_json::json!({})
}

fn default_frame_step() -> u32 {
    1
}

fn default_scale() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectSettings {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: u32,
    /// Output scale applied after logical composition rendering.
    #[serde(default = "default_scale")]
    pub scale: f64,
    /// Optional FFmpeg quality value; hosts use their default when omitted.
    #[serde(default)]
    pub crf: Option<u32>,
    /// Optional FFmpeg encoder preset.
    #[serde(default)]
    pub preset: Option<String>,
    /// Optional render worker count. `None` lets the host choose automatically.
    #[serde(default)]
    pub concurrency: Option<u32>,
    #[serde(default = "default_frame_step")]
    pub frame_step: u32,
    /// Optional inclusive frame range for automation and partial renders.
    #[serde(default)]
    pub frame_start: Option<u32>,
    #[serde(default)]
    pub frame_end: Option<u32>,
    #[serde(default)]
    pub backend: BackendKind,
    /// Browser worker screenshot format (`png` or `jpeg`).
    #[serde(default)]
    pub browser_image_format: Option<String>,
    /// JPEG quality used by the browser worker.
    #[serde(default)]
    pub browser_jpeg_quality: Option<u8>,
    /// Browser worker response timeout in milliseconds.
    #[serde(default)]
    pub browser_frame_timeout_ms: Option<u64>,
    /// Browser frame transport (`base64` by default or `file`).
    #[serde(default)]
    pub browser_transport: Option<String>,
    /// Number of transport retries for browser worker failures.
    #[serde(default)]
    pub browser_transport_retries: Option<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    #[default]
    Native,
    Browser,
    Gpu,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssetRef {
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub kind: AssetKind,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    #[default]
    Other,
    Image,
    Video,
    Audio,
    Font,
    Lottie,
    Model,
    Shader,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub id: String,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Clip {
    pub id: String,
    pub composition: String,
    pub start: u32,
    pub duration: u32,
    #[serde(default)]
    pub props: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Preparing,
    Rendering,
    Encoding,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenderJob {
    pub id: String,
    pub project: Project,
    pub status: JobStatus,
    pub completed_frames: u32,
    /// Frames accepted by the media encoder. During rendering this may lag
    /// behind `completed_frames` when frame rendering is concurrent.
    #[serde(default)]
    pub encoded_frames: u32,
    pub error: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub gpu_frames: Option<u64>,
    #[serde(default)]
    pub cpu_fallback_frames: Option<u64>,
    #[serde(default)]
    pub gpu_fallback_reason: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProjectError {
    #[error("project composition cannot be empty")]
    EmptyComposition,
    #[error("project dimensions and duration must be greater than zero")]
    InvalidDimensions,
    #[error("project fps must be finite and greater than zero")]
    InvalidFps,
    #[error("track '{track}' contains invalid clip '{clip}': {reason}")]
    InvalidClip {
        track: String,
        clip: String,
        reason: String,
    },
    #[error("invalid project frame range: {start}..={end} for duration {duration}")]
    InvalidFrameRange { start: u32, end: u32, duration: u32 },
    #[error("project frame step must be greater than zero")]
    InvalidFrameStep,
    #[error("project concurrency must be greater than zero")]
    InvalidConcurrency,
    #[error("browser image format must be png or jpeg")]
    InvalidBrowserImageFormat,
    #[error("browser JPEG quality must be between 1 and 100")]
    InvalidBrowserJpegQuality,
    #[error("browser frame timeout must be greater than zero")]
    InvalidBrowserFrameTimeout,
    #[error("browser transport must be base64 or file")]
    InvalidBrowserTransport,
    #[error("project asset id cannot be empty")]
    EmptyAssetId,
    #[error("project asset path cannot be empty for '{0}'")]
    EmptyAssetPath(String),
    #[error("project asset id '{0}' is duplicated")]
    DuplicateAssetId(String),
    #[error("project references unknown asset '{0}'")]
    UnknownAssetReference(String),
    #[error("project asset '{asset}' could not be read: {reason}")]
    AssetRead { asset: String, reason: String },
    #[error("project asset '{0}' resolves outside the project directory")]
    AssetOutsideProject(String),
    #[error("project asset '{asset}' SHA-256 mismatch: expected {expected}, got {actual}")]
    AssetHashMismatch {
        asset: String,
        expected: String,
        actual: String,
    },
    #[error("project asset '{0}' has an invalid SHA-256 digest")]
    InvalidAssetHash(String),
    #[error("project scale must be finite and greater than zero")]
    InvalidScale,
    #[error("invalid render job transition from {from:?} to {to:?}")]
    InvalidJobTransition { from: JobStatus, to: JobStatus },
    #[error("render job '{0}' was not found")]
    JobNotFound(String),
    #[error("render progress cannot regress from {previous} to {next}")]
    ProgressRegressed { previous: u32, next: u32 },
    #[error("unsupported project schema version {0}")]
    UnsupportedVersion(u32),
    #[error("project file error: {0}")]
    File(String),
    #[error("project JSON error: {0}")]
    Json(String),
}

impl Project {
    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.version != 1 {
            return Err(ProjectError::UnsupportedVersion(self.version));
        }
        if self.composition.trim().is_empty() {
            return Err(ProjectError::EmptyComposition);
        }
        if self.settings.width == 0 || self.settings.height == 0 || self.settings.duration == 0 {
            return Err(ProjectError::InvalidDimensions);
        }
        if !self.settings.fps.is_finite() || self.settings.fps <= 0.0 {
            return Err(ProjectError::InvalidFps);
        }
        if !self.settings.scale.is_finite() || self.settings.scale <= 0.0 {
            return Err(ProjectError::InvalidScale);
        }
        if self.settings.frame_step == 0 {
            return Err(ProjectError::InvalidFrameStep);
        }
        if self.settings.concurrency == Some(0) {
            return Err(ProjectError::InvalidConcurrency);
        }
        if let Some(format) = &self.settings.browser_image_format {
            let format = format.trim().to_ascii_lowercase();
            if format != "png" && format != "jpeg" {
                return Err(ProjectError::InvalidBrowserImageFormat);
            }
        }
        if self
            .settings
            .browser_jpeg_quality
            .is_some_and(|quality| !(1..=100).contains(&quality))
        {
            return Err(ProjectError::InvalidBrowserJpegQuality);
        }
        if self
            .settings
            .browser_frame_timeout_ms
            .is_some_and(|timeout| timeout == 0)
        {
            return Err(ProjectError::InvalidBrowserFrameTimeout);
        }
        if self
            .settings
            .browser_transport
            .as_deref()
            .is_some_and(|transport| {
                !matches!(
                    transport.trim().to_ascii_lowercase().as_str(),
                    "base64" | "file"
                )
            })
        {
            return Err(ProjectError::InvalidBrowserTransport);
        }
        if let Some(end) = self.settings.frame_end {
            let start = self.settings.frame_start.unwrap_or(0);
            if start > end || end >= self.settings.duration {
                return Err(ProjectError::InvalidFrameRange {
                    start,
                    end,
                    duration: self.settings.duration,
                });
            }
        }
        let mut asset_ids = BTreeSet::new();
        for asset in &self.assets {
            if asset.id.trim().is_empty() {
                return Err(ProjectError::EmptyAssetId);
            }
            if asset.path.trim().is_empty() {
                return Err(ProjectError::EmptyAssetPath(asset.id.clone()));
            }
            if let Some(hash) = &asset.sha256 {
                if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(ProjectError::InvalidAssetHash(asset.id.clone()));
                }
            }
            if !asset_ids.insert(asset.id.as_str()) {
                return Err(ProjectError::DuplicateAssetId(asset.id.clone()));
            }
        }
        for track in &self.tracks {
            for clip in &track.clips {
                let reason = if clip.composition.trim().is_empty() {
                    Some("composition cannot be empty")
                } else if clip.duration == 0 {
                    Some("duration must be greater than zero")
                } else if clip
                    .start
                    .checked_add(clip.duration)
                    .is_none_or(|end| end > self.settings.duration)
                {
                    Some("clip range must fit within the project duration")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    return Err(ProjectError::InvalidClip {
                        track: track.id.clone(),
                        clip: clip.id.clone(),
                        reason: reason.into(),
                    });
                }
                validate_asset_references(&clip.props, &asset_ids)?;
            }
        }
        validate_asset_references(&self.props, &asset_ids)?;
        Ok(())
    }

    pub fn from_json_str(source: &str) -> Result<Self, ProjectError> {
        let project: Self =
            serde_json::from_str(source).map_err(|error| ProjectError::Json(error.to_string()))?;
        project.validate()?;
        Ok(project)
    }

    /// Validate local manifest assets relative to the project file directory.
    /// Remote URLs are intentionally left to the browser backend.
    pub fn validate_asset_files(
        &self,
        base_dir: impl AsRef<std::path::Path>,
    ) -> Result<(), ProjectError> {
        let base_dir =
            base_dir
                .as_ref()
                .canonicalize()
                .map_err(|error| ProjectError::AssetRead {
                    asset: "<project>".into(),
                    reason: error.to_string(),
                })?;
        for asset in &self.assets {
            if asset.path.contains("://") || asset.path.starts_with("data:") {
                continue;
            }
            let path = base_dir.join(&asset.path);
            let canonical = path
                .canonicalize()
                .map_err(|error| ProjectError::AssetRead {
                    asset: asset.id.clone(),
                    reason: format!("{} ({})", error, path.display()),
                })?;
            if !canonical.starts_with(&base_dir) {
                return Err(ProjectError::AssetOutsideProject(asset.id.clone()));
            }
            let bytes = std::fs::read(&canonical).map_err(|error| ProjectError::AssetRead {
                asset: asset.id.clone(),
                reason: format!("{} ({})", error, canonical.display()),
            })?;
            if let Some(expected) = &asset.sha256 {
                let actual = format!("{:x}", Sha256::digest(bytes));
                if !expected.eq_ignore_ascii_case(&actual) {
                    return Err(ProjectError::AssetHashMismatch {
                        asset: asset.id.clone(),
                        expected: expected.clone(),
                        actual,
                    });
                }
            }
        }
        Ok(())
    }

    /// Resolve local manifest paths and matching prop values from one project directory.
    pub fn resolve_local_asset_paths(&mut self, base_dir: impl AsRef<std::path::Path>) {
        let base_dir = base_dir.as_ref();
        let replacements: Vec<(String, String)> = self
            .assets
            .iter_mut()
            .flat_map(|asset| {
                let reference = format!("asset://{}", asset.id);
                let original = asset.path.clone();
                let resolved = if original.contains("://") || original.starts_with("data:") {
                    original.clone()
                } else {
                    base_dir.join(&original).to_string_lossy().into_owned()
                };
                asset.path = resolved.clone();
                vec![(original, resolved.clone()), (reference, resolved)]
            })
            .collect();

        fn rewrite(value: &mut serde_json::Value, replacements: &[(String, String)]) {
            match value {
                serde_json::Value::String(text) => {
                    if let Some((_, resolved)) =
                        replacements.iter().find(|(original, _)| original == text)
                    {
                        *text = resolved.clone();
                    }
                }
                serde_json::Value::Array(values) => values
                    .iter_mut()
                    .for_each(|value| rewrite(value, replacements)),
                serde_json::Value::Object(values) => values
                    .values_mut()
                    .for_each(|value| rewrite(value, replacements)),
                serde_json::Value::Null
                | serde_json::Value::Bool(_)
                | serde_json::Value::Number(_) => {}
            }
        }
        rewrite(&mut self.props, &replacements);
        for track in &mut self.tracks {
            for clip in &mut track.clips {
                rewrite(&mut clip.props, &replacements);
            }
        }
    }

    /// Download remote HTTP(S) assets into `cache_dir` and rewrite the project
    /// to use the downloaded local files. This is opt-in so Browser projects
    /// can continue to let Chromium fetch remote media directly.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn materialize_remote_assets(
        &mut self,
        cache_dir: impl AsRef<std::path::Path>,
        max_bytes: usize,
    ) -> Result<(), ProjectError> {
        if max_bytes == 0 {
            return Err(ProjectError::AssetRead {
                asset: "remote".into(),
                reason: "remote asset byte limit must be greater than zero".into(),
            });
        }
        let cache_dir = cache_dir.as_ref();
        std::fs::create_dir_all(cache_dir)
            .map_err(|error| ProjectError::File(error.to_string()))?;
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|error| ProjectError::AssetRead {
                asset: "remote".into(),
                reason: error.to_string(),
            })?;
        let mut replacements = Vec::new();
        for asset in &mut self.assets {
            let source = asset.path.trim();
            if !(source.starts_with("http://") || source.starts_with("https://")) {
                continue;
            }
            let response = client
                .get(source)
                .send()
                .and_then(|response| response.error_for_status())
                .map_err(|error| ProjectError::AssetRead {
                    asset: asset.id.clone(),
                    reason: error.to_string(),
                })?;
            if response
                .content_length()
                .is_some_and(|length| length > max_bytes as u64)
            {
                return Err(ProjectError::AssetRead {
                    asset: asset.id.clone(),
                    reason: format!("remote asset exceeds the {max_bytes} byte limit"),
                });
            }
            let mut bytes = Vec::with_capacity(max_bytes.min(1024 * 1024));
            std::io::Read::read_to_end(
                &mut response.take((max_bytes as u64).saturating_add(1)),
                &mut bytes,
            )
            .map_err(|error| ProjectError::AssetRead {
                asset: asset.id.clone(),
                reason: error.to_string(),
            })?;
            if bytes.len() > max_bytes {
                return Err(ProjectError::AssetRead {
                    asset: asset.id.clone(),
                    reason: format!("remote asset exceeds the {max_bytes} byte limit"),
                });
            }
            let actual = format!("{:x}", Sha256::digest(&bytes));
            if let Some(expected) = &asset.sha256 {
                if !expected.eq_ignore_ascii_case(&actual) {
                    return Err(ProjectError::AssetHashMismatch {
                        asset: asset.id.clone(),
                        expected: expected.clone(),
                        actual,
                    });
                }
            }
            // Use only the digest in the cache filename. Asset IDs are user
            // controlled metadata and must never influence filesystem paths.
            let filename = format!("{actual}.asset");
            let local = cache_dir.join(filename);
            std::fs::write(&local, &bytes).map_err(|error| ProjectError::AssetRead {
                asset: asset.id.clone(),
                reason: error.to_string(),
            })?;
            let reference = format!("asset://{}", asset.id);
            replacements.push((source.to_string(), local.to_string_lossy().into_owned()));
            replacements.push((reference, local.to_string_lossy().into_owned()));
            asset.path = local.to_string_lossy().into_owned();
        }
        fn rewrite(value: &mut serde_json::Value, replacements: &[(String, String)]) {
            match value {
                serde_json::Value::String(text) => {
                    if let Some((_, replacement)) =
                        replacements.iter().find(|(source, _)| source == text)
                    {
                        *text = replacement.clone();
                    }
                }
                serde_json::Value::Array(values) => values
                    .iter_mut()
                    .for_each(|value| rewrite(value, replacements)),
                serde_json::Value::Object(values) => values
                    .values_mut()
                    .for_each(|value| rewrite(value, replacements)),
                serde_json::Value::Null
                | serde_json::Value::Bool(_)
                | serde_json::Value::Number(_) => {}
            }
        }
        rewrite(&mut self.props, &replacements);
        for track in &mut self.tracks {
            for clip in &mut track.clips {
                rewrite(&mut clip.props, &replacements);
            }
        }
        Ok(())
    }

    /// Convert local asset paths back to project-relative paths before saving.
    pub fn relativize_local_asset_paths(&mut self, base_dir: impl AsRef<std::path::Path>) {
        let base_dir = base_dir.as_ref();
        let replacements: Vec<(String, String)> = self
            .assets
            .iter_mut()
            .flat_map(|asset| {
                if asset.path.contains("://") || asset.path.starts_with("data:") {
                    return vec![(asset.path.clone(), format!("asset://{}", asset.id))];
                }
                let path = std::path::Path::new(&asset.path);
                let Some(relative) = path.strip_prefix(base_dir).ok() else {
                    return Vec::new();
                };
                let relative = relative.to_string_lossy().into_owned();
                let resolved = asset.path.clone();
                asset.path = relative.clone();
                vec![(resolved, relative)]
            })
            .collect();

        fn rewrite(value: &mut serde_json::Value, replacements: &[(String, String)]) {
            match value {
                serde_json::Value::String(text) => {
                    if let Some((_resolved, relative)) =
                        replacements.iter().find(|(resolved, _)| resolved == text)
                    {
                        *text = relative.clone();
                    }
                }
                serde_json::Value::Array(values) => values
                    .iter_mut()
                    .for_each(|value| rewrite(value, replacements)),
                serde_json::Value::Object(values) => values
                    .values_mut()
                    .for_each(|value| rewrite(value, replacements)),
                serde_json::Value::Null
                | serde_json::Value::Bool(_)
                | serde_json::Value::Number(_) => {}
            }
        }
        rewrite(&mut self.props, &replacements);
    }

    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, ProjectError> {
        let source =
            std::fs::read_to_string(path).map_err(|error| ProjectError::File(error.to_string()))?;
        Self::from_json_str(&source)
    }

    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), ProjectError> {
        self.validate()?;
        let source = serde_json::to_vec_pretty(self)
            .map_err(|error| ProjectError::Json(error.to_string()))?;
        std::fs::write(path, source).map_err(|error| ProjectError::File(error.to_string()))
    }
}

/// In-memory job registry used by desktop hosts and suitable for replacement
/// by a persistent/server implementation without changing the API contract.
#[derive(Debug, Default)]
pub struct JobStore {
    jobs: BTreeMap<String, RenderJob>,
    next_id: u64,
}

impl JobStore {
    pub fn submit(&mut self, project: Project) -> Result<String, ProjectError> {
        project.validate()?;
        self.next_id += 1;
        let id = format!("job-{}", self.next_id);
        self.jobs.insert(
            id.clone(),
            RenderJob {
                id: id.clone(),
                project,
                status: JobStatus::Queued,
                completed_frames: 0,
                encoded_frames: 0,
                error: None,
                output: None,
                gpu_frames: None,
                cpu_fallback_frames: None,
                gpu_fallback_reason: None,
            },
        );
        Ok(id)
    }
    pub fn get(&self, id: &str) -> Option<&RenderJob> {
        self.jobs.get(id)
    }
    pub fn list(&self) -> Vec<RenderJob> {
        self.jobs.values().cloned().collect()
    }
    pub fn set_output(&mut self, id: &str, output: impl Into<String>) -> Result<(), ProjectError> {
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| ProjectError::JobNotFound(id.to_string()))?;
        job.output = Some(output.into());
        Ok(())
    }

    pub fn set_render_diagnostics(
        &mut self,
        id: &str,
        gpu_frames: u64,
        cpu_fallback_frames: u64,
        fallback_reason: Option<String>,
    ) -> Result<(), ProjectError> {
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| ProjectError::JobNotFound(id.to_string()))?;
        job.gpu_frames = Some(gpu_frames);
        job.cpu_fallback_frames = Some(cpu_fallback_frames);
        job.gpu_fallback_reason = fallback_reason;
        Ok(())
    }

    pub fn set_encoding_progress(
        &mut self,
        id: &str,
        encoded_frames: u32,
    ) -> Result<(), ProjectError> {
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| ProjectError::JobNotFound(id.to_string()))?;
        if !matches!(
            job.status,
            JobStatus::Queued | JobStatus::Preparing | JobStatus::Rendering | JobStatus::Encoding
        ) {
            return Err(ProjectError::InvalidJobTransition {
                from: job.status.clone(),
                to: JobStatus::Encoding,
            });
        }
        if encoded_frames < job.encoded_frames {
            return Err(ProjectError::ProgressRegressed {
                previous: job.encoded_frames,
                next: encoded_frames,
            });
        }
        job.encoded_frames = encoded_frames;
        Ok(())
    }

    /// Advance a successfully rendered job through encoding to completion.
    /// Keeping this transition sequence in the store prevents backend workers
    /// from accidentally omitting or reordering terminal states.
    pub fn complete_render(&mut self, id: &str, completed_frames: u32) -> Result<(), ProjectError> {
        let status = self
            .jobs
            .get(id)
            .ok_or_else(|| ProjectError::JobNotFound(id.to_string()))?
            .status
            .clone();
        if status == JobStatus::Preparing {
            self.try_update(id, JobStatus::Rendering, completed_frames)?;
        } else if status != JobStatus::Rendering {
            return Err(ProjectError::InvalidJobTransition {
                from: status,
                to: JobStatus::Completed,
            });
        }
        self.try_update(id, JobStatus::Encoding, completed_frames)?;
        self.set_encoding_progress(id, completed_frames)?;
        self.try_update(id, JobStatus::Completed, completed_frames)
    }
    pub fn update(&mut self, id: &str, status: JobStatus, completed_frames: u32) -> bool {
        self.try_update(id, status, completed_frames).is_ok()
    }
    pub fn try_update(
        &mut self,
        id: &str,
        status: JobStatus,
        completed_frames: u32,
    ) -> Result<(), ProjectError> {
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| ProjectError::JobNotFound(id.to_string()))?;
        let valid = matches!(
            (&job.status, &status),
            (
                JobStatus::Queued,
                JobStatus::Preparing | JobStatus::Cancelled | JobStatus::Failed
            ) | (
                JobStatus::Preparing,
                JobStatus::Rendering | JobStatus::Cancelled | JobStatus::Failed
            ) | (
                JobStatus::Rendering,
                JobStatus::Rendering
                    | JobStatus::Encoding
                    | JobStatus::Cancelled
                    | JobStatus::Failed
            ) | (
                JobStatus::Encoding,
                JobStatus::Encoding
                    | JobStatus::Completed
                    | JobStatus::Cancelled
                    | JobStatus::Failed
            ) | (JobStatus::Completed, JobStatus::Completed)
                | (JobStatus::Failed, JobStatus::Failed)
                | (JobStatus::Cancelled, JobStatus::Cancelled)
        );
        if !valid {
            return Err(ProjectError::InvalidJobTransition {
                from: job.status.clone(),
                to: status,
            });
        }
        if completed_frames < job.completed_frames {
            return Err(ProjectError::ProgressRegressed {
                previous: job.completed_frames,
                next: completed_frames,
            });
        }
        job.status = status;
        job.completed_frames = completed_frames;
        Ok(())
    }
    pub fn fail(&mut self, id: &str, message: impl Into<String>) -> Result<(), ProjectError> {
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| ProjectError::JobNotFound(id.to_string()))?;
        if !matches!(
            job.status,
            JobStatus::Queued
                | JobStatus::Preparing
                | JobStatus::Rendering
                | JobStatus::Encoding
                | JobStatus::Failed
        ) {
            return Err(ProjectError::InvalidJobTransition {
                from: job.status.clone(),
                to: JobStatus::Failed,
            });
        }
        job.status = JobStatus::Failed;
        job.error = Some(message.into());
        Ok(())
    }

    pub fn cancel(&mut self, id: &str) -> Result<(), ProjectError> {
        let job = self
            .jobs
            .get_mut(id)
            .ok_or_else(|| ProjectError::JobNotFound(id.to_string()))?;
        if !matches!(
            job.status,
            JobStatus::Queued | JobStatus::Preparing | JobStatus::Rendering | JobStatus::Encoding
        ) {
            return Err(ProjectError::InvalidJobTransition {
                from: job.status.clone(),
                to: JobStatus::Cancelled,
            });
        }
        job.status = JobStatus::Cancelled;
        Ok(())
    }

    pub fn retry(&mut self, id: &str) -> Result<String, ProjectError> {
        let project = self
            .jobs
            .get(id)
            .ok_or_else(|| ProjectError::JobNotFound(id.to_string()))?;
        if !matches!(project.status, JobStatus::Failed | JobStatus::Cancelled) {
            return Err(ProjectError::InvalidJobTransition {
                from: project.status.clone(),
                to: JobStatus::Queued,
            });
        }
        self.submit(project.project.clone())
    }
}

fn validate_asset_references(
    value: &serde_json::Value,
    asset_ids: &BTreeSet<&str>,
) -> Result<(), ProjectError> {
    match value {
        serde_json::Value::String(text) if text.starts_with("asset://") => {
            let id = text.trim_start_matches("asset://");
            if id.is_empty() || !asset_ids.contains(id) {
                return Err(ProjectError::UnknownAssetReference(id.to_string()));
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                validate_asset_references(value, asset_ids)?;
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                validate_asset_references(value, asset_ids)?;
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> Project {
        Project {
            version: 1,
            composition: "shorts".into(),
            settings: ProjectSettings {
                width: 1080,
                height: 1920,
                fps: 30.0,
                duration: 60,
                scale: 1.0,
                crf: None,
                preset: None,
                concurrency: None,
                frame_step: 1,
                frame_start: None,
                frame_end: None,
                backend: BackendKind::Native,
                browser_image_format: None,
                browser_jpeg_quality: None,
                browser_frame_timeout_ms: None,
                browser_transport: None,
                browser_transport_retries: None,
            },
            props: serde_json::json!({"title":"hello"}),
            assets: vec![],
            tracks: vec![],
        }
    }
    #[test]
    fn project_json_round_trip() {
        let p = project();
        assert_eq!(
            serde_json::from_str::<Project>(&serde_json::to_string(&p).unwrap()).unwrap(),
            p
        );
    }

    #[test]
    fn project_rejects_ambiguous_asset_manifest() {
        let mut p = project();
        p.assets = vec![
            AssetRef {
                id: "logo".into(),
                path: "logo.png".into(),
                kind: AssetKind::Image,
                sha256: None,
            },
            AssetRef {
                id: "logo".into(),
                path: "other.png".into(),
                kind: AssetKind::Image,
                sha256: None,
            },
        ];
        assert_eq!(
            p.validate(),
            Err(ProjectError::DuplicateAssetId("logo".into()))
        );
        p.assets[1].id.clear();
        assert_eq!(p.validate(), Err(ProjectError::EmptyAssetId));
        p.assets[1].id = "other".into();
        p.assets[1].path.clear();
        assert_eq!(
            p.validate(),
            Err(ProjectError::EmptyAssetPath("other".into()))
        );
        p.assets[1].path = "other.png".into();
        p.assets[1].sha256 = Some("not-a-digest".into());
        assert_eq!(
            p.validate(),
            Err(ProjectError::InvalidAssetHash("other".into()))
        );
        p.assets[1].sha256 = None;
        p.props = serde_json::json!({"src": "asset://missing"});
        assert_eq!(
            p.validate(),
            Err(ProjectError::UnknownAssetReference("missing".into()))
        );
    }

    #[test]
    fn project_asset_files_verify_sha256_relative_to_base_dir() {
        let base =
            std::env::temp_dir().join(format!("dioxuscut-project-assets-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let asset_path = base.join("logo.bin");
        std::fs::write(&asset_path, b"dioxuscut asset").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"dioxuscut asset"));
        let mut p = project();
        p.assets = vec![AssetRef {
            id: "logo".into(),
            path: "logo.bin".into(),
            kind: AssetKind::Image,
            sha256: Some(digest.clone()),
        }];
        assert!(p.validate_asset_files(&base).is_ok());
        p.assets[0].sha256 = Some("00".repeat(32));
        assert!(matches!(
            p.validate_asset_files(&base),
            Err(ProjectError::AssetHashMismatch { .. })
        ));
        p.assets[0].path = "../outside.bin".into();
        std::fs::write(base.join("../outside.bin"), b"outside").unwrap();
        assert_eq!(
            p.validate_asset_files(&base),
            Err(ProjectError::AssetOutsideProject("logo".into()))
        );
        std::fs::remove_file(base.join("../outside.bin")).unwrap();
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn materializes_remote_assets_and_rewrites_clip_props() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let payload = b"remote project asset".to_vec();
        let server_payload = payload.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                server_payload.len()
            )
            .unwrap();
            stream.write_all(&server_payload).unwrap();
        });

        let cache =
            std::env::temp_dir().join(format!("dioxuscut-remote-assets-{}", std::process::id()));
        let url = format!("http://{address}/poster.png");
        let digest = format!("{:x}", Sha256::digest(&payload));
        let mut p = project();
        p.assets = vec![AssetRef {
            id: "poster".into(),
            path: url.clone(),
            kind: AssetKind::Image,
            sha256: Some(digest),
        }];
        p.props = serde_json::json!({"poster": "asset://poster"});
        p.tracks = vec![Track {
            id: "track".into(),
            clips: vec![Clip {
                id: "clip".into(),
                composition: "shorts".into(),
                start: 0,
                duration: 1,
                props: serde_json::json!({"src": url}),
            }],
        }];

        p.materialize_remote_assets(&cache, 1024).unwrap();
        let local = std::path::PathBuf::from(&p.assets[0].path);
        assert_eq!(std::fs::read(&local).unwrap(), payload);
        assert_eq!(p.props["poster"], local.to_string_lossy().as_ref());
        assert_eq!(
            p.tracks[0].clips[0].props["src"],
            local.to_string_lossy().as_ref()
        );
        server.join().unwrap();
        std::fs::remove_dir_all(cache).unwrap();
    }

    #[test]
    fn materializer_rejects_unknown_length_response_over_limit() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 256];
            let _ = stream.read(&mut request);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .unwrap();
            stream.write_all(&[7; 2048]).unwrap();
        });
        let mut p = project();
        p.assets = vec![AssetRef {
            id: "large".into(),
            path: format!("http://{address}/large.bin"),
            kind: AssetKind::Other,
            sha256: None,
        }];
        let cache =
            std::env::temp_dir().join(format!("dioxuscut-remote-limit-{}", std::process::id()));
        assert!(matches!(
            p.materialize_remote_assets(&cache, 1024),
            Err(ProjectError::AssetRead { reason, .. }) if reason.contains("exceeds")
        ));
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(cache);
    }

    #[cfg(unix)]
    #[test]
    fn project_asset_files_reject_symlink_escape() {
        use std::os::unix::fs::symlink;
        let base =
            std::env::temp_dir().join(format!("dioxuscut-project-symlink-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let outside = base.with_file_name(format!("dioxuscut-outside-{}", std::process::id()));
        std::fs::write(&outside, b"outside").unwrap();
        symlink(&outside, base.join("linked.bin")).unwrap();
        let mut p = project();
        p.assets = vec![AssetRef {
            id: "linked".into(),
            path: "linked.bin".into(),
            kind: AssetKind::Other,
            sha256: None,
        }];
        assert_eq!(
            p.validate_asset_files(&base),
            Err(ProjectError::AssetOutsideProject("linked".into()))
        );
        std::fs::remove_file(base.join("linked.bin")).unwrap();
        std::fs::remove_dir_all(base).unwrap();
        std::fs::remove_file(outside).unwrap();
    }

    #[test]
    fn project_resolves_manifest_paths_inside_nested_props() {
        let mut p = project();
        p.assets = vec![
            AssetRef {
                id: "poster".into(),
                path: "assets/poster.png".into(),
                kind: AssetKind::Image,
                sha256: None,
            },
            AssetRef {
                id: "remote".into(),
                path: "https://cdn.example/poster.png".into(),
                kind: AssetKind::Image,
                sha256: None,
            },
        ];
        p.props =
            serde_json::json!({"layers": [{"src": "asset://poster"}, {"src": "asset://remote"}]});
        p.resolve_local_asset_paths("/tmp/project");
        let expected_poster = std::path::Path::new("/tmp/project").join("assets/poster.png");
        assert_eq!(std::path::Path::new(&p.assets[0].path), expected_poster);
        assert_eq!(p.assets[1].path, "https://cdn.example/poster.png");
        assert_eq!(
            std::path::Path::new(p.props["layers"][0]["src"].as_str().unwrap()),
            expected_poster
        );
        assert_eq!(
            p.props["layers"][1]["src"],
            "https://cdn.example/poster.png"
        );
    }
    #[test]
    fn project_json_loader_validates_schema() {
        let mut value = serde_json::to_value(project()).unwrap();
        value["version"] = serde_json::json!(2);
        assert_eq!(
            Project::from_json_str(&value.to_string()),
            Err(ProjectError::UnsupportedVersion(2))
        );
    }

    #[test]
    fn project_json_rejects_unknown_fields() {
        let mut value = serde_json::to_value(project()).unwrap();
        value["unexpected"] = serde_json::json!(true);
        assert!(matches!(
            Project::from_json_str(&value.to_string()),
            Err(ProjectError::Json(_))
        ));
    }

    #[test]
    fn project_rejects_zero_concurrency() {
        let mut value = serde_json::to_value(project()).unwrap();
        value["settings"]["concurrency"] = serde_json::json!(0);

        assert_eq!(
            Project::from_json_str(&serde_json::to_string(&value).unwrap()),
            Err(ProjectError::InvalidConcurrency)
        );
    }

    #[test]
    fn project_rejects_invalid_browser_capture_settings() {
        let mut p = project();
        p.settings.browser_image_format = Some("webp".into());
        assert_eq!(p.validate(), Err(ProjectError::InvalidBrowserImageFormat));

        let mut p = project();
        p.settings.browser_jpeg_quality = Some(0);
        assert_eq!(p.validate(), Err(ProjectError::InvalidBrowserJpegQuality));

        let mut p = project();
        p.settings.browser_frame_timeout_ms = Some(0);
        assert_eq!(p.validate(), Err(ProjectError::InvalidBrowserFrameTimeout));

        let mut p = project();
        p.settings.browser_transport = Some("shared-memory".into());
        assert_eq!(p.validate(), Err(ProjectError::InvalidBrowserTransport));
    }

    #[test]
    fn project_accepts_valid_browser_capture_settings() {
        let mut p = project();
        p.settings.browser_image_format = Some(" JPEG ".into());
        p.settings.browser_jpeg_quality = Some(90);
        p.settings.browser_frame_timeout_ms = Some(5_000);
        p.settings.browser_transport = Some(" FILE ".into());
        assert!(p.validate().is_ok());
        p.settings.browser_transport_retries = Some(2);
        assert!(p.validate().is_ok());
    }

    #[test]
    fn job_store_validates_and_tracks_progress() {
        let mut store = JobStore::default();
        let id = store.submit(project()).unwrap();
        assert_eq!(store.get(&id).unwrap().status, JobStatus::Queued);
        assert!(store.update(&id, JobStatus::Preparing, 0));
        assert!(store.update(&id, JobStatus::Rendering, 12));
        assert_eq!(store.get(&id).unwrap().completed_frames, 12);
    }
    #[test]
    fn invalid_project_is_rejected() {
        let mut p = project();
        p.settings.fps = 0.0;
        assert_eq!(p.validate(), Err(ProjectError::InvalidFps));
    }
    #[test]
    fn invalid_project_frame_ranges_are_rejected() {
        let mut p = project();
        p.settings.frame_start = Some(20);
        p.settings.frame_end = Some(10);
        assert!(matches!(
            p.validate(),
            Err(ProjectError::InvalidFrameRange { .. })
        ));

        p.settings.frame_start = Some(0);
        p.settings.frame_end = Some(p.settings.duration);
        assert!(matches!(
            p.validate(),
            Err(ProjectError::InvalidFrameRange { .. })
        ));
    }

    #[test]
    fn project_rejects_clips_outside_duration() {
        let mut p = project();
        p.tracks = vec![Track {
            id: "main".into(),
            clips: vec![Clip {
                id: "late".into(),
                composition: "caption".into(),
                start: 59,
                duration: 2,
                props: serde_json::json!({}),
            }],
        }];
        assert!(matches!(
            p.validate(),
            Err(ProjectError::InvalidClip { reason, .. })
                if reason == "clip range must fit within the project duration"
        ));

        p.tracks[0].clips[0].start = u32::MAX;
        assert!(matches!(
            p.validate(),
            Err(ProjectError::InvalidClip { reason, .. })
                if reason == "clip range must fit within the project duration"
        ));
    }

    #[test]
    fn project_frame_start_without_end_defaults_to_duration_end() {
        let mut p = project();
        p.settings.frame_start = Some(12);
        assert!(p.validate().is_ok());
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["settings"]["frame_start"], 12);
    }
    #[test]
    fn invalid_job_transition_is_rejected() {
        let mut store = JobStore::default();
        let id = store.submit(project()).unwrap();
        assert!(store.try_update(&id, JobStatus::Encoding, 0).is_err());
        assert!(store.try_update(&id, JobStatus::Preparing, 0).is_ok());
    }
    #[test]
    fn progress_regression_and_failure_are_recorded() {
        let mut store = JobStore::default();
        let id = store.submit(project()).unwrap();
        store.try_update(&id, JobStatus::Preparing, 4).unwrap();
        assert!(matches!(
            store.try_update(&id, JobStatus::Rendering, 3),
            Err(ProjectError::ProgressRegressed { .. })
        ));
        store.fail(&id, "worker exited").unwrap();
        let job = store.get(&id).unwrap();
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error.as_deref(), Some("worker exited"));
    }

    #[test]
    fn cancellation_preserves_progress() {
        let mut store = JobStore::default();
        let id = store.submit(project()).unwrap();
        store.update(&id, JobStatus::Preparing, 0);
        store.update(&id, JobStatus::Rendering, 12);
        store.cancel(&id).unwrap();
        let job = store.get(&id).unwrap();
        assert_eq!(job.status, JobStatus::Cancelled);
        assert_eq!(job.completed_frames, 12);
    }

    #[test]
    fn cancellation_is_terminal_against_late_worker_updates() {
        let mut store = JobStore::default();
        let completed_id = store.submit(project()).unwrap();
        store.update(&completed_id, JobStatus::Preparing, 0);
        store.update(&completed_id, JobStatus::Rendering, 4);
        store.cancel(&completed_id).unwrap();
        assert!(matches!(
            store.try_update(&completed_id, JobStatus::Completed, 4),
            Err(ProjectError::InvalidJobTransition {
                from: JobStatus::Cancelled,
                to: JobStatus::Completed,
            })
        ));

        let failed_id = store.submit(project()).unwrap();
        store.update(&failed_id, JobStatus::Preparing, 0);
        store.update(&failed_id, JobStatus::Rendering, 4);
        store.cancel(&failed_id).unwrap();
        assert!(matches!(
            store.fail(&failed_id, "late worker failure"),
            Err(ProjectError::InvalidJobTransition {
                from: JobStatus::Cancelled,
                to: JobStatus::Failed,
            })
        ));
        assert_eq!(store.get(&failed_id).unwrap().status, JobStatus::Cancelled);
    }

    #[test]
    fn render_diagnostics_are_stored_and_serde_compatible() {
        let mut store = JobStore::default();
        let id = store.submit(project()).unwrap();
        store
            .set_render_diagnostics(
                &id,
                120,
                3,
                Some("scene contains GPU-unsupported nodes or effects".into()),
            )
            .unwrap();
        let job = store.get(&id).unwrap();
        assert_eq!(job.gpu_frames, Some(120));
        assert_eq!(job.cpu_fallback_frames, Some(3));
        assert_eq!(
            job.gpu_fallback_reason.as_deref(),
            Some("scene contains GPU-unsupported nodes or effects")
        );

        let encoded = serde_json::to_value(job).unwrap();
        let decoded: RenderJob = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, *job);
    }

    #[test]
    fn render_job_lifecycle_keeps_diagnostics_until_completion() {
        let mut store = JobStore::default();
        let id = store.submit(project()).unwrap();
        store.try_update(&id, JobStatus::Preparing, 0).unwrap();
        store.try_update(&id, JobStatus::Rendering, 8).unwrap();
        store.set_render_diagnostics(&id, 8, 0, None).unwrap();
        store.set_encoding_progress(&id, 6).unwrap();
        assert_eq!(store.get(&id).unwrap().encoded_frames, 6);
        store.complete_render(&id, 8).unwrap();

        let job = store.get(&id).unwrap();
        assert_eq!(job.status, JobStatus::Completed);
        assert_eq!(job.completed_frames, 8);
        assert_eq!(job.encoded_frames, 8);
        assert_eq!(job.gpu_frames, Some(8));
        assert_eq!(job.cpu_fallback_frames, Some(0));
        assert_eq!(job.gpu_fallback_reason, None);
    }

    #[test]
    fn complete_render_rejects_cancelled_job_without_reviving_it() {
        let mut store = JobStore::default();
        let id = store.submit(project()).unwrap();
        store.try_update(&id, JobStatus::Preparing, 0).unwrap();
        store.try_update(&id, JobStatus::Rendering, 4).unwrap();
        store.cancel(&id).unwrap();
        assert!(matches!(
            store.complete_render(&id, 4),
            Err(ProjectError::InvalidJobTransition {
                from: JobStatus::Cancelled,
                to: JobStatus::Completed,
            })
        ));
        assert_eq!(store.get(&id).unwrap().status, JobStatus::Cancelled);
    }

    #[test]
    fn late_encoding_progress_cannot_mutate_terminal_job() {
        let mut store = JobStore::default();
        let id = store.submit(project()).unwrap();
        store.try_update(&id, JobStatus::Preparing, 0).unwrap();
        store.try_update(&id, JobStatus::Rendering, 4).unwrap();
        store.cancel(&id).unwrap();

        assert!(matches!(
            store.set_encoding_progress(&id, 4),
            Err(ProjectError::InvalidJobTransition {
                from: JobStatus::Cancelled,
                to: JobStatus::Encoding,
            })
        ));
        assert_eq!(store.get(&id).unwrap().encoded_frames, 0);
    }

    #[test]
    fn list_returns_jobs_in_submission_order() {
        let mut store = JobStore::default();
        let first = store.submit(project()).unwrap();
        let second = store.submit(project()).unwrap();
        let jobs = store.list();
        assert_eq!(
            jobs.iter().map(|job| job.id.as_str()).collect::<Vec<_>>(),
            vec![first.as_str(), second.as_str()]
        );
    }

    #[test]
    fn retry_creates_a_fresh_queued_job() {
        let mut store = JobStore::default();
        let original = store.submit(project()).unwrap();
        store.cancel(&original).unwrap();
        let retry = store.retry(&original).unwrap();
        assert_eq!(retry, "job-2");
        let retry_job = store.get(&retry).unwrap();
        assert_eq!(retry_job.status, JobStatus::Queued);
        assert_eq!(retry_job.project, project());
        assert_eq!(retry_job.gpu_frames, None);
        assert_eq!(retry_job.cpu_fallback_frames, None);
        assert_eq!(retry_job.gpu_fallback_reason, None);
    }
}
