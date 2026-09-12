#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dioxuscut_cli::{
    execute_render_command_with_control, RenderBackend, RenderCodec, RenderRequest,
};
use dioxuscut_project::{JobStatus, JobStore, Project, RenderJob};
use dioxuscut_rasterizer::{
    make_cancel_signal, render_still_fallible, render_web_to_ffmpeg_pipe_fallible,
    BackendCapabilities, BrowserFrameBackend, PipeConfig, RenderCancellationToken, RenderControl,
    StillImageFormat, VideoCodec, WebFrameRequest, WebWorkerMessage, WEB_WORKER_PROTOCOL_VERSION,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

fn project_render_codec(path: &std::path::Path) -> RenderCodec {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "webm" => RenderCodec::Vp9,
        "mov" => RenderCodec::ProRes,
        "gif" => RenderCodec::Gif,
        _ => RenderCodec::H264,
    }
}

fn project_video_codec(path: &std::path::Path) -> VideoCodec {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "webm" => VideoCodec::Vp9,
        "mov" => VideoCodec::ProRes,
        "gif" => VideoCodec::Gif,
        _ => VideoCodec::H264,
    }
}

fn project_still_format(path: &std::path::Path) -> Option<StillImageFormat> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some(StillImageFormat::Png),
        "jpg" | "jpeg" => Some(StillImageFormat::Jpeg),
        "webp" => Some(StillImageFormat::WebP),
        _ => None,
    }
}

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
    let backend_kind = project.settings.backend;
    if backend_kind == dioxuscut_project::BackendKind::Native {
        let state_jobs = Arc::clone(&state.jobs);
        let state_cancellations = Arc::clone(&state.cancellations);
        let cancellation = make_cancel_signal();
        state_cancellations
            .lock()
            .map_err(|_| "cancellation store lock poisoned".to_string())?
            .insert(id.clone(), cancellation.clone());
        thread::spawn(move || {
            let props_path = std::env::temp_dir().join(format!("dioxuscut-{id}-props.json"));
            let result = (|| -> Result<(), String> {
                std::fs::write(
                    &props_path,
                    serde_json::to_vec(&project.props).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                let output_path = PathBuf::from(&output);
                let request = RenderRequest {
                    composition: Some(project.composition.clone()),
                    script: None,
                    props: Some(props_path.clone()),
                    output: output_path.clone(),
                    audio: vec![],
                    width: project.settings.width,
                    height: project.settings.height,
                    fps: project.settings.fps,
                    duration: project.settings.duration,
                    backend: RenderBackend::Native,
                    codec: project_render_codec(&output_path),
                    frame_start: 0,
                    frame_end: None,
                    timeout_seconds: None,
                    crf: 18,
                    preset: "fast".into(),
                    hw_accel: dioxuscut_rasterizer::HwAccel::Auto,
                    sandbox_roots: vec![],
                    permissive: true,
                };
                let progress_state = Arc::clone(&state_jobs);
                let progress_id = id.clone();
                let control = RenderControl::new()
                    .with_cancellation(cancellation)
                    .with_progress(move |progress| {
                        if let Ok(mut store) = progress_state.lock() {
                            let _ = store.try_update(
                                &progress_id,
                                JobStatus::Rendering,
                                progress.completed_frames,
                            );
                        }
                    });
                tokio::runtime::Runtime::new()
                    .map_err(|e| e.to_string())?
                    .block_on(execute_render_command_with_control(&request, control))
                    .map_err(|e| e.to_string())
            })();
            let _ = std::fs::remove_file(&props_path);
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
            } else if let Ok(mut store) = state_jobs.lock() {
                let frames = project.settings.duration;
                let _ = store.try_update(&id, JobStatus::Rendering, frames);
                let _ = store.try_update(&id, JobStatus::Encoding, frames);
                let _ = store.try_update(&id, JobStatus::Completed, frames);
            }
        });
        return Ok(());
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
        store
            .set_output(&id, &output)
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
            backend
                .set_composition(&project.composition)
                .map_err(|error| error.to_string())?;
            backend
                .set_assets(
                    project
                        .assets
                        .iter()
                        .map(|asset| asset.path.clone())
                        .collect(),
                )
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
            let output_path = PathBuf::from(&output);
            if let Some(format) = project_still_format(&output_path) {
                render_still_fallible(
                    &backend,
                    project.settings.width,
                    project.settings.height,
                    project.settings.fps,
                    0,
                    &output_path,
                    format,
                    &control,
                    |_| Ok::<_, std::convert::Infallible>(dioxuscut_rasterizer::Scene::new()),
                )
                .map_err(|error| error.to_string())?;
            } else {
                let config = PipeConfig::new(
                    project.settings.width,
                    project.settings.height,
                    project.settings.fps,
                    project.settings.duration,
                    output_path.clone(),
                )
                .with_codec(project_video_codec(&output_path))
                .with_control(control);
                render_web_to_ffmpeg_pipe_fallible(&backend, &config, project.props.clone())
                    .map_err(|error| error.to_string())?;
            }
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
fn validate_project(source: String) -> Result<Project, String> {
    Project::from_json_str(&source).map_err(|error| error.to_string())
}

#[tauri::command]
fn project_schema() -> Result<serde_json::Value, String> {
    serde_json::from_str(include_str!(
        "../../../../schemas/dioxuscut-project-v1.schema.json"
    ))
    .map_err(|error| format!("embedded project schema is invalid: {error}"))
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
            compositions: vec![],
        }).expect("protocol message is serializable"),
    })
}

#[tauri::command]
fn list_browser_compositions() -> Result<Vec<String>, String> {
    let worker = std::env::var_os("DIOXUSCUT_BROWSER_WORKER")
        .ok_or_else(|| "Browser backend requires DIOXUSCUT_BROWSER_WORKER".to_string())?;
    let url = std::env::var("DIOXUSCUT_BROWSER_URL")
        .unwrap_or_else(|_| "http://localhost:1420".to_string());
    let backend = BrowserFrameBackend::with_concurrency("node", worker, url, 1)
        .map_err(|error| error.to_string())?;
    Ok(backend.compositions())
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
            list_browser_compositions,
            validate_frame_request,
            submit_project,
            start_render_job,
            load_project,
            save_project,
            validate_project,
            project_schema,
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
