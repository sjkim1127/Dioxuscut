//! Dioxuscut CLI library core options, argument parsing, validation, and command execution handlers.

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
    #[error("Invalid FPS: {0} must be greater than 0")]
    InvalidFps(String),
    #[error("Invalid duration: {0} must be greater than 0 frames")]
    InvalidDuration(u32),
}

/// Native render backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum RenderBackend {
    /// Pure-Rust CPU rasterizer via tiny-skia — no browser or GPU required. Default.
    #[default]
    Native,
    /// GPU-accelerated rasterizer via wgpu (Vulkan/Metal/DX12). Requires `--features gpu`.
    Gpu,
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

        /// Port to bind web server (legacy compatibility)
        #[arg(long, default_value_t = 0)]
        port: u16,

        /// Path to web asset directory (legacy compatibility)
        #[arg(long)]
        web_dir: Option<PathBuf>,

        /// Optional external server URL (legacy compatibility)
        #[arg(long)]
        server_url: Option<String>,
    },
}

/// Validates command-line parameters prior to launching renderer.
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
    _port: u16,
    _web_dir: Option<&PathBuf>,
    _server_url: Option<String>,
) -> anyhow::Result<()> {
    validate_render_params(composition, props, width, height, fps, duration)?;

    tracing::info!(
        "Starting native browser-free render for composition '{}' (backend: {:?})",
        composition,
        backend
    );

    // Read props JSON if specified
    let props_json = if let Some(p) = props {
        fs::read_to_string(p)?
    } else {
        "{}".to_string()
    };

    match backend {
        // ── Native CPU rasterizer (tiny-skia + Rayon parallel + FFmpeg pipe) ──
        RenderBackend::Native => {
            use dioxuscut_rasterizer::{
                render_to_ffmpeg_pipe, Color, GradientStop, PipeConfig, Scene, SceneNode,
                TinySkiaBackend,
            };

            tracing::info!("Using native tiny-skia CPU rasterizer with Rayon parallel pipeline");

            let rasterizer = TinySkiaBackend::new();
            let pipe_config = PipeConfig::new(width, height, fps, duration, output);

            let prop_value: serde_json::Value =
                serde_json::from_str(&props_json).unwrap_or(serde_json::Value::Null);

            let bg_start = prop_value
                .get("background_start")
                .and_then(|v| v.as_str())
                .and_then(Color::from_hex)
                .unwrap_or(Color::rgb(15, 23, 42)); // #0f172a

            let bg_end = prop_value
                .get("background_end")
                .and_then(|v| v.as_str())
                .and_then(Color::from_hex)
                .unwrap_or(Color::rgb(30, 27, 75)); // #1e1b4b

            let raw_title = prop_value
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or(composition);

            // Filter out emojis/unsupported glyphs to prevent TTF font tofu boxes
            let title: String = raw_title
                .chars()
                .filter(|c| c.is_ascii())
                .collect::<String>()
                .trim()
                .to_string();

            let title = if title.is_empty() {
                "Dioxuscut".to_string()
            } else {
                title
            };

            let accent = prop_value
                .get("accent_color")
                .and_then(|v| v.as_str())
                .and_then(Color::from_hex)
                .unwrap_or(Color::rgb(108, 99, 255)); // #6c63ff

            render_to_ffmpeg_pipe(&rasterizer, &pipe_config, |frame| {
                let mut scene = Scene::new();
                let t = frame as f32 / duration as f32;
                let angle = 135.0 + t * 90.0;

                // 1. Dynamic background gradient
                scene.push(SceneNode::LinearGradient {
                    x: 0.0,
                    y: 0.0,
                    w: width as f32,
                    h: height as f32,
                    angle_deg: angle,
                    stops: vec![
                        GradientStop {
                            position: 0.0,
                            color: bg_start,
                        },
                        GradientStop {
                            position: 1.0,
                            color: bg_end,
                        },
                    ],
                });

                // 2. Animated background accent circles
                let center_x = width as f32 * 0.5;
                let center_y = height as f32 * 0.5;

                let r1 = (width.min(height) as f32 * 0.2) + (t * std::f32::consts::TAU).sin() * 20.0;
                scene.push(SceneNode::Circle {
                    cx: center_x,
                    cy: center_y,
                    r: r1,
                    fill: accent.with_opacity(0.12),
                    stroke: Some(accent),
                    stroke_width: 2.0,
                });

                let r2 = (width.min(height) as f32 * 0.3) + (t * std::f32::consts::PI).cos() * 30.0;
                scene.push(SceneNode::Circle {
                    cx: center_x,
                    cy: center_y,
                    r: r2,
                    fill: Color::TRANSPARENT,
                    stroke: Some(Color::rgba(0, 242, 254, 180)),
                    stroke_width: 1.5,
                });

                // 3. Motion graphics accent rects
                let rect_size = 80.0 + (t * std::f32::consts::TAU).sin() * 15.0;

                scene.push(SceneNode::Rect {
                    x: width as f32 * 0.15,
                    y: height as f32 * 0.2,
                    w: rect_size,
                    h: rect_size,
                    fill: Color::rgba(0, 242, 254, 40),
                    stroke: Some(Color::rgb(0, 242, 254)),
                    stroke_width: 2.0,
                    corner_radius: 12.0,
                });

                scene.push(SceneNode::Rect {
                    x: width as f32 * 0.78,
                    y: height as f32 * 0.65,
                    w: rect_size * 1.2,
                    h: rect_size * 1.2,
                    fill: Color::rgba(255, 230, 0, 30),
                    stroke: Some(Color::rgb(255, 230, 0)),
                    stroke_width: 2.0,
                    corner_radius: 16.0,
                });

                // 4. Progress indicator ring (growing bottom bar)
                let bar_h = 6.0;
                let bar_w = width as f32 * t;
                scene.push(SceneNode::Rect {
                    x: 0.0,
                    y: height as f32 - bar_h,
                    w: bar_w,
                    h: bar_h,
                    fill: Color::rgb(0, 242, 254),
                    stroke: None,
                    stroke_width: 0.0,
                    corner_radius: 0.0,
                });

                // 5. Main Title & Subtitle
                let font_size = (width as f32 * 0.045).max(28.0);
                let text_x = width as f32 * 0.12;
                let text_y = height as f32 * 0.45;
                scene.push(SceneNode::Text {
                    x: text_x,
                    y: text_y,
                    content: title.clone(),
                    font_size,
                    color: Color::rgb(255, 255, 255),
                    font_weight: 700,
                });

                let sub_size = font_size * 0.45;
                scene.push(SceneNode::Text {
                    x: text_x,
                    y: text_y + font_size * 0.8,
                    content: "Declarative Programmatic Video in Pure Rust".to_string(),
                    font_size: sub_size,
                    color: Color::rgb(0, 242, 254),
                    font_weight: 400,
                });

                scene
            })?;

            tracing::info!(
                "Native rasterizer: {} frames rendered directly to {}",
                duration,
                output.display()
            );
        }

        // ── wgpu GPU renderer (feature = "gpu") ──────────────────────────────
        RenderBackend::Gpu => {
            #[cfg(not(feature = "gpu"))]
            {
                anyhow::bail!("GPU backend is not compiled in. Rebuild with `--features gpu`:\n  cargo build -p dioxuscut-cli --features gpu");
            }

            #[cfg(feature = "gpu")]
            {
                use dioxuscut_rasterizer::{
                    render_to_ffmpeg_pipe, Color, GradientStop, PipeConfig, Scene, SceneNode,
                    WgpuBackend,
                };

                tracing::info!("Using wgpu GPU-accelerated rasterizer with zero-copy FFmpeg pipe");

                let rasterizer = WgpuBackend::new()
                    .map_err(|e| anyhow::anyhow!("GPU backend init failed: {e}"))?;
                let pipe_config = PipeConfig::new(width, height, fps, duration, output);

                let prop_value: serde_json::Value =
                    serde_json::from_str(&props_json).unwrap_or(serde_json::Value::Null);

                let bg_start = prop_value
                    .get("background_start")
                    .and_then(|v| v.as_str())
                    .and_then(Color::from_hex)
                    .unwrap_or(Color::rgb(15, 23, 42));
                let bg_end = prop_value
                    .get("background_end")
                    .and_then(|v| v.as_str())
                    .and_then(Color::from_hex)
                    .unwrap_or(Color::rgb(30, 27, 75));
                let accent = prop_value
                    .get("accent_color")
                    .and_then(|v| v.as_str())
                    .and_then(Color::from_hex)
                    .unwrap_or(Color::rgb(108, 99, 255));

                render_to_ffmpeg_pipe(&rasterizer, &pipe_config, |frame| {
                    let mut scene = Scene::new();
                    let t = frame as f32 / duration as f32;
                    scene.push(SceneNode::LinearGradient {
                        x: 0.0,
                        y: 0.0,
                        w: width as f32,
                        h: height as f32,
                        angle_deg: 135.0,
                        stops: vec![
                            GradientStop {
                                position: 0.0,
                                color: bg_start,
                            },
                            GradientStop {
                                position: 1.0,
                                color: bg_end,
                            },
                        ],
                    });
                    let r =
                        (width.min(height) as f32 * 0.15) + t * (width.min(height) as f32 * 0.1);
                    scene.push(SceneNode::Circle {
                        cx: width as f32 * 0.5,
                        cy: height as f32 * 0.5,
                        r,
                        fill: accent.with_opacity(0.15 + t * 0.15),
                        stroke: Some(accent),
                        stroke_width: 3.0,
                    });
                    scene
                })?;

                tracing::info!(
                    "GPU rasterizer: {} frames rendered directly to {}",
                    duration,
                    output.display()
                );
            }
        }
    }

    tracing::info!("Successfully rendered video to {}", output.display());

    Ok(())
}
