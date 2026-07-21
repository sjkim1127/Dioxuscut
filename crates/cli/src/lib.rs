//! Dioxuscut CLI library core options, argument parsing, validation, and command execution handlers.

use clap::{Parser, Subcommand};
use dioxuscut_renderer::{encode_frames, render_frames, spawn_server, EncodeConfig, RenderConfig};
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
    #[error("Invalid FPS: {0} must be greater than 0")]
    InvalidFps(String),
    #[error("Invalid duration: {0} must be greater than 0 frames")]
    InvalidDuration(u32),
}

/// Dioxuscut CLI — render videos from code
#[derive(Parser, Debug, Clone, PartialEq)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum Commands {
    /// Render a composition to a video file
    Render {
        /// Name of the composition to render
        #[arg(long, short)]
        composition: String,

        /// Path to a JSON file containing the input props
        #[arg(long, short)]
        props: Option<PathBuf>,

        /// Output video file path
        #[arg(long, short, default_value = "out.mp4")]
        output: PathBuf,

        /// Resolution width
        #[arg(long, default_value_t = 1920)]
        width: u32,

        /// Resolution height
        #[arg(long, default_value_t = 1080)]
        height: u32,

        /// Frames per second
        #[arg(long, default_value_t = 30.0)]
        fps: f64,

        /// Duration in frames
        #[arg(long, default_value_t = 150)]
        duration: u32,

        /// Port to bind web server (0 for dynamic port allocation)
        #[arg(long, default_value_t = 0)]
        port: u16,

        /// Path to web asset directory (default: "dist")
        #[arg(long)]
        web_dir: Option<PathBuf>,

        /// Optional external server URL (skips auto-spawning server if specified)
        #[arg(long)]
        server_url: Option<String>,
    },
}

/// Validates command-line parameters prior to launching web server and browser renderer.
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
    if let Some(p) = props {
        if !p.exists() {
            return Err(ValidationError::PropsFileNotFound(p.clone()));
        }
    }
    if width == 0 || height == 0 {
        return Err(ValidationError::InvalidZeroResolution(width, height));
    }
    if width % 2 != 0 || height % 2 != 0 {
        return Err(ValidationError::InvalidOddResolution(width, height));
    }
    if fps <= 0.0 {
        return Err(ValidationError::InvalidFps(fps.to_string()));
    }
    if duration == 0 {
        return Err(ValidationError::InvalidDuration(duration));
    }
    Ok(())
}

/// Executes the full render pipeline given the validated render options.
pub async fn execute_render_command(
    composition: &str,
    props: Option<&PathBuf>,
    output: &PathBuf,
    width: u32,
    height: u32,
    fps: f64,
    duration: u32,
    port: u16,
    web_dir: Option<&PathBuf>,
    server_url: Option<String>,
) -> anyhow::Result<()> {
    validate_render_params(composition, props, width, height, fps, duration)?;

    tracing::info!("Starting render for composition '{}'", composition);

    // 1. Read props JSON if specified
    let props_json = if let Some(p) = props {
        fs::read_to_string(p)?
    } else {
        "{}".to_string()
    };

    // 2. Set environment variable for Dioxus web app consumption
    std::env::set_var("DIOXUSCUT_PROPS", &props_json);

    // 3. Spawns web server or uses provided external server URL
    let server_handle = if let Some(ref custom_url) = server_url {
        tracing::info!("Using external server URL: {}", custom_url);
        None
    } else {
        let dir = web_dir.cloned().unwrap_or_else(|| PathBuf::from("dist"));
        tracing::info!(
            "Spawning web server for directory '{}' (requested port: {})",
            dir.display(),
            port
        );
        let handle = spawn_server(port, &dir).await?;
        tracing::info!("Web server ready at {}", handle.url());
        Some(handle)
    };

    let url = match &server_handle {
        Some(h) => h.url().to_string(),
        None => server_url.unwrap(),
    };

    let out_dir = std::env::temp_dir().join(format!(
        "dioxuscut_render_frames_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    if out_dir.exists() {
        fs::remove_dir_all(&out_dir)?;
    }

    let render_cfg = RenderConfig::new(url, &out_dir, width, height, fps, duration);

    // 4. Render frames via Headless Chrome CDP
    render_frames(&render_cfg).await?;

    // 5. Encode frame sequence into MP4 video via FFmpeg
    let encode_cfg = EncodeConfig::h264(&out_dir, output, fps).with_resolution(width, height);
    encode_frames(&encode_cfg).await?;

    tracing::info!("Successfully rendered video to {}", output.display());

    // Clean up frame directory
    if out_dir.exists() {
        let _ = fs::remove_dir_all(&out_dir);
    }

    if let Some(handle) = server_handle {
        handle.stop().await?;
    }

    Ok(())
}
