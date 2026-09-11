//! Python native bindings for Dioxuscut using PyO3.
#![allow(clippy::useless_conversion)]
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

/// Deterministic pseudo-random number generator matching Remotion's random(seed).
#[pyfunction]
fn random(seed: &Bound<'_, PyAny>) -> PyResult<f64> {
    if let Ok(s) = seed.extract::<String>() {
        Ok(dioxuscut_animation::random(s))
    } else if let Ok(n) = seed.extract::<f64>() {
        Ok(dioxuscut_animation::random(n))
    } else if let Ok(i) = seed.extract::<i64>() {
        Ok(dioxuscut_animation::random(i as f64))
    } else {
        Err(PyValueError::new_err(
            "random() seed must be a string or number",
        ))
    }
}

/// Map an input value from an input range to an output range, matching Remotion interpolate().
#[pyfunction]
#[pyo3(signature = (
    input,
    input_range,
    output_range,
    extrapolate_left = "extend",
    extrapolate_right = "extend"
))]
fn interpolate(
    input: f64,
    input_range: Vec<f64>,
    output_range: Vec<f64>,
    extrapolate_left: &str,
    extrapolate_right: &str,
) -> PyResult<f64> {
    let parse_extrapolate = |s: &str| match s.to_ascii_lowercase().as_str() {
        "extend" => Ok(dioxuscut_animation::ExtrapolateType::Extend),
        "identity" => Ok(dioxuscut_animation::ExtrapolateType::Identity),
        "clamp" => Ok(dioxuscut_animation::ExtrapolateType::Clamp),
        other => Err(PyValueError::new_err(format!(
            "Unsupported extrapolate '{other}'. Supported: extend, identity, clamp"
        ))),
    };

    let left = parse_extrapolate(extrapolate_left)?;
    let right = parse_extrapolate(extrapolate_right)?;

    let opts = dioxuscut_animation::InterpolateOptions {
        extrapolate_left: left,
        extrapolate_right: right,
        easing: None,
    };

    if input_range.len() != output_range.len() {
        return Err(PyValueError::new_err(
            "input_range and output_range must have the same length",
        ));
    }
    if input_range.len() < 2 {
        return Err(PyValueError::new_err(
            "input_range must have at least 2 elements",
        ));
    }

    Ok(dioxuscut_animation::interpolate(
        input,
        &input_range,
        &output_range,
        opts,
    ))
}

/// Interpolate colors, supporting both:
/// 1) Remotion multi-range: `interpolate_colors(input, input_range, output_range)`
/// 2) 2-color shorthand: `interpolate_colors(from, to, progress)`
#[pyfunction]
fn interpolate_colors(
    arg1: &Bound<'_, PyAny>,
    arg2: &Bound<'_, PyAny>,
    arg3: &Bound<'_, PyAny>,
) -> PyResult<String> {
    if let Ok(input) = arg1.extract::<f64>() {
        if let (Ok(in_range), Ok(out_range)) =
            (arg2.extract::<Vec<f64>>(), arg3.extract::<Vec<String>>())
        {
            let str_slices: Vec<&str> = out_range.iter().map(|s| s.as_str()).collect();
            return Ok(dioxuscut_animation::interpolate_colors_range(
                input,
                &in_range,
                &str_slices,
            ));
        }
    }

    if let (Ok(from), Ok(to), Ok(progress)) = (
        arg1.extract::<String>(),
        arg2.extract::<String>(),
        arg3.extract::<f64>(),
    ) {
        return Ok(dioxuscut_animation::interpolate_colors(
            &from, &to, progress,
        ));
    }

    Err(PyValueError::new_err(
        "interpolate_colors requires either (input: float, input_range: list[float], output_range: list[str]) or (from_color: str, to_color: str, progress: float)",
    ))
}

/// Physics-based spring oscillation curve, matching Remotion spring().
#[pyfunction]
#[pyo3(signature = (
    frame,
    fps = 30.0,
    damping = 10.0,
    mass = 1.0,
    stiffness = 100.0,
    overshoot_clamping = false
))]
fn spring(
    frame: f64,
    fps: f64,
    damping: f64,
    mass: f64,
    stiffness: f64,
    overshoot_clamping: bool,
) -> PyResult<f64> {
    let config = dioxuscut_animation::SpringConfig {
        damping,
        mass,
        stiffness,
        overshoot_clamping,
    };
    dioxuscut_animation::spring_with_options(
        frame,
        fps,
        config,
        dioxuscut_animation::SpringOptions::default(),
    )
    .map_err(|e| PyValueError::new_err(format!("Spring error: {e}")))
}

/// Resolve a static asset file relative to project public or asset directories.
#[pyfunction]
fn static_file(path: &str) -> PyResult<String> {
    dioxuscut_media::static_file(path)
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| PyValueError::new_err(format!("static_file error: {e}")))
}

