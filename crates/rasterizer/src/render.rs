//! Frame rendering coordinator — sequential, parallel, and streaming pipeline modes.
//!
//! # Render Modes
//!
//! | Mode                     | I/O               | Parallelism | Best for                         |
//! |--------------------------|-------------------|-------------|----------------------------------|
//! | [`render_all_frames`]    | PNG files on disk | Sequential  | Debugging, inspection            |
//! | [`render_parallel`]      | PNG files on disk | Rayon (N cores) | Large renders with disk I/O  |
//! | [`render_to_ffmpeg_pipe`]| FFmpeg stdin      | Rayon + pipe| **Fastest** — zero PNG overhead  |
//!
//! ## Pipeline comparison
//!
//! ```text
//! Sequential PNG:   [frame 0] → PNG → [frame 1] → PNG → … → FFmpeg
//! Parallel PNG:     [frame 0]
//!                   [frame 1]  (all at once, Rayon)
//!                   [frame 2] → disk → FFmpeg
//!
//! Pipe (fastest):   Rayon renders → sorted RGBA chunks → FFmpeg stdin → MP4
//!                   Zero disk I/O, zero PNG compression overhead
//! ```

use crate::backend::{FrameConfig, RasterError, RasterizerBackend};
use crate::scene::Scene;
use image::RgbaImage;
use rayon::prelude::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

// ── Types ────────────────────────────────────────────────────────────────────

/// Configuration for a native render job.
#[derive(Debug, Clone)]
pub struct NativeRenderConfig {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_in_frames: u32,
    pub output_dir: PathBuf,
    /// Number of Rayon threads to use. `None` = auto (# logical CPUs).
    pub concurrency: Option<usize>,
}

impl NativeRenderConfig {
    pub fn new(
        width: u32,
        height: u32,
        fps: f64,
        duration_in_frames: u32,
        output_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            width,
            height,
            fps,
            duration_in_frames,
            output_dir: output_dir.into(),
            concurrency: None,
        }
    }

    pub fn with_concurrency(mut self, n: usize) -> Self {
        self.concurrency = Some(n);
        self
    }
}

/// Piped encoding configuration (no intermediate PNG files).
#[derive(Debug, Clone)]
pub struct PipeConfig {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_in_frames: u32,
    /// Output MP4 file path.
    pub output: PathBuf,
    /// Number of parallel render workers. `None` = auto.
    pub concurrency: Option<usize>,
    /// FFmpeg CRF quality (0–51, lower = better).
    pub crf: u32,
    /// FFmpeg preset: "ultrafast", "fast", "medium", etc.
    pub preset: String,
}

impl PipeConfig {
    pub fn new(
        width: u32,
        height: u32,
        fps: f64,
        duration_in_frames: u32,
        output: impl Into<PathBuf>,
    ) -> Self {
        Self {
            width,
            height,
            fps,
            duration_in_frames,
            output: output.into(),
            concurrency: None,
            crf: 18,
            preset: "fast".to_string(),
        }
    }

    pub fn with_concurrency(mut self, n: usize) -> Self {
        self.concurrency = Some(n);
        self
    }

    pub fn with_quality(mut self, crf: u32, preset: impl Into<String>) -> Self {
        self.crf = crf;
        self.preset = preset.into();
        self
    }
}

// ── Mode 1: Sequential PNG ───────────────────────────────────────────────────

/// Render all frames sequentially to PNG files.
///
/// Each frame is rendered in order and saved as `frame_000001.png`, etc.
/// Simple and debuggable, but slow for large frame counts.
pub fn render_all_frames<F>(
    backend: &dyn RasterizerBackend,
    config: &NativeRenderConfig,
    mut scene_fn: F,
) -> Result<Vec<PathBuf>, RasterError>
where
    F: FnMut(u32) -> Scene,
{
    std::fs::create_dir_all(&config.output_dir)?;

    let mut paths = Vec::with_capacity(config.duration_in_frames as usize);

    for frame in 0..config.duration_in_frames {
        let scene = scene_fn(frame);
        let frame_config = FrameConfig::new(config.width, config.height, frame, config.fps);
        let img = backend.render_frame(&scene, &frame_config)?;

        let path = config
            .output_dir
            .join(format!("frame_{:06}.png", frame + 1));
        img.save(&path)
            .map_err(|e| RasterError::ImageEncode(e.to_string()))?;
        paths.push(path);
    }

    Ok(paths)
}

