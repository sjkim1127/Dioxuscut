use dioxuscut_project::{JobStore, Project, RenderJob};
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_reads_empty_store_without_tauri_runtime() {
        let controller = RenderJobController::new(Arc::new(Mutex::new(JobStore::default())));
        assert_eq!(controller.get("job-missing").unwrap(), None);
        assert!(controller.list().unwrap().is_empty());
    }
}
