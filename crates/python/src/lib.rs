//! Python native bindings for Dioxuscut using PyO3.
use dioxuscut_cli::{
    built_in_registry, execute_render_command, RenderBackend, RenderCodec, RenderRequest,
};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use std::path::PathBuf;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Return the current Dioxuscut engine version.
#[pyfunction]
fn get_version() -> &'static str {
    VERSION
}

/// Return the list of registered built-in composition IDs.
#[pyfunction]
fn list_compositions() -> Vec<String> {
    let registry = built_in_registry();
    registry.ids().into_iter().map(|s| s.to_string()).collect()
}

/// Parse a codec string into RenderCodec.
fn parse_codec(codec: &str) -> Result<RenderCodec, PyErr> {
    match codec.to_ascii_lowercase().as_str() {
        "h264" | "mp4" => Ok(RenderCodec::H264),
        "h265" | "hevc" => Ok(RenderCodec::H265),
        "vp9" | "webm" => Ok(RenderCodec::Vp9),
        "av1" => Ok(RenderCodec::Av1),
        "prores" | "mov" => Ok(RenderCodec::ProRes),
        "gif" => Ok(RenderCodec::Gif),
        "png" => Ok(RenderCodec::Png),
        "jpeg" | "jpg" => Ok(RenderCodec::Jpeg),
        "webp" => Ok(RenderCodec::Webp),
        other => Err(PyValueError::new_err(format!(
            "Unsupported codec '{other}'. Supported: h264, h265, vp9, av1, prores, gif, png, jpeg, webp"
        ))),
    }
}

/// Parse backend string.
fn parse_backend(backend: &str) -> Result<RenderBackend, PyErr> {
    match backend.to_ascii_lowercase().as_str() {
        "native" | "cpu" => Ok(RenderBackend::Native),
        "gpu" => Ok(RenderBackend::Gpu),
        other => Err(PyValueError::new_err(format!(
            "Unsupported backend '{other}'. Supported: native, gpu"
        ))),
    }
}

/// Parse hw_accel string.
fn parse_hw_accel(hw_accel: &str) -> Result<dioxuscut_rasterizer::HwAccel, PyErr> {
    match hw_accel.to_ascii_lowercase().as_str() {
        "auto" => Ok(dioxuscut_rasterizer::HwAccel::Auto),
        "disabled" | "none" | "cpu" | "sw" => Ok(dioxuscut_rasterizer::HwAccel::Disabled),
        "videotoolbox" | "vt" | "apple" => Ok(dioxuscut_rasterizer::HwAccel::VideoToolbox),
        "nvenc" | "nvidia" => Ok(dioxuscut_rasterizer::HwAccel::Nvenc),
        other => Err(PyValueError::new_err(format!(
            "Unsupported hw_accel '{other}'. Supported: auto, disabled, videotoolbox, nvenc"
        ))),
    }
}

/// Render a video composition or Rhai script to an output file.
///
/// Releases the Python GIL during rendering for high-performance concurrency.
#[pyfunction]
#[pyo3(signature = (
    output,
    composition = None,
    script = None,
    props_json = None,
    width = 1920,
    height = 1080,
    fps = 30.0,
    duration = 180,
    backend = "native",
    codec = "h264",
    frame_start = 0,
    frame_end = None,
    crf = 18,
    preset = "fast",
    hw_accel = "auto"
))]
#[allow(clippy::too_many_arguments)]
fn render_native(
    py: Python<'_>,
    output: String,
    composition: Option<String>,
    script: Option<String>,
    props_json: Option<String>,
    width: u32,
    height: u32,
    fps: f64,
    duration: u32,
    backend: &str,
    codec: &str,
    frame_start: u32,
    frame_end: Option<u32>,
    crf: u32,
    preset: &str,
    hw_accel: &str,
) -> PyResult<()> {
    let parsed_codec = parse_codec(codec)?;
    let parsed_backend = parse_backend(backend)?;
    let parsed_hw_accel = parse_hw_accel(hw_accel)?;

    // Handle props: if JSON string provided, write to a temporary file or validate
    let props_path = if let Some(ref json_str) = props_json {
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("dioxuscut_props_{}.json", std::process::id()));
        std::fs::write(&temp_file, json_str)
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to write props: {e}")))?;
        Some(temp_file)
    } else {
        None
    };

    let request = RenderRequest {
        composition,
        script: script.map(PathBuf::from),
        props: props_path.clone(),
        output: PathBuf::from(output),
        audio: Vec::new(),
        width,
        height,
        fps,
        duration,
        backend: parsed_backend,
        codec: parsed_codec,
        frame_start,
        frame_end,
        timeout_seconds: None,
        crf,
        preset: preset.to_string(),
        hw_accel: parsed_hw_accel,
    };

    // Release GIL while rendering in a dedicated Tokio runtime
    let result = py.allow_threads(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to initialize runtime: {e}")))?;

        rt.block_on(async {
            execute_render_command(&request)
                .await
                .map_err(|e| PyRuntimeError::new_err(format!("Render failed: {e:#}")))
        })
    });

    if let Some(path) = props_path {
        let _ = std::fs::remove_file(path);
    }

    result
}

/// Python module initialization.
#[pymodule]
fn _dioxuscut(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(get_version, m)?)?;
    m.add_function(wrap_pyfunction!(list_compositions, m)?)?;
    m.add_function(wrap_pyfunction!(render_native, m)?)?;
    Ok(())
}