// ── Mode 2: Parallel PNG ─────────────────────────────────────────────────────

/// Render all frames in parallel using Rayon, then write PNGs.
///
/// Frames are rendered concurrently across all available CPU cores.
/// Requires `backend` to implement `Send + Sync`.
///
/// # Performance
/// On a machine with N cores, this is roughly N× faster than sequential
/// for the rasterization step. PNG file I/O is still sequential to preserve order.
pub fn render_parallel<F, B>(
    backend: &B,
    config: &NativeRenderConfig,
    scene_fn: F,
) -> Result<Vec<PathBuf>, RasterError>
where
    B: RasterizerBackend + Send + Sync,
    F: Fn(u32) -> Scene + Send + Sync,
{
    std::fs::create_dir_all(&config.output_dir)?;

    let width = config.width;
    let height = config.height;
    let fps = config.fps;
    let total = config.duration_in_frames;
    let dir = &config.output_dir;

    // Configure Rayon thread pool if explicit concurrency was requested
    let pool = match config.concurrency {
        Some(n) => rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .map_err(|e| RasterError::Init(format!("Failed to build thread pool: {e}")))?,
        None => rayon::ThreadPoolBuilder::new()
            .build()
            .map_err(|e| RasterError::Init(format!("Failed to build thread pool: {e}")))?,
    };

    // Render all frames in parallel → collect (frame_index, img) pairs
    let rendered: Result<Vec<(u32, RgbaImage)>, RasterError> = pool.install(|| {
        (0..total)
            .into_par_iter()
            .map(|frame| {
                let scene = scene_fn(frame);
                let frame_cfg = FrameConfig::new(width, height, frame, fps);
                let img = backend.render_frame(&scene, &frame_cfg)?;
                Ok((frame, img))
            })
            .collect()
    });

    let mut pairs = rendered?;
    // Sort by frame index (parallel iteration doesn't guarantee order)
    pairs.sort_by_key(|(f, _)| *f);

    // Write PNGs in order
    let mut paths = Vec::with_capacity(total as usize);
    for (frame, img) in pairs {
        let path = dir.join(format!("frame_{:06}.png", frame + 1));
        img.save(&path)
            .map_err(|e| RasterError::ImageEncode(e.to_string()))?;
        paths.push(path);
    }

    Ok(paths)
}

// ── Mode 3: FFmpeg stdin pipe (fastest) ──────────────────────────────────────

