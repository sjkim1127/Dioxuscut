#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dioxuscut_project::{JobStatus, JobStore, Project, RenderJob};
use dioxuscut_rasterizer::{
    make_cancel_signal, render_web_to_ffmpeg_pipe_fallible, BackendCapabilities,
    BrowserFrameBackend, PipeConfig, RenderCancellationToken, RenderControl, VideoCodec,
    WebFrameRequest, WebWorkerMessage, WEB_WORKER_PROTOCOL_VERSION,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

struct AppState {
    jobs: Arc<Mutex<JobStore>>,
    cancellations: Arc<Mutex<HashMap<String, RenderCancellationToken>>>,
}

#[tauri::command]
fn submit_project(state: tauri::State<'_, AppState>, project: Project) -> Result<String, String> {
    state
        .jobs
        .lock()
        .map_err(|_| "job store lock poisoned".to_string())?
        .submit(project)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn start_render_job(
    state: tauri::State<'_, AppState>,
    id: String,
    output: String,
) -> Result<(), String> {
    let project = {
        let store = state
            .jobs
            .lock()
            .map_err(|_| "job store lock poisoned".to_string())?;
        store
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("render job '{id}' was not found"))?
            .project
    };
    if project.settings.backend != dioxuscut_project::BackendKind::Browser {
        return Err("Tauri executor currently supports browser backend jobs only".into());
    }
    let worker = std::env::var_os("DIOXUSCUT_BROWSER_WORKER")
        .ok_or_else(|| "Browser backend requires DIOXUSCUT_BROWSER_WORKER".to_string())?;
    let url = std::env::var("DIOXUSCUT_BROWSER_URL")
        .unwrap_or_else(|_| "http://localhost:1420".to_string());
    let concurrency = std::env::var("DIOXUSCUT_BROWSER_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    {
        let mut store = state
            .jobs
            .lock()
            .map_err(|_| "job store lock poisoned".to_string())?;
        store
            .try_update(&id, JobStatus::Preparing, 0)
            .map_err(|error| error.to_string())?;
    }
    let state_jobs = Arc::clone(&state.jobs);
    let state_cancellations = Arc::clone(&state.cancellations);
    let cancellation = make_cancel_signal();
    state_cancellations
        .lock()
        .map_err(|_| "cancellation store lock poisoned".to_string())?
        .insert(id.clone(), cancellation.clone());
    thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let backend = BrowserFrameBackend::with_concurrency("node", worker, url, concurrency)
                .map_err(|error| error.to_string())?;
            let state_for_progress = Arc::clone(&state_jobs);
            let progress_id = id.clone();
            let control = RenderControl::new()
                .with_cancellation(cancellation)
                .with_progress(move |progress| {
                    if let Ok(mut store) = state_for_progress.lock() {
                        let _ = store.try_update(
                            &progress_id,
                            JobStatus::Rendering,
                            progress.completed_frames,
                        );
                    }
                });
            let config = PipeConfig::new(
                project.settings.width,
                project.settings.height,
                project.settings.fps,
                project.settings.duration,
                PathBuf::from(output),
            )
            .with_codec(VideoCodec::H264)
            .with_control(control);
            render_web_to_ffmpeg_pipe_fallible(&backend, &config, project.props.clone())
                .map_err(|error| error.to_string())?;
            let mut store = state_jobs
                .lock()
                .map_err(|_| "job store lock poisoned".to_string())?;
            store
                .try_update(&id, JobStatus::Encoding, project.settings.duration)
                .map_err(|error| error.to_string())?;
            store
                .try_update(&id, JobStatus::Completed, project.settings.duration)
                .map_err(|error| error.to_string())
        })();
        if let Ok(mut cancellations) = state_cancellations.lock() {
            cancellations.remove(&id);
        }
        if let Err(error) = result {
            if let Ok(mut store) = state_jobs.lock() {
                if store
                    .get(&id)
                    .is_some_and(|job| job.status != JobStatus::Cancelled)
                {
                    let _ = store.fail(&id, error);
                }
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn load_project(path: String) -> Result<Project, String> {
    Project::load(path).map_err(|error| error.to_string())
}

#[tauri::command]
fn save_project(path: String, project: Project) -> Result<(), String> {
    project.save(path).map_err(|error| error.to_string())
}

#[tauri::command]
fn get_render_job(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Option<RenderJob>, String> {
    Ok(state
        .jobs
        .lock()
        .map_err(|_| "job store lock poisoned".to_string())?
        .get(&id)
        .cloned())
}

#[tauri::command]
fn list_render_jobs(state: tauri::State<'_, AppState>) -> Result<Vec<RenderJob>, String> {
    Ok(state
        .jobs
        .lock()
        .map_err(|_| "job store lock poisoned".to_string())?
        .list())
}

#[tauri::command]
fn update_render_job(
    state: tauri::State<'_, AppState>,
    id: String,
    status: JobStatus,
    completed_frames: u32,
) -> Result<(), String> {
    state
        .jobs
        .lock()
        .map_err(|_| "job store lock poisoned".to_string())?
        .try_update(&id, status, completed_frames)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn fail_render_job(
    state: tauri::State<'_, AppState>,
    id: String,
    message: String,
) -> Result<(), String> {
    state
        .jobs
        .lock()
        .map_err(|_| "job store lock poisoned".to_string())?
        .fail(&id, message)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn cancel_render_job(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    if let Ok(cancellations) = state.cancellations.lock() {
        if let Some(token) = cancellations.get(&id) {
            token.cancel();
        }
    }
    state
        .jobs
        .lock()
        .map_err(|_| "job store lock poisoned".to_string())?
        .cancel(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn retry_render_job(state: tauri::State<'_, AppState>, id: String) -> Result<String, String> {
    state
        .jobs
        .lock()
        .map_err(|_| "job store lock poisoned".to_string())?
        .retry(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn validate_frame_request(request: WebFrameRequest) -> Result<WebFrameRequest, String> {
    if request.width == 0 || request.height == 0 {
        return Err("preview dimensions must be greater than zero".into());
    }
    if !request.fps.is_finite() || request.fps <= 0.0 {
        return Err("preview fps must be finite and greater than zero".into());
    }
    Ok(request)
}

#[tauri::command]
fn backend_capabilities() -> BackendCapabilities {
    BackendCapabilities {
        native_scene: true,
        browser_runtime: true,
        gpu_accelerated: true,
        supports_streaming: false,
    }
}

#[tauri::command]
fn web_worker_protocol() -> serde_json::Value {
    serde_json::json!({
        "version": WEB_WORKER_PROTOCOL_VERSION,
        "messages": ["ready", "render", "frame", "error", "shutdown"],
        "example": serde_json::to_value(WebWorkerMessage::Ready {
            protocol: WEB_WORKER_PROTOCOL_VERSION,
        }).expect("protocol message is serializable"),
    })
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            jobs: Arc::new(Mutex::new(JobStore::default())),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
        })
        .invoke_handler(tauri::generate_handler![
            backend_capabilities,
            web_worker_protocol,
            validate_frame_request,
            submit_project,
            start_render_job,
            load_project,
            save_project,
            get_render_job,
            list_render_jobs,
            update_render_job,
            fail_render_job,
            cancel_render_job,
            retry_render_job
        ])
        .run(tauri::generate_context!())
        .expect("error while running Dioxuscut Studio");
}
