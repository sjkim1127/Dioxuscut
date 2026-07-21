//! CLI options, validation, native composition registry, and render execution.

pub mod composition;
#[cfg(feature = "rhai")]
pub mod rhai_runtime;

pub use composition::{
    built_in_registry, Composition, CompositionError, CompositionRegistry,
    CompositionRegistryError, HelloWorldComposition, NativeComposition, NativeCompositionContext,
    PreparedComposition,
};
#[cfg(feature = "rhai")]
pub use rhai_runtime::{RhaiComposition, SceneBuilder};

use clap::{Parser, Subcommand, ValueEnum};
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

/// Error types for CLI input parameter validation.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum ValidationError {
    #[error("Composition name cannot be empty")]
    EmptyComposition,
    #[error("Provide exactly one composition source: --composition <ID> or --script <PATH>")]
    MissingCompositionSource,
    #[error("--composition and --script cannot be used together")]
    ConflictingCompositionSources,
    #[error("Rhai script file not found: {0}")]
    ScriptFileNotFound(PathBuf),
    #[error("Props file not found: {0}")]
    PropsFileNotFound(PathBuf),
    #[error("Audio file not found: {0}")]
    AudioFileNotFound(PathBuf),
    #[error("Invalid resolution: width ({0}) and height ({1}) must be greater than 0")]
    InvalidZeroResolution(u32, u32),
    #[error(
        "Invalid resolution: width ({0}) and height ({1}) must be even numbers for video encoding"
    )]
    InvalidOddResolution(u32, u32),
    #[error("Invalid FPS: {0} must be a finite number greater than 0")]
    InvalidFps(String),
    #[error("Invalid duration: {0} must be greater than 0 frames")]
    InvalidDuration(u32),
    #[error("Invalid frame range: start {start}, end {end}, composition duration {duration}")]
    InvalidFrameRange { start: u32, end: u32, duration: u32 },
    #[error("Output extension '.{actual}' is invalid for {codec}; expected {expected}")]
    InvalidOutputExtension {
        codec: String,
        actual: String,
        expected: String,
    },
    #[error("Audio tracks are not supported for {0} output")]
    AudioNotSupported(String),
    #[error("Timeout must be greater than zero seconds")]
    InvalidTimeout,
    #[error("Invalid CRF {value} for {codec}; expected {range}")]
    InvalidCrf {
        codec: String,
        value: u32,
        range: String,
    },
    #[error("Invalid encoder preset '{0}'; expected ultrafast, superfast, veryfast, faster, fast, medium, slow, slower, veryslow, or placebo")]
    InvalidPreset(String),
}

/// Native render backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum RenderBackend {
    /// Pure-Rust CPU rasterizer via tiny-skia. Default.
    #[default]
    Native,
    /// GPU rasterizer via wgpu. Requires `--features gpu`.
    Gpu,
}

/// Output codec or still-image format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum RenderCodec {
    #[default]
    H264,
    H265,
    Vp9,
    Av1,
    #[value(name = "prores", alias = "pro-res")]
    ProRes,
    Gif,
    Png,
    Jpeg,
    Webp,
}

impl RenderCodec {
    fn still_format(self) -> Option<dioxuscut_rasterizer::StillImageFormat> {
        match self {
            Self::Png => Some(dioxuscut_rasterizer::StillImageFormat::Png),
            Self::Jpeg => Some(dioxuscut_rasterizer::StillImageFormat::Jpeg),
            Self::Webp => Some(dioxuscut_rasterizer::StillImageFormat::WebP),
            _ => None,
        }
    }

