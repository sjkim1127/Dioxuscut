//! Dioxuscut CLI library core options, argument parsing, validation, and command execution handlers.

use clap::{Parser, Subcommand, ValueEnum};
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

/// Render backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum RenderBackend {
    /// Pure-Rust CPU rasterizer — no browser or GPU required. Default.
    #[default]
    Native,
    /// GPU-accelerated rasterizer via wgpu (Vulkan/Metal/DX12). Requires `--features gpu`.
    Gpu,
    /// Headless Chrome CDP renderer (Phase 2/3 behaviour). Requires Chrome installed.
    Chrome,
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

        /// Rendering backend
        #[arg(long, value_enum, default_value_t = RenderBackend::Native)]
        backend: RenderBackend,

        /// Port to bind web server (0 for dynamic port allocation). Only used with --backend chrome.
        #[arg(long, default_value_t = 0)]
        port: u16,

        /// Path to web asset directory. Only used with --backend chrome.
        #[arg(long)]
        web_dir: Option<PathBuf>,

        /// Optional external server URL. Only used with --backend chrome.
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
    backend: RenderBackend,
    port: u16,
    web_dir: Option<&PathBuf>,
    server_url: Option<String>,
) -> anyhow::Result<()> {
    validate_render_params(composition, props, width, height, fps, duration)?;

    tracing::info!(
        "Starting render for composition '{}' (backend: {:?})",
        composition, backend
    );

    // 1. Read props JSON if specified
    let props_json = if let Some(p) = props {
        fs::read_to_string(p)?
    } else {
        "{}".to_string()
    };

    // 2. Shared temp output dir
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

    match backend {
        // ── Native rasterizer (Phase 4) ──────────────────────────────────────
        RenderBackend::Native => {
            use dioxuscut_rasterizer::{
                TinySkiaBackend, NativeRenderConfig, Scene, SceneNode, Color,
                GradientStop, render_all_frames,
            };

            tracing::info!("Using native tiny-skia CPU rasterizer");

            let rasterizer = TinySkiaBackend::new();
            let native_config = NativeRenderConfig::new(width, height, fps, duration, &out_dir);

            // Parse props JSON for background colours (fallback to defaults)
            let prop_value: serde_json::Value =
                serde_json::from_str(&props_json).unwrap_or(serde_json::Value::Null);

            let bg_start = prop_value
                .get("background_start")
                .and_then(|v| v.as_str())
                .and_then(Color::from_hex)
                .unwrap_or(Color::rgb(15, 23, 42));     // #0f172a

            let bg_end = prop_value
                .get("background_end")
                .and_then(|v| v.as_str())
                .and_then(Color::from_hex)
                .unwrap_or(Color::rgb(30, 27, 75));     // #1e1b4b

            let title = prop_value
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or(composition)
                .to_string();

            let accent = prop_value
                .get("accent_color")
                .and_then(|v| v.as_str())
                .and_then(Color::from_hex)
                .unwrap_or(Color::rgb(108, 99, 255));   // #6c63ff

            render_all_frames(&rasterizer, &native_config, |frame| {
                let mut scene = Scene::new();

                // Background gradient
                scene.push(SceneNode::LinearGradient {
                    x: 0.0, y: 0.0,
                    w: width as f32,
                    h: height as f32,
                    angle_deg: 135.0,
                    stops: vec![
                        GradientStop { position: 0.0, color: bg_start },
                        GradientStop { position: 1.0, color: bg_end },
                    ],
                });

                // Animated accent circle (grows with frame)
                let t = frame as f32 / duration as f32;
                let r = (width.min(height) as f32 * 0.15) + t * (width.min(height) as f32 * 0.1);
                scene.push(SceneNode::Circle {
                    cx: width as f32 * 0.5,
                    cy: height as f32 * 0.5,
                    r,
                    fill: accent.with_opacity(0.15 + t * 0.15),
                    stroke: Some(accent),
                    stroke_width: 3.0,
                });

                // Title text placeholder
                let font_size = (width as f32 * 0.04).max(24.0);
                let text_x = width as f32 * 0.1;
                let text_y = height as f32 * 0.5 + font_size / 2.0;
                scene.push(SceneNode::Text {
                    x: text_x,
                    y: text_y,
                    content: title.clone(),
                    font_size,
                    color: Color::rgb(255, 255, 255),
                    font_weight: 700,
                });

                scene
            })?;

            tracing::info!("Native rasterizer: {} frames written to {:?}", duration, out_dir);
        }

        // ── wgpu GPU renderer (Phase 4, feature = "gpu") ─────────────────────
        RenderBackend::Gpu => {
            #[cfg(not(feature = "gpu"))]
            {
                anyhow::bail!("GPU backend is not compiled in. Rebuild with `--features gpu`:\n  cargo build -p dioxuscut-cli --features dioxuscut-rasterizer/gpu");
            }

            #[cfg(feature = "gpu")]
            {
                use dioxuscut_rasterizer::{
                    WgpuBackend, NativeRenderConfig, Scene, SceneNode, Color,
                    GradientStop, render_all_frames,
                };

                tracing::info!("Using wgpu GPU-accelerated rasterizer");

                let rasterizer = WgpuBackend::new()
                    .map_err(|e| anyhow::anyhow!("GPU backend init failed: {e}"))?;
                let native_config = NativeRenderConfig::new(width, height, fps, duration, &out_dir);

                let prop_value: serde_json::Value =
                    serde_json::from_str(&props_json).unwrap_or(serde_json::Value::Null);

                let bg_start = prop_value.get("background_start")
                    .and_then(|v| v.as_str()).and_then(Color::from_hex)
                    .unwrap_or(Color::rgb(15, 23, 42));
                let bg_end = prop_value.get("background_end")
                    .and_then(|v| v.as_str()).and_then(Color::from_hex)
                    .unwrap_or(Color::rgb(30, 27, 75));
                let accent = prop_value.get("accent_color")
                    .and_then(|v| v.as_str()).and_then(Color::from_hex)
                    .unwrap_or(Color::rgb(108, 99, 255));

                render_all_frames(&rasterizer, &native_config, |frame| {
                    let mut scene = Scene::new();
                    let t = frame as f32 / duration as f32;
                    scene.push(SceneNode::LinearGradient {
                        x: 0.0, y: 0.0, w: width as f32, h: height as f32,
                        angle_deg: 135.0,
                        stops: vec![
                            GradientStop { position: 0.0, color: bg_start },
                            GradientStop { position: 1.0, color: bg_end },
                        ],
                    });
                    let r = (width.min(height) as f32 * 0.15) + t * (width.min(height) as f32 * 0.1);
                    scene.push(SceneNode::Circle {
                        cx: width as f32 * 0.5, cy: height as f32 * 0.5, r,
                        fill: accent.with_opacity(0.15 + t * 0.15),
                        stroke: Some(accent), stroke_width: 3.0,
                    });
                    scene
                })?;

                tracing::info!("GPU rasterizer: {} frames written to {:?}", duration, out_dir);
            }
        }

        // ── Chrome CDP renderer (Phase 2/3 legacy) ───────────────────────────
        RenderBackend::Chrome => {
            tracing::info!("Using Headless Chrome CDP renderer");

            // Set environment variable for Dioxus web app consumption
            std::env::set_var("DIOXUSCUT_PROPS", &props_json);

            let server_handle = if let Some(ref custom_url) = server_url {
                tracing::info!("Using external server URL: {}", custom_url);
                None
            } else {
                let dir = web_dir.cloned().unwrap_or_else(|| PathBuf::from("dist"));
                tracing::info!(
                    "Spawning web server for directory '{}' (port: {})",
                    dir.display(), port
                );
                let handle = spawn_server(port, &dir).await?;
                tracing::info!("Web server ready at {}", handle.url());
                Some(handle)
            };

            let url = match &server_handle {
                Some(h) => h.url().to_string(),
                None => server_url.unwrap(),
            };

            let render_cfg = RenderConfig::new(url, &out_dir, width, height, fps, duration);
            render_frames(&render_cfg).await?;

            if let Some(handle) = server_handle {
                handle.stop().await?;
            }
        }
    }

    // 3. Encode frame sequence into MP4 via FFmpeg (both backends)
    let encode_cfg = EncodeConfig::h264(&out_dir, output, fps).with_resolution(width, height);
    encode_frames(&encode_cfg).await?;

    tracing::info!("Successfully rendered video to {}", output.display());

    if out_dir.exists() {
        let _ = fs::remove_dir_all(&out_dir);
    }

    Ok(())
}
