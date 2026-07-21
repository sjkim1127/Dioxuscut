//! CLI options, validation, native composition registry, and render execution.

pub mod composition;

pub use composition::{
    built_in_registry, CompositionRegistry, CompositionRegistryError, HelloWorldComposition,
    NativeComposition, NativeCompositionContext,
};

use clap::{Parser, Subcommand, ValueEnum};
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

/// Error types for CLI input parameter validation.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum ValidationError {
    #[error("Composition name cannot be empty")]
    EmptyComposition,
    #[error("Props file not found: {0}")]
    PropsFileNotFound(PathBuf),
    #[error("Invalid resolution: width ({0}) and height ({1}) must be greater than 0")]
    InvalidZeroResolution(u32, u32),
    #[error("Invalid resolution: width ({0}) and height ({1}) must be even numbers for H.264 video encoding")]
    InvalidOddResolution(u32, u32),
    #[error("Invalid FPS: {0} must be a finite number greater than 0")]
    InvalidFps(String),
    #[error("Invalid duration: {0} must be greater than 0 frames")]
    InvalidDuration(u32),
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

/// Dioxuscut CLI — render registered native compositions to video.
#[derive(Parser, Debug, Clone, PartialEq)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum Commands {
    /// Render a registered native composition to a video file.
    Render {
        /// ID of the composition to render.
        #[arg(long, short)]
        composition: String,

        /// Path to a JSON file containing input props.
        #[arg(long, short)]
        props: Option<PathBuf>,

        /// Output video file path.
        #[arg(long, short, default_value = "out.mp4")]
        output: PathBuf,

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
    },
}

/// A validated render request independent from argument parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderRequest {
    pub composition: String,
    pub props: Option<PathBuf>,
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: u32,
    pub backend: RenderBackend,
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
    if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
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

/// Execute a render using the compositions shipped with the standalone CLI.
pub async fn execute_render_command(request: &RenderRequest) -> anyhow::Result<()> {
    let registry = built_in_registry();
    execute_render_command_with_registry(request, &registry).await
}

/// Execute a render using an application-provided composition registry.
pub async fn execute_render_command_with_registry(
    request: &RenderRequest,
    registry: &CompositionRegistry,
) -> anyhow::Result<()> {
    validate_render_params(
        &request.composition,
        request.props.as_ref(),
        request.width,
        request.height,
        request.fps,
        request.duration,
    )?;

    let composition = registry.get(&request.composition)?;
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

    tracing::info!(
        composition = composition.id(),
        backend = ?request.backend,
        "Starting browser-free native render"
    );

    match request.backend {
        RenderBackend::Native => {
            use dioxuscut_rasterizer::{render_to_ffmpeg_pipe, PipeConfig, TinySkiaBackend};

            let rasterizer = TinySkiaBackend::new();
            let pipe_config = PipeConfig::new(
                request.width,
                request.height,
                request.fps,
                request.duration,
                &request.output,
            );
            render_to_ffmpeg_pipe(&rasterizer, &pipe_config, |frame| {
                composition.render(frame, &props, context)
            })?;
        }
        RenderBackend::Gpu => {
            #[cfg(not(feature = "gpu"))]
            anyhow::bail!(
                "GPU backend is not compiled in. Rebuild with `--features gpu`:\n  \
                 cargo build -p dioxuscut-cli --features gpu"
            );

            #[cfg(feature = "gpu")]
            {
                use dioxuscut_rasterizer::{render_to_ffmpeg_pipe, PipeConfig, WgpuBackend};

                let rasterizer = WgpuBackend::new()
                    .map_err(|error| anyhow::anyhow!("GPU backend init failed: {error}"))?;
                let pipe_config = PipeConfig::new(
                    request.width,
                    request.height,
                    request.fps,
                    request.duration,
                    &request.output,
                );
                render_to_ffmpeg_pipe(&rasterizer, &pipe_config, |frame| {
                    composition.render(frame, &props, context)
                })?;
            }
        }
    }

    tracing::info!(output = %request.output.display(), "Render completed");
    Ok(())
}