/// Render frames in parallel and stream raw RGBA directly to FFmpeg stdin.
///
/// **This is the fastest rendering mode.** It eliminates:
/// - PNG compression overhead
/// - Disk write latency for intermediate frames
/// - A second disk read by FFmpeg
///
/// # Pipeline
/// ```text
/// Rayon workers → sorted RGBA chunks → FFmpeg stdin → MP4
/// ```
///
/// # FFmpeg invocation
/// ```text
/// ffmpeg -f rawvideo -pix_fmt rgba -s WxH -r FPS -i pipe:0
///        -c:v libx264 -pix_fmt yuv420p -crf N -preset P
///        -movflags +faststart output.mp4
/// ```
pub fn render_to_ffmpeg_pipe<F, B>(
    backend: &B,
    config: &PipeConfig,
    scene_fn: F,
) -> Result<(), RasterError>
where
    B: RasterizerBackend + Send + Sync,
    F: Fn(u32) -> Scene + Send + Sync,
{
    let width = config.width;
    let height = config.height;
    let fps = config.fps;
    let total = config.duration_in_frames;

    // ── 1. Spawn FFmpeg ──────────────────────────────────────────────────────
    let ffmpeg_args = build_pipe_ffmpeg_args(config);
    let mut ffmpeg = Command::new("ffmpeg")
        .args(&ffmpeg_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            RasterError::Init(format!("Failed to spawn FFmpeg: {e}\nIs ffmpeg installed?"))
        })?;

    // ── 2. Render frames in parallel ─────────────────────────────────────────
    let concurrency = config.concurrency.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    });

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(concurrency)
        .build()
        .map_err(|e| RasterError::Init(format!("Rayon pool error: {e}")))?;

    // Render all frames → collect Vec<(frame, raw_rgba)>
    let rendered: Result<Vec<(u32, Vec<u8>)>, RasterError> = pool.install(|| {
        (0..total)
            .into_par_iter()
            .map(|frame| {
                let scene = scene_fn(frame);
                let frame_cfg = FrameConfig::new(width, height, frame, fps);
                let img = backend.render_frame(&scene, &frame_cfg)?;
                Ok((frame, img.into_raw()))
            })
            .collect()
    });

    let mut pairs = rendered?;
    // Sort to ensure frame order before piping
    pairs.sort_by_key(|(f, _)| *f);

    // ── 3. Stream sorted RGBA bytes to FFmpeg stdin ───────────────────────────
    {
        let mut stdin = ffmpeg
            .stdin
            .take()
            .ok_or_else(|| RasterError::Init("Failed to open FFmpeg stdin".into()))?;

        for (_, rgba) in &pairs {
            stdin
                .write_all(rgba)
                .map_err(|e| RasterError::ImageEncode(format!("FFmpeg pipe write error: {e}")))?;
        }
        stdin.flush().ok();
    } // `stdin` drops here -> EOF sent to FFmpeg pipe

    // ── 4. Wait for FFmpeg to finish ─────────────────────────────────────────
    let status = ffmpeg
        .wait()
        .map_err(|e| RasterError::ImageEncode(format!("FFmpeg wait error: {e}")))?;

    if !status.success() {
        return Err(RasterError::ImageEncode(format!(
            "FFmpeg exited with non-zero status: {:?}",
            status.code()
        )));
    }

    Ok(())
}

/// Build FFmpeg arguments for rawvideo stdin pipe → MP4.
pub fn build_pipe_ffmpeg_args(config: &PipeConfig) -> Vec<String> {
    let args = vec![
        "-y".into(), // overwrite output
        "-f".into(),
        "rawvideo".into(), // input format
        "-pix_fmt".into(),
        "rgba".into(), // pixel format
        "-s".into(),
        format!("{}x{}", config.width, config.height),
        "-r".into(),
        format!("{}", config.fps),
        "-i".into(),
        "pipe:0".into(), // read from stdin
        "-c:v".into(),
        "libx264".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-crf".into(),
        config.crf.to_string(),
        "-preset".into(),
        config.preset.clone(),
        "-movflags".into(),
        "+faststart".into(),
        config.output.to_string_lossy().to_string(),
    ];
    args
}

/// Save a single `RgbaImage` frame to disk.
pub fn save_frame(img: &RgbaImage, path: &Path) -> Result<(), RasterError> {
    img.save(path)
        .map_err(|e| RasterError::ImageEncode(e.to_string()))
}

// ── Benchmark helper ─────────────────────────────────────────────────────────

