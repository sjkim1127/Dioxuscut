//! Host-neutral project and render-job contracts.
//!
//! Dioxus, Tauri, CLI, and Python integrations should exchange these models
//! instead of depending on one another's UI state.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
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
    pub error: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
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
    #[error("project asset id cannot be empty")]
    EmptyAssetId,
    #[error("project asset path cannot be empty for '{0}'")]
    EmptyAssetPath(String),
    #[error("project asset id '{0}' is duplicated")]
    DuplicateAssetId(String),
    #[error("project asset '{asset}' could not be read: {reason}")]
    AssetRead { asset: String, reason: String },
    #[error("project asset '{asset}' SHA-256 mismatch: expected {expected}, got {actual}")]
    AssetHashMismatch {
        asset: String,
        expected: String,
        actual: String,
    },
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
            }
        }
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
        for asset in &self.assets {
            if asset.path.contains("://") || asset.path.starts_with("data:") {
                continue;
            }
            let path = base_dir.as_ref().join(&asset.path);
            let bytes = std::fs::read(&path).map_err(|error| ProjectError::AssetRead {
                asset: asset.id.clone(),
                reason: format!("{} ({})", error, path.display()),
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
                error: None,
                output: None,
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
                JobStatus::Encoding | JobStatus::Cancelled | JobStatus::Failed
            ) | (
                JobStatus::Encoding,
                JobStatus::Completed | JobStatus::Cancelled | JobStatus::Failed
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
        std::fs::remove_dir_all(base).unwrap();
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
        assert_eq!(store.get(&retry).unwrap().status, JobStatus::Queued);
        assert_eq!(store.get(&retry).unwrap().project, project());
    }
}
