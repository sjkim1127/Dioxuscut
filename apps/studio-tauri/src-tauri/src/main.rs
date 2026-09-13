#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dioxuscut_cli::{
    execute_project_render_command_with_control, RenderBackend, RenderCodec, RenderRequest,
};
use dioxuscut_project::{JobStatus, JobStore, Project, RenderJob};
use dioxuscut_rasterizer::{
    make_cancel_signal, render_still_fallible_scaled, render_web_to_ffmpeg_pipe_fallible,
    BackendCapabilities, BrowserFrameBackend, PipeConfig, RenderCancellationToken, RenderControl,
    StillImageFormat, VideoCodec, WebFrameRequest, WebTimelineClip, WebWorkerMessage,
    WEB_WORKER_PROTOCOL_VERSION,
};
use dioxuscut_renderer::spawn_server;
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

fn browser_frame_cache_bytes() -> usize {
    std::env::var("DIOXUSCUT_FRAME_CACHE_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(dioxuscut_rasterizer::DEFAULT_MAX_CACHE_BYTES)
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

fn browser_worker_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("DIOXUSCUT_BROWSER_WORKER") {
        return Ok(PathBuf::from(path));
    }

    let mut candidates = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            candidates.push(parent.join("resources/_up_/scripts/three-render-worker.mjs"));
            candidates.push(parent.join("resources/scripts/three-render-worker.mjs"));
            candidates.push(parent.join("../Resources/scripts/three-render-worker.mjs"));
            candidates.push(parent.join("../Resources/_up_/scripts/three-render-worker.mjs"));
            candidates.push(parent.join("scripts/three-render-worker.mjs"));
        }
    }
    candidates
        .push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/three-render-worker.mjs"));

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "Browser backend requires DIOXUSCUT_BROWSER_WORKER or a bundled three-render-worker.mjs"
                .to_string()
        })
}

fn browser_frontend_path() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            candidates.push(parent.join("resources/_up_/dist"));
            candidates.push(parent.join("resources/dist"));
            candidates.push(parent.join("../Resources/_up_/dist"));
        }
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist"));
    candidates
        .into_iter()
        .find(|path| path.join("index.html").is_file())
        .ok_or_else(|| "bundled browser frontend dist/index.html was not found".to_string())
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

/// Load and submit a project using its file directory as the asset base.
/// This is the AI-friendly counterpart to `submit_project(Project)`, which is
/// intentionally path-independent for callers that already resolved assets.
#[tauri::command]
fn submit_project_from_path(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let project = load_project_file(std::path::Path::new(&path))?;
    submit_project(state, project)
}