/// Probe video metadata including resolution, fps, duration, and aspect ratio.
#[pyfunction]
fn get_video_metadata(py: Python<'_>, path: &str) -> PyResult<PyObject> {
    let meta = dioxuscut_media::get_video_metadata(path)
        .map_err(|e| PyValueError::new_err(format!("get_video_metadata error: {e}")))?;

    let dict = pyo3::types::PyDict::new_bound(py);
    dict.set_item("width", meta.width)?;
    dict.set_item("height", meta.height)?;
    dict.set_item("fps", meta.fps)?;
    dict.set_item("duration_in_seconds", meta.duration_in_seconds)?;
    dict.set_item("duration_in_frames", meta.duration_in_frames)?;
    dict.set_item("aspect_ratio", meta.aspect_ratio)?;
    dict.set_item("is_landscape", meta.is_landscape)?;
    Ok(dict.into())
}

fn tokens_to_py_list(
    py: Python<'_>,
    tokens: &[dioxuscut_captions::CaptionToken],
) -> PyResult<Vec<PyObject>> {
    let mut list = Vec::new();
    for t in tokens {
        let dict = pyo3::types::PyDict::new_bound(py);
        dict.set_item("text", &t.text)?;
        dict.set_item("start_ms", t.start_ms)?;
        dict.set_item("end_ms", t.end_ms)?;
        dict.set_item("start", t.start_ms as f64 / 1000.0)?;
        dict.set_item("end", t.end_ms as f64 / 1000.0)?;
        list.push(dict.into());
    }
    Ok(list)
}

/// Parse OpenAI / Faster-Whisper JSON into a list of word tokens.
#[pyfunction]
fn parse_whisper(py: Python<'_>, json_str: &str) -> PyResult<Vec<PyObject>> {
    let tokens = dioxuscut_captions::parse_whisper_json(json_str)
        .map_err(|e| PyValueError::new_err(format!("Whisper parse error: {e}")))?;
    tokens_to_py_list(py, &tokens)
}

/// Parse SRT subtitle file content into a list of word tokens.
#[pyfunction]
fn parse_srt(py: Python<'_>, srt_str: &str) -> PyResult<Vec<PyObject>> {
    let tokens = dioxuscut_captions::parse_srt(srt_str)
        .map_err(|e| PyValueError::new_err(format!("SRT parse error: {e}")))?;
    tokens_to_py_list(py, &tokens)
}

/// Parse WebVTT subtitle file content into a list of word tokens.
#[pyfunction]
fn parse_vtt(py: Python<'_>, vtt_str: &str) -> PyResult<Vec<PyObject>> {
    let tokens = dioxuscut_captions::parse_vtt(vtt_str)
        .map_err(|e| PyValueError::new_err(format!("VTT parse error: {e}")))?;
    tokens_to_py_list(py, &tokens)
}

/// Calculate background music auto-ducking volume keyframes from speech intervals.
#[pyfunction]
#[pyo3(signature = (
    intervals,
    total_duration,
    base_volume = 0.8,
    duck_volume = 0.15,
    attack_sec = 0.25,
    release_sec = 0.40,
    hold_threshold_sec = 0.35
))]
fn calculate_ducking(
    intervals: Vec<(f64, f64)>,
    total_duration: f64,
    base_volume: f64,
    duck_volume: f64,
    attack_sec: f64,
    release_sec: f64,
    hold_threshold_sec: f64,
) -> Vec<(f64, f64)> {
    let speech_intervals: Vec<dioxuscut_media::SpeechInterval> = intervals
        .into_iter()
        .map(|(s, e)| dioxuscut_media::SpeechInterval::new(s, e))
        .collect();

    let options = dioxuscut_media::DuckingOptions {
        base_volume,
        duck_volume,
        attack_sec,
        release_sec,
        hold_threshold_sec,
    };

    dioxuscut_media::calculate_ducking_envelope(&speech_intervals, total_duration, &options)
}

/// Compute frequency spectrum band magnitudes for an audio file at a given timestamp.
#[pyfunction]
#[pyo3(signature = (src, time_secs, n_bars = 32))]
fn get_audio_spectrum(src: &str, time_secs: f64, n_bars: usize) -> PyResult<Vec<f32>> {
    let path = std::path::Path::new(src);
    let data = dioxuscut_rasterizer::audio_cache::decode_audio_file(path)
        .map_err(|e| PyRuntimeError::new_err(format!("Failed to decode audio: {e}")))?;
    Ok(data.get_spectrum(time_secs, n_bars))
}

/// Python module initialization.
#[pymodule]
fn _dioxuscut(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(get_version, m)?)?;
    m.add_function(wrap_pyfunction!(list_compositions, m)?)?;
    m.add_function(wrap_pyfunction!(render_native, m)?)?;
    m.add_function(wrap_pyfunction!(random, m)?)?;
    m.add_function(wrap_pyfunction!(interpolate, m)?)?;
    m.add_function(wrap_pyfunction!(interpolate_colors, m)?)?;
    m.add_function(wrap_pyfunction!(spring, m)?)?;
    m.add_function(wrap_pyfunction!(static_file, m)?)?;
    m.add_function(wrap_pyfunction!(get_video_metadata, m)?)?;
    m.add_function(wrap_pyfunction!(parse_whisper, m)?)?;
    m.add_function(wrap_pyfunction!(parse_srt, m)?)?;
    m.add_function(wrap_pyfunction!(parse_vtt, m)?)?;
    m.add_function(wrap_pyfunction!(calculate_ducking, m)?)?;
    m.add_function(wrap_pyfunction!(get_audio_spectrum, m)?)?;
    Ok(())
}