/// Render a single frame and return elapsed time (for benchmarking).
pub fn render_frame_timed(
    backend: &dyn RasterizerBackend,
    scene: &Scene,
    config: &FrameConfig,
) -> Result<(RgbaImage, std::time::Duration), RasterError> {
    let start = std::time::Instant::now();
    let img = backend.render_frame(scene, config)?;
    Ok((img, start.elapsed()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Color, Scene, SceneNode};
    use crate::tiny_skia_backend::TinySkiaBackend;

    fn solid_scene(color: Color) -> impl Fn(u32) -> Scene + Send + Sync {
        move |_frame| {
            let mut s = Scene::new();
            s.push(SceneNode::Rect {
                x: 0.0,
                y: 0.0,
                w: 64.0,
                h: 64.0,
                fill: color,
                stroke: None,
                stroke_width: 0.0,
                corner_radius: 0.0,
            });
            s
        }
    }

    #[test]
    fn test_sequential_renders_correct_count() {
        let backend = TinySkiaBackend::headless();
        let tmp = std::env::temp_dir().join("dioxuscut_test_seq");
        let _ = std::fs::remove_dir_all(&tmp);

        let config = NativeRenderConfig::new(64, 64, 30.0, 5, &tmp);
        let paths = render_all_frames(&backend, &config, |frame| {
            let mut s = Scene::new();
            s.push(SceneNode::Rect {
                x: 0.0,
                y: 0.0,
                w: 64.0,
                h: 64.0,
                fill: Color::rgb(frame as u8 * 40, 0, 0),
                stroke: None,
                stroke_width: 0.0,
                corner_radius: 0.0,
            });
            s
        })
        .expect("sequential render failed");

        assert_eq!(paths.len(), 5, "Should have 5 frame files");
        for p in &paths {
            assert!(p.exists(), "PNG file should exist: {p:?}");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_parallel_renders_same_count_as_sequential() {
        let backend = TinySkiaBackend::headless();
        let tmp = std::env::temp_dir().join("dioxuscut_test_par");
        let _ = std::fs::remove_dir_all(&tmp);

        let config = NativeRenderConfig::new(64, 64, 30.0, 10, &tmp).with_concurrency(4);

        let paths = render_parallel(&backend, &config, solid_scene(Color::rgb(0, 0, 255)))
            .expect("parallel render failed");

        assert_eq!(paths.len(), 10, "Should have 10 frame files");
        // Verify they are in order
        for (i, p) in paths.iter().enumerate() {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            assert_eq!(name, format!("frame_{:06}.png", i + 1));
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_parallel_pixel_values_correct() {
        let backend = TinySkiaBackend::headless();
        let tmp = std::env::temp_dir().join("dioxuscut_test_par_px");
        let _ = std::fs::remove_dir_all(&tmp);

        let config = NativeRenderConfig::new(64, 64, 30.0, 4, &tmp);
        let paths = render_parallel(&backend, &config, |frame| {
            let mut s = Scene::new();
            // Different colour per frame so we can verify correctness
            s.push(SceneNode::Rect {
                x: 0.0,
                y: 0.0,
                w: 64.0,
                h: 64.0,
                fill: Color::rgb(frame as u8 * 60, 0, 0),
                stroke: None,
                stroke_width: 0.0,
                corner_radius: 0.0,
            });
            s
        })
        .expect("parallel pixel render failed");

        for (i, path) in paths.iter().enumerate() {
            let img = image::open(path).expect("open frame").into_rgba8();
            let px = img.get_pixel(32, 32);
            let expected_r = (i as u8) * 60;
            assert_eq!(px[0], expected_r, "Frame {i}: red channel mismatch");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_pipe_config_ffmpeg_args() {
        let config = PipeConfig::new(1920, 1080, 30.0, 10, "/tmp/out.mp4");
        let args = build_pipe_ffmpeg_args(&config);
        assert!(
            args.contains(&"rawvideo".to_string()),
            "args should contain rawvideo"
        );
        assert!(
            args.contains(&"1920x1080".to_string()),
            "args should contain resolution"
        );
        assert!(
            args.contains(&"pipe:0".to_string()),
            "args should read from stdin"
        );
        assert!(
            args.contains(&"/tmp/out.mp4".to_string()),
            "args should contain output path"
        );
    }

    #[test]
    fn test_frame_timed() {
        let backend = TinySkiaBackend::headless();
        let mut scene = Scene::new();
        scene.push(SceneNode::Circle {
            cx: 32.0,
            cy: 32.0,
            r: 20.0,
            fill: Color::rgb(255, 128, 0),
            stroke: None,
            stroke_width: 0.0,
        });
        let config = FrameConfig::new(64, 64, 0, 30.0);

        let (img, elapsed) =
            render_frame_timed(&backend, &scene, &config).expect("timed render failed");
        assert_eq!(img.width(), 64);
        println!("Single 64x64 frame: {:?}", elapsed);
    }
}