    fn video_codec(self) -> Option<dioxuscut_rasterizer::VideoCodec> {
        match self {
            Self::H264 => Some(dioxuscut_rasterizer::VideoCodec::H264),
            Self::H265 => Some(dioxuscut_rasterizer::VideoCodec::H265),
            Self::Vp9 => Some(dioxuscut_rasterizer::VideoCodec::Vp9),
            Self::Av1 => Some(dioxuscut_rasterizer::VideoCodec::Av1),
            Self::ProRes => Some(dioxuscut_rasterizer::VideoCodec::ProRes),
            Self::Gif => Some(dioxuscut_rasterizer::VideoCodec::Gif),
            Self::Png | Self::Jpeg | Self::Webp => None,
        }
    }

    fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::H264 | Self::H265 => &["mp4"],
            Self::Vp9 | Self::Av1 => &["webm"],
            Self::ProRes => &["mov"],
            Self::Gif => &["gif"],
            Self::Png => &["png"],
            Self::Jpeg => &["jpg", "jpeg"],
            Self::Webp => &["webp"],
        }
    }
}

/// Dioxuscut CLI — render registered Rust or Rhai compositions to video.
#[derive(Parser, Debug, Clone, PartialEq)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum Commands {
    /// Render a registered Rust composition or Rhai script to a media file.
    Render {
        /// ID of the composition to render.
        #[arg(
            long,
            short,
            required_unless_present = "script",
            conflicts_with = "script"
        )]
        composition: Option<String>,

        /// Path to a Rhai composition script. Requires the `rhai` feature.
        #[arg(
            long,
            required_unless_present = "composition",
            conflicts_with = "composition"
        )]
        script: Option<PathBuf>,

        /// Path to a JSON file containing input props.
        #[arg(long, short)]
        props: Option<PathBuf>,

        /// Output media file path.
        #[arg(long, short, default_value = "out.mp4")]
        output: PathBuf,

        /// Local audio file to mix into the output. May be repeated.
        #[arg(long = "audio", value_name = "PATH")]
        audio: Vec<PathBuf>,

        /// Resolution width.
        #[arg(long, default_value_t = 1920)]
        width: u32,

        /// Resolution height.
        #[arg(long, default_value_t = 1080)]
        height: u32,

        /// Frames per second.
        #[arg(long, default_value_t = 30.0)]
        fps: f64,

        /// Duration in frames.
        #[arg(long, default_value_t = 150)]
        duration: u32,

        /// Rendering backend.
        #[arg(long, value_enum, default_value_t = RenderBackend::Native)]
        backend: RenderBackend,

        /// Video codec or still-image output format.
        #[arg(long, value_enum, default_value_t = RenderCodec::H264)]
        codec: RenderCodec,

        /// First composition frame to render.
        #[arg(long, default_value_t = 0)]
        frame_start: u32,

        /// Last composition frame to render, inclusive.
        #[arg(long)]
        frame_end: Option<u32>,

        /// Abort the render after this many seconds.
        #[arg(long)]
        timeout_seconds: Option<u64>,

        /// Codec quality value. Lower is generally higher quality.
        #[arg(long, default_value_t = 18)]
        crf: u32,

        /// FFmpeg encoder preset for H.264 and H.265.
        #[arg(long, default_value = "fast")]
        preset: String,
    },
}

/// A validated render request independent from argument parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderRequest {
    pub composition: Option<String>,
    pub script: Option<PathBuf>,
    pub props: Option<PathBuf>,
    pub output: PathBuf,
    pub audio: Vec<PathBuf>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: u32,
    pub backend: RenderBackend,
    pub codec: RenderCodec,
    pub frame_start: u32,
    pub frame_end: Option<u32>,
    pub timeout_seconds: Option<u64>,
    pub crf: u32,
    pub preset: String,
}

/// Validates that a render request selects exactly one available composition source.
pub fn validate_composition_source(
    composition: Option<&str>,
    script: Option<&PathBuf>,
) -> Result<(), ValidationError> {
    match (composition, script) {
        (None, None) => Err(ValidationError::MissingCompositionSource),
        (Some(_), Some(_)) => Err(ValidationError::ConflictingCompositionSources),
        (Some(composition), None) if composition.trim().is_empty() => {
            Err(ValidationError::EmptyComposition)
        }
        (Some(_), None) => Ok(()),
        (None, Some(path)) if !path.is_file() => {
            Err(ValidationError::ScriptFileNotFound(path.clone()))
        }
        (None, Some(_)) => Ok(()),
    }
}

