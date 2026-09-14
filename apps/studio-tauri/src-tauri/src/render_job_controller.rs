use dioxuscut_project::{JobStatus, JobStore, Project, RenderJob};
use std::sync::{Arc, Mutex};

/// Host-neutral adapter for the Tauri render-job commands.
///
/// Keeping the mutex and store lookup here makes job orchestration testable
/// without constructing a Tauri runtime or UI window.
#[derive(Clone)]
pub(crate) struct RenderJobController {
    jobs: Arc<Mutex<JobStore>>,
}

impl RenderJobController {
    pub(crate) fn new(jobs: Arc<Mutex<JobStore>>) -> Self {
        Self { jobs }
    }

    pub(crate) fn submit(&self, project: Project) -> Result<String, String> {
        self.jobs
            .lock()
            .map_err(|_| "job store lock poisoned".to_string())?
            .submit(project)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn get(&self, id: &str) -> Result<Option<RenderJob>, String> {
        Ok(self
            .jobs
            .lock()
            .map_err(|_| "job store lock poisoned".to_string())?
            .get(id)
            .cloned())
    }

    pub(crate) fn list(&self) -> Result<Vec<RenderJob>, String> {
        Ok(self
            .jobs
            .lock()
            .map_err(|_| "job store lock poisoned".to_string())?
            .list())
    }

    pub(crate) fn complete(&self, id: &str, frames: u32) -> Result<(), String> {
        self.jobs
            .lock()
            .map_err(|_| "job store lock poisoned".to_string())?
            .complete_render(id, frames)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn update(&self, id: &str, status: JobStatus, frames: u32) -> Result<(), String> {
        self.jobs
            .lock()
            .map_err(|_| "job store lock poisoned".to_string())?
            .try_update(id, status, frames)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn set_encoding_progress(&self, id: &str, frames: u32) -> Result<(), String> {
        self.jobs
            .lock()
            .map_err(|_| "job store lock poisoned".to_string())?
            .set_encoding_progress(id, frames)
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxuscut_project::{BackendKind, ProjectSettings};

    #[test]
    fn controller_reads_empty_store_without_tauri_runtime() {
        let controller = RenderJobController::new(Arc::new(Mutex::new(JobStore::default())));
        assert_eq!(controller.get("job-missing").unwrap(), None);
        assert!(controller.list().unwrap().is_empty());
    }

    #[test]
    fn controller_replays_render_and_encoding_job_lifecycle() {
        let controller = RenderJobController::new(Arc::new(Mutex::new(JobStore::default())));
        let id = controller
            .submit(Project {
                version: 1,
                composition: "smoke".into(),
                settings: ProjectSettings {
                    width: 16,
                    height: 16,
                    fps: 30.0,
                    duration: 3,
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
                props: serde_json::json!({}),
                assets: vec![],
                tracks: vec![],
            })
            .unwrap();
        controller.update(&id, JobStatus::Preparing, 0).unwrap();
        controller.update(&id, JobStatus::Rendering, 3).unwrap();
        controller.set_encoding_progress(&id, 2).unwrap();
        controller.complete(&id, 3).unwrap();

        let job = controller.get(&id).unwrap().unwrap();
        assert_eq!(job.status, JobStatus::Completed);
        assert_eq!(job.completed_frames, 3);
        assert_eq!(job.encoded_frames, 3);
    }
}