#[tauri::command]
fn start_render_job(
    state: tauri::State<'_, AppState>,
    id: String,
    output: String,
) -> Result<(), String> {
    let mut project = {
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
    let render_frame_start = project.settings.frame_start.unwrap_or(0);
    let render_frame_end = project
        .settings
        .frame_end
        .unwrap_or(project.settings.duration.saturating_sub(1));
    let render_frame_count = render_frame_end
        .saturating_sub(render_frame_start)
        .saturating_add(1);
    let gif_output = std::path::Path::new(&output)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gif"));
    let output_frame_count = if gif_output {
        render_frame_count.div_ceil(project.settings.frame_step)
    } else {
        render_frame_count
    };
    {
        let mut store = state
            .jobs
            .lock()
            .map_err(|_| "job store lock poisoned".to_string())?;
        store
            .set_output(&id, &output)
            .map_err(|error| error.to_string())?;
        store
            .try_update(&id, JobStatus::Preparing, 0)
            .map_err(|error| error.to_string())?;
    }
    if backend_kind != dioxuscut_project::BackendKind::Browser {
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
                let asset_cache_dir = std::env::temp_dir().join("dioxuscut-assets").join(&id);
                project
                    .materialize_remote_assets(&asset_cache_dir, 256 * 1024 * 1024)
                    .map_err(|error| error.to_string())?;
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
                    audio: dioxuscut_cli::project_audio_assets(&project),
                    width: project.settings.width,
                    height: project.settings.height,
                    scale: project.settings.scale,
                    fps: project.settings.fps,
                    duration: project.settings.duration,
                    backend: match project.settings.backend {
                        dioxuscut_project::BackendKind::Native => RenderBackend::Native,
                        dioxuscut_project::BackendKind::Gpu => RenderBackend::Gpu,
                        dioxuscut_project::BackendKind::Browser => unreachable!(),
                    },
                    codec: project_render_codec(&output_path),
                    frame_start: project.settings.frame_start.unwrap_or(0),
                    frame_end: project.settings.frame_end,
                    frame_step: project.settings.frame_step,
                    concurrency: project.settings.concurrency.map(|value| value as usize),
                    timeout_seconds: None,
                    crf: project.settings.crf.unwrap_or(18),
                    preset: project
                        .settings
                        .preset
                        .clone()
                        .unwrap_or_else(|| "fast".into()),
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
                    .block_on(execute_project_render_command_with_control(
                        &request, &project, control,
                    ))
                    .map_err(|e| e.to_string())
            })();
            let _ = std::fs::remove_file(&props_path);
            let _ =
                std::fs::remove_dir_all(std::env::temp_dir().join("dioxuscut-assets").join(&id));
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
                let frames = output_frame_count;
                let _ = store.try_update(&id, JobStatus::Rendering, frames);
                let _ = store.try_update(&id, JobStatus::Encoding, frames);
                let _ = store.try_update(&id, JobStatus::Completed, frames);
            }
        });
        return Ok(());
    }
    let worker = browser_worker_path()?;
    let configured_url = std::env::var("DIOXUSCUT_BROWSER_URL").ok();
    let frontend_path = if configured_url.is_none() {
        Some(browser_frontend_path()?)
    } else {
        None
    };
    let concurrency = project
        .settings
        .concurrency
        .map(|value| value as usize)
        .or_else(|| {
            std::env::var("DIOXUSCUT_BROWSER_CONCURRENCY")
                .ok()
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(1);
    let state_jobs = Arc::clone(&state.jobs);
    let state_cancellations = Arc::clone(&state.cancellations);
    let cancellation = make_cancel_signal();
    state_cancellations
        .lock()
        .map_err(|_| "cancellation store lock poisoned".to_string())?
        .insert(id.clone(), cancellation.clone());
    thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
            let local_server = frontend_path
                .as_ref()
                .map(|root| runtime.block_on(spawn_server(0, root)))
                .transpose()
                .map_err(|error| error.to_string())?;
            let url = configured_url
                .clone()
                .or_else(|| local_server.as_ref().map(|server| server.url().to_string()))
                .ok_or_else(|| "browser rendering URL is unavailable".to_string())?;
            let node = std::env::var_os("DIOXUSCUT_BROWSER_NODE").unwrap_or_else(|| "node".into());
            let mut backend = BrowserFrameBackend::with_concurrency(node, worker, url, concurrency)
                .map_err(|error| error.to_string())?
                .with_frame_cache_bytes(browser_frame_cache_bytes());
            if let Some(format) = project.settings.browser_image_format.as_deref() {
                backend = backend.with_image_format(format);
            }
            if let Some(quality) = project.settings.browser_jpeg_quality {
                backend = backend.with_jpeg_quality(quality);
            }
            if let Some(timeout_ms) = project.settings.browser_frame_timeout_ms {
                backend = backend.with_frame_timeout(std::time::Duration::from_millis(timeout_ms));
            }
            if let Some(retries) = project.settings.browser_transport_retries {
                backend = backend.with_transport_retries(retries);
            }
            if project
                .settings
                .browser_transport
                .as_deref()
                .is_some_and(|transport| transport.trim().eq_ignore_ascii_case("file"))
            {
                backend = backend.with_file_transport(true);
            }
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
            backend
                .set_timeline(
                    project
                        .tracks
                        .iter()
                        .flat_map(|track| track.clips.iter())
                        .map(|clip| WebTimelineClip {
                            id: clip.id.clone(),
                            composition: clip.composition.clone(),
                            start: clip.start,
                            duration: clip.duration,
                            props: clip.props.clone(),
                        })
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
                render_still_fallible_scaled(
                    &backend,
                    project.settings.width,
                    project.settings.height,
                    project.settings.fps,
                    render_frame_start,
                    &output_path,
                    format,
                    &control,
                    project.settings.scale,
                    |_| Ok::<_, std::convert::Infallible>(dioxuscut_rasterizer::Scene::new()),
                )
                .map_err(|error| error.to_string())?;
            } else {
                let config = PipeConfig::new(
                    project.settings.width,
                    project.settings.height,
                    project.settings.fps,
                    output_frame_count,
                    output_path.clone(),
                )
                .with_codec(project_video_codec(&output_path))
                .with_scale(project.settings.scale)
                .with_frame_step(project.settings.frame_step)
                .with_frame_start(render_frame_start)
                .with_quality(
                    project.settings.crf.unwrap_or(18),
                    project
                        .settings
                        .preset
                        .clone()
                        .unwrap_or_else(|| "fast".into()),
                )
                .with_audio_tracks(
                    dioxuscut_cli::project_audio_assets(&project)
                        .into_iter()
                        .map(|path| dioxuscut_rasterizer::AudioTrack::new(path.to_string_lossy()))
                        .collect::<Vec<_>>(),
                )
                .with_control(control);
                render_web_to_ffmpeg_pipe_fallible(&backend, &config, project.props.clone())
                    .map_err(|error| error.to_string())?;
            }
            let mut store = state_jobs
                .lock()
                .map_err(|_| "job store lock poisoned".to_string())?;
            store
                .try_update(&id, JobStatus::Encoding, output_frame_count)
                .map_err(|error| error.to_string())?;
            store
                .try_update(&id, JobStatus::Completed, output_frame_count)
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
    load_project_file(std::path::Path::new(&path))
}

fn load_project_file(path: &std::path::Path) -> Result<Project, String> {
    let mut project = Project::load(path).map_err(|error| error.to_string())?;
    let base_dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    project
        .validate_asset_files(base_dir)
        .map_err(|error| error.to_string())?;
    project.resolve_local_asset_paths(base_dir);
    Ok(project)
}

#[tauri::command]
fn save_project(path: String, project: Project) -> Result<(), String> {
    let path = std::path::PathBuf::from(path);
    let mut project = project;
    let base_dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    project.relativize_local_asset_paths(base_dir);
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
    let worker = browser_worker_path()?;
    let url = std::env::var("DIOXUSCUT_BROWSER_URL")
        .unwrap_or_else(|_| "http://localhost:1420".to_string());
    let node = std::env::var_os("DIOXUSCUT_BROWSER_NODE").unwrap_or_else(|| "node".into());
    let backend = BrowserFrameBackend::with_concurrency(node, worker, url, 1)
        .map_err(|error| error.to_string())?
        .with_frame_cache_bytes(browser_frame_cache_bytes());
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
            submit_project_from_path,
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