/// Validates command-line parameters prior to launching the renderer.
pub fn validate_render_params(
    composition: &str,
    props: Option<&PathBuf>,
    width: u32,
    height: u32,
    fps: f64,
    duration: u32,
) -> Result<(), ValidationError> {
    validate_render_params_for_codec(
        composition,
        props,
        width,
        height,
        fps,
        duration,
        RenderCodec::H264,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_render_params_for_codec(
    composition: &str,
    props: Option<&PathBuf>,
    width: u32,
    height: u32,
    fps: f64,
    duration: u32,
    codec: RenderCodec,
) -> Result<(), ValidationError> {
    if composition.trim().is_empty() {
        return Err(ValidationError::EmptyComposition);
    }
    if let Some(path) = props {
        if !path.is_file() {
            return Err(ValidationError::PropsFileNotFound(path.clone()));
        }
    }
    if width == 0 || height == 0 {
        return Err(ValidationError::InvalidZeroResolution(width, height));
    }
    let requires_even_dimensions = matches!(
        codec,
        RenderCodec::H264
            | RenderCodec::H265
            | RenderCodec::Vp9
            | RenderCodec::Av1
            | RenderCodec::ProRes
    );
    if requires_even_dimensions && (!width.is_multiple_of(2) || !height.is_multiple_of(2)) {
        return Err(ValidationError::InvalidOddResolution(width, height));
    }
    if !fps.is_finite() || fps <= 0.0 {
        return Err(ValidationError::InvalidFps(fps.to_string()));
    }
    if duration == 0 {
        return Err(ValidationError::InvalidDuration(duration));
    }
    Ok(())
}

fn validate_render_options(request: &RenderRequest) -> Result<(u32, u32), ValidationError> {
    let end = request.frame_end.unwrap_or_else(|| {
        if request.codec.still_format().is_some() {
            request.frame_start
        } else {
            request.duration.saturating_sub(1)
        }
    });
    if request.frame_start > end || end >= request.duration {
        return Err(ValidationError::InvalidFrameRange {
            start: request.frame_start,
            end,
            duration: request.duration,
        });
    }
    if request.timeout_seconds == Some(0) {
        return Err(ValidationError::InvalidTimeout);
    }
    if request.codec.still_format().is_some() && end != request.frame_start {
        return Err(ValidationError::InvalidFrameRange {
            start: request.frame_start,
            end,
            duration: request.duration,
        });
    }
    let max_crf = match request.codec {
        RenderCodec::H264 | RenderCodec::H265 => Some(51),
        RenderCodec::Vp9 | RenderCodec::Av1 => Some(63),
        RenderCodec::ProRes
        | RenderCodec::Gif
        | RenderCodec::Png
        | RenderCodec::Jpeg
        | RenderCodec::Webp => None,
    };
    if max_crf.is_some_and(|max| request.crf > max) {
        return Err(ValidationError::InvalidCrf {
            codec: format!("{:?}", request.codec),
            value: request.crf,
            range: format!("0..={}", max_crf.expect("checked above")),
        });
    }
    if matches!(request.codec, RenderCodec::H264 | RenderCodec::H265)
        && !matches!(
            request.preset.as_str(),
            "ultrafast"
                | "superfast"
                | "veryfast"
                | "faster"
                | "fast"
                | "medium"
                | "slow"
                | "slower"
                | "veryslow"
                | "placebo"
        )
    {
        return Err(ValidationError::InvalidPreset(request.preset.clone()));
    }
    let actual = request
        .output
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let expected = request.codec.extensions();
    if !expected.contains(&actual.as_str()) {
        return Err(ValidationError::InvalidOutputExtension {
            codec: format!("{:?}", request.codec),
            actual,
            expected: expected.join(" or ."),
        });
    }
    if !request.audio.is_empty()
        && (request.codec.still_format().is_some() || request.codec == RenderCodec::Gif)
    {
        return Err(ValidationError::AudioNotSupported(format!(
            "{:?}",
            request.codec
        )));
    }
    Ok((request.frame_start, end))
}

/// Execute a render using the compositions shipped with the standalone CLI.
pub async fn execute_render_command(request: &RenderRequest) -> anyhow::Result<()> {
    let registry = built_in_registry();
    execute_render_command_with_registry_and_control(
        request,
        &registry,
        default_render_control(request),
    )
    .await
}

/// Execute a render using an application-provided composition registry.
pub async fn execute_render_command_with_registry(
    request: &RenderRequest,
    registry: &CompositionRegistry,
) -> anyhow::Result<()> {
    execute_render_command_with_registry_and_control(
        request,
        registry,
        default_render_control(request),
    )
    .await
}

/// Build the standard CLI progress and timeout controls for a render request.
pub fn default_render_control(request: &RenderRequest) -> dioxuscut_rasterizer::RenderControl {
    let mut control = dioxuscut_rasterizer::RenderControl::new().with_progress(|progress| {
        let total = u64::from(progress.total_frames.max(1));
        let completed = u64::from(progress.completed_frames);
        let percent = completed.saturating_mul(100) / total;
        let previous_percent = completed.saturating_sub(1).saturating_mul(100) / total;
        if completed == 1 || completed == total || percent != previous_percent {
            tracing::info!(
                completed = progress.completed_frames,
                total = progress.total_frames,
                frame = progress.frame,
                percent,
                "Render progress"
            );
        }
    });
    if let Some(seconds) = request.timeout_seconds {
        control = control.with_timeout(std::time::Duration::from_secs(seconds));
    }
    control
}

/// Execute a render with caller-owned progress, cancellation, and timeout controls.
pub async fn execute_render_command_with_control(
    request: &RenderRequest,
    control: dioxuscut_rasterizer::RenderControl,
) -> anyhow::Result<()> {
    let registry = built_in_registry();
    execute_render_command_with_registry_and_control(request, &registry, control).await
}

/// Execute an application-provided composition registry with caller-owned controls.
pub async fn execute_render_command_with_registry_and_control(
    request: &RenderRequest,
    registry: &CompositionRegistry,
    control: dioxuscut_rasterizer::RenderControl,
) -> anyhow::Result<()> {
    validate_composition_source(request.composition.as_deref(), request.script.as_ref())?;
    validate_render_params_for_codec(
        request.composition.as_deref().unwrap_or("RhaiScript"),
        request.props.as_ref(),
        request.width,
        request.height,
        request.fps,
        request.duration,
        request.codec,
    )?;
    let (frame_start, frame_end) = validate_render_options(request)?;
    let frame_count = frame_end - frame_start + 1;
    for path in &request.audio {
        if !path.is_file() {
            return Err(ValidationError::AudioFileNotFound(path.clone()).into());
        }
    }

    let props = match &request.props {
        Some(path) => {
            let json = fs::read_to_string(path)?;
            serde_json::from_str(&json).map_err(|error| {
                anyhow::anyhow!("Invalid props JSON in {}: {error}", path.display())
            })?
        }
        None => serde_json::Value::Object(Default::default()),
    };
    let context = NativeCompositionContext {
        width: request.width,
        height: request.height,
        fps: request.fps,
        duration_in_frames: request.duration,
    };

    #[cfg(feature = "rhai")]
    let script_composition = request
        .script
        .as_deref()
        .map(RhaiComposition::from_file)
        .transpose()?;

    #[cfg(feature = "rhai")]
    let composition: &dyn Composition = match script_composition.as_ref() {
        Some(composition) => composition,
        None => registry.get(
            request
                .composition
                .as_deref()
                .expect("validated native composition ID"),
        )?,
    };

    #[cfg(not(feature = "rhai"))]
    let composition: &dyn Composition = {
        if request.script.is_some() {
            anyhow::bail!(
                "Rhai support is not compiled in. Rebuild with `--features rhai`:\n  \
                 cargo build -p dioxuscut-cli --features rhai"
            );
        }
        registry.get(
            request
                .composition
                .as_deref()
                .expect("validated native composition ID"),
        )?
    };

    let prepared = composition.prepare(&props, context)?;

    // Validate the first frame before starting FFmpeg. Dynamic compositions
    // therefore report syntax, type, and API errors without creating an output.
    let first_scene = prepared.render(0)?;
    let mut audio_tracks = first_scene.audio_tracks();
    audio_tracks.extend(
        request
            .audio
            .iter()
            .map(|path| dioxuscut_rasterizer::AudioTrack::new(path.to_string_lossy().into_owned())),
    );

    tracing::info!(
        composition = composition.id(),
        backend = ?request.backend,
        codec = ?request.codec,
        frame_start,
        frame_end,
        "Starting browser-free native render"
    );

    match request.backend {
        RenderBackend::Native => {
            use dioxuscut_rasterizer::{
                render_still_fallible, render_to_ffmpeg_pipe_fallible, PipeConfig, TinySkiaBackend,
            };

            let rasterizer = TinySkiaBackend::new();
            if let Some(format) = request.codec.still_format() {
                render_still_fallible(
                    &rasterizer,
                    request.width,
                    request.height,
                    request.fps,
                    frame_start,
                    &request.output,
                    format,
                    &control,
                    |frame| prepared.render(frame),
                )?;
            } else {
                let pipe_config = PipeConfig::new(
                    request.width,
                    request.height,
                    request.fps,
                    frame_count,
                    &request.output,
                )
                .with_frame_start(frame_start)
                .with_codec(request.codec.video_codec().expect("video codec validated"))
                .with_quality(request.crf, &request.preset)
                .with_audio_tracks(audio_tracks.clone())
                .with_control(control.clone());
                render_to_ffmpeg_pipe_fallible(&rasterizer, &pipe_config, |frame| {
                    prepared.render(frame)
                })?;
            }
        }
        RenderBackend::Gpu => {
            #[cfg(not(feature = "gpu"))]
            anyhow::bail!(
                "GPU backend is not compiled in. Rebuild with `--features gpu`:\n  \
                 cargo build -p dioxuscut-cli --features gpu"
            );

            #[cfg(feature = "gpu")]
            {
                use dioxuscut_rasterizer::{
                    render_still_fallible, render_to_ffmpeg_pipe_fallible, PipeConfig, WgpuBackend,
                };

                let rasterizer = WgpuBackend::new()
                    .map_err(|error| anyhow::anyhow!("GPU backend init failed: {error}"))?;
                if let Some(format) = request.codec.still_format() {
                    render_still_fallible(
                        &rasterizer,
                        request.width,
                        request.height,
                        request.fps,
                        frame_start,
                        &request.output,
                        format,
                        &control,
                        |frame| prepared.render(frame),
                    )?;
                } else {
                    let pipe_config = PipeConfig::new(
                        request.width,
                        request.height,
                        request.fps,
                        frame_count,
                        &request.output,
                    )
                    .with_frame_start(frame_start)
                    .with_codec(request.codec.video_codec().expect("video codec validated"))
                    .with_quality(request.crf, &request.preset)
                    .with_audio_tracks(audio_tracks.clone())
                    .with_control(control.clone());
                    render_to_ffmpeg_pipe_fallible(&rasterizer, &pipe_config, |frame| {
                        prepared.render(frame)
                    })?;
                }
            }
        }
    }

    tracing::info!(output = %request.output.display(), "Render completed");
    Ok(())
}
