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
//! Pipe (fastest):   bounded Rayon batch → ordered RGBA frames → FFmpeg → MP4
//!                   Zero disk I/O, zero PNG compression overhead
//! ```

use crate::backend::{FrameConfig, RasterError, RasterizerBackend};
use crate::scene::{AudioTrack, Scene};
use crate::video_cache::canonical_local_path;
use image::RgbaImage;
use rayon::prelude::*;
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

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

/// Video codec used by the FFmpeg pipe encoder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VideoCodec {
    #[default]
    H264,
    H265,
    Vp9,
    Av1,
    ProRes,
    Gif,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StillImageFormat {
    #[default]
    Png,
    Jpeg,
    WebP,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderProgress {
    pub completed_frames: u32,
    pub total_frames: u32,
    pub frame: u32,
}

#[derive(Clone, Default)]
pub struct RenderCancellationToken(Arc<AtomicBool>);

impl RenderCancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Default)]
pub struct RenderControl {
    cancellation: RenderCancellationToken,
    timeout: Option<Duration>,
    progress: Option<Arc<dyn Fn(RenderProgress) + Send + Sync>>,
}

impl fmt::Debug for RenderControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenderControl")
            .field("cancelled", &self.cancellation.is_cancelled())
            .field("timeout", &self.timeout)
            .field("has_progress_callback", &self.progress.is_some())
            .finish()
    }
}

impl RenderControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancellation_token(&self) -> RenderCancellationToken {
        self.cancellation.clone()
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_progress(
        mut self,
        callback: impl Fn(RenderProgress) + Send + Sync + 'static,
    ) -> Self {
        self.progress = Some(Arc::new(callback));
        self
    }

    fn check(&self, started: Instant) -> Result<(), RasterError> {
        if self.cancellation.is_cancelled() {
            return Err(RasterError::Cancelled);
        }
        if self
            .timeout
            .is_some_and(|timeout| started.elapsed() >= timeout)
        {
            return Err(RasterError::Timeout);
        }
        Ok(())
    }
}

/// Piped encoding configuration (no intermediate PNG files).
#[derive(Debug, Clone)]
pub struct PipeConfig {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_in_frames: u32,
    /// First composition frame included in the output.
    pub start_frame: u32,
    /// Output media file path.
    pub output: PathBuf,
    /// Number of parallel render workers. `None` = auto.
    pub concurrency: Option<usize>,
    /// FFmpeg CRF quality (0–51, lower = better).
    pub crf: u32,
    /// FFmpeg preset: "ultrafast", "fast", "medium", etc.
    pub preset: String,
    pub codec: VideoCodec,
    /// Audio tracks mixed and trimmed to the rendered video duration.
    pub audio_tracks: Vec<AudioTrack>,
    pub control: RenderControl,
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
            start_frame: 0,
            output: output.into(),
            concurrency: None,
            crf: 18,
            preset: "fast".to_string(),
            codec: VideoCodec::H264,
            audio_tracks: Vec::new(),
            control: RenderControl::default(),
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

    pub fn with_audio_tracks(mut self, tracks: impl IntoIterator<Item = AudioTrack>) -> Self {
        self.audio_tracks = tracks.into_iter().collect();
        self
    }

    pub fn with_codec(mut self, codec: VideoCodec) -> Self {
        self.codec = codec;
        self
    }

    pub fn with_frame_start(mut self, start_frame: u32) -> Self {
        self.start_frame = start_frame;
        self
    }

    pub fn with_control(mut self, control: RenderControl) -> Self {
        self.control = control;
        self
    }
}

/// Render one composition frame directly to PNG, JPEG, or WebP.
#[allow(clippy::too_many_arguments)]
pub fn render_still_fallible<F, B, E>(
    backend: &B,
    width: u32,
    height: u32,
    fps: f64,
    frame: u32,
    output: &Path,
    format: StillImageFormat,
    control: &RenderControl,
    scene_fn: F,
) -> Result<(), RasterError>
where
    B: RasterizerBackend + Send + Sync,
    F: FnOnce(u32) -> Result<Scene, E>,
    E: std::fmt::Display,
{
    let started = Instant::now();
    control.check(started)?;
    let scene = scene_fn(frame).map_err(|error| RasterError::Frame {
        frame,
        reason: error.to_string(),
    })?;
    let image = backend.render_frame(&scene, &FrameConfig::new(width, height, frame, fps))?;
    control.check(started)?;
    match format {
        StillImageFormat::Png => image
            .save_with_format(output, image::ImageFormat::Png)
            .map_err(|error| RasterError::ImageEncode(error.to_string()))?,
        StillImageFormat::Jpeg => image::DynamicImage::ImageRgba8(image)
            .to_rgb8()
            .save_with_format(output, image::ImageFormat::Jpeg)
            .map_err(|error| RasterError::ImageEncode(error.to_string()))?,
        StillImageFormat::WebP => image
            .save_with_format(output, image::ImageFormat::WebP)
            .map_err(|error| RasterError::ImageEncode(error.to_string()))?,
    }
    if let Some(callback) = &control.progress {
        callback(RenderProgress {
            completed_frames: 1,
            total_frames: 1,
            frame,
        });
    }
    Ok(())
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

/// Render frames in bounded parallel batches and stream raw RGBA to FFmpeg stdin.
///
/// **This is the fastest rendering mode.** It eliminates:
/// - PNG compression overhead
/// - Disk write latency for intermediate frames
/// - A second disk read by FFmpeg
///
/// # Pipeline
/// ```text
/// bounded Rayon batch → ordered RGBA frames → FFmpeg stdin → MP4
/// ```
///
/// # FFmpeg invocation
/// ```text
/// ffmpeg -f rawvideo -pix_fmt rgba -s WxH -r FPS -i pipe:0
///        [-i audio ... -filter_complex mix] -c:v libx264 -c:a aac
///        -pix_fmt yuv420p -crf N -preset P
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
    render_to_ffmpeg_pipe_fallible(backend, config, |frame| {
        Ok::<Scene, std::convert::Infallible>(scene_fn(frame))
    })
}

/// Fallible variant of [`render_to_ffmpeg_pipe`].
///
/// Scene generation errors are annotated with the frame number and propagated
/// before rasterization. This is intended for script-backed compositions and
/// other dynamic scene sources that can fail while evaluating a frame.
pub fn render_to_ffmpeg_pipe_fallible<F, B, E>(
    backend: &B,
    config: &PipeConfig,
    scene_fn: F,
) -> Result<(), RasterError>
where
    B: RasterizerBackend + Send + Sync,
    F: Fn(u32) -> Result<Scene, E> + Send + Sync,
    E: std::fmt::Display + Send,
{
    let width = config.width;
    let height = config.height;
    let fps = config.fps;
    let total = config.duration_in_frames;
    let started = Instant::now();

    validate_pipe_config(config)?;
    config.control.check(started)?;

    // ── 1. Spawn FFmpeg ──────────────────────────────────────────────────────
    let ffmpeg_args = build_pipe_ffmpeg_args(config);
    let mut ffmpeg = Command::new("ffmpeg")
        .args(&ffmpeg_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
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
    if concurrency == 0 {
        let _ = ffmpeg.kill();
        let _ = ffmpeg.wait();
        return Err(RasterError::Init(
            "Render concurrency must be greater than zero".into(),
        ));
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(concurrency)
        .build()
        .map_err(|e| {
            let _ = ffmpeg.kill();
            let _ = ffmpeg.wait();
            RasterError::Init(format!("Rayon pool error: {e}"))
        })?;

    // ── 3. Render and stream bounded batches in frame order ───────────────────
    // At most `concurrency` raw frames are retained at once. This keeps memory
    // proportional to the worker count instead of the video duration.
    let mut stdin = ffmpeg
        .stdin
        .take()
        .ok_or_else(|| RasterError::Init("Failed to open FFmpeg stdin".into()))?;

    let render_result = (0..total).step_by(concurrency).try_for_each(|batch_start| {
        config.control.check(started)?;
        let batch_end = total.min(batch_start.saturating_add(concurrency as u32));
        let rendered: Result<Vec<(u32, u32, Vec<u8>)>, RasterError> = pool.install(|| {
            (batch_start..batch_end)
                .into_par_iter()
                .map(|frame| {
                    config.control.check(started)?;
                    let composition_frame =
                        config.start_frame.checked_add(frame).ok_or_else(|| {
                            RasterError::Scene("render frame range overflows u32".into())
                        })?;
                    let scene =
                        scene_fn(composition_frame).map_err(|error| RasterError::Frame {
                            frame: composition_frame,
                            reason: error.to_string(),
                        })?;
                    let frame_cfg = FrameConfig::new(width, height, composition_frame, fps);
                    let img = backend.render_frame(&scene, &frame_cfg)?;
                    config.control.check(started)?;
                    Ok((frame, composition_frame, img.into_raw()))
                })
                .collect()
        });

        let mut frames = rendered?;
        frames.sort_by_key(|(frame, _, _)| *frame);
        for (frame, composition_frame, rgba) in frames {
            config.control.check(started)?;
            stdin
                .write_all(&rgba)
                .map_err(|e| RasterError::ImageEncode(format!("FFmpeg pipe write error: {e}")))?;
            if let Some(callback) = &config.control.progress {
                callback(RenderProgress {
                    completed_frames: frame + 1,
                    total_frames: total,
                    frame: composition_frame,
                });
            }
        }
        Ok::<(), RasterError>(())
    });

    if render_result.is_ok() {
        let _ = stdin.flush();
    }
    drop(stdin); // EOF for FFmpeg

    if let Err(error) = render_result {
        let _ = ffmpeg.kill();
        let _ = ffmpeg.wait();
        return Err(error);
    }

    // ── 4. Wait for FFmpeg to finish ─────────────────────────────────────────
    let output = wait_for_ffmpeg(ffmpeg, &config.control, started)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RasterError::ImageEncode(format!(
            "FFmpeg exited with non-zero status {:?}: {}",
            output.status.code(),
            stderr.trim()
        )));
    }

    Ok(())
}

fn validate_pipe_config(config: &PipeConfig) -> Result<(), RasterError> {
    if config.width == 0 || config.height == 0 {
        return Err(RasterError::Init(
            "render width and height must be positive".into(),
        ));
    }
    if !config.fps.is_finite() || config.fps <= 0.0 {
        return Err(RasterError::Init(
            "render FPS must be finite and positive".into(),
        ));
    }
    if config.duration_in_frames == 0 {
        return Err(RasterError::Init(
            "render duration must contain at least one frame".into(),
        ));
    }
    config
        .start_frame
        .checked_add(config.duration_in_frames - 1)
        .ok_or_else(|| RasterError::Init("render frame range overflows u32".into()))?;
    if config.codec != VideoCodec::Gif
        && (!config.width.is_multiple_of(2) || !config.height.is_multiple_of(2))
    {
        return Err(RasterError::Init(
            "video render width and height must be even".into(),
        ));
    }
    let max_crf = match config.codec {
        VideoCodec::H264 | VideoCodec::H265 => Some(51),
        VideoCodec::Vp9 | VideoCodec::Av1 => Some(63),
        VideoCodec::ProRes | VideoCodec::Gif => None,
    };
    if max_crf.is_some_and(|maximum| config.crf > maximum) {
        return Err(RasterError::Init(format!(
            "CRF {} exceeds the {:?} maximum of {}",
            config.crf,
            config.codec,
            max_crf.expect("checked above")
        )));
    }
    if config.codec == VideoCodec::Gif && !config.audio_tracks.is_empty() {
        return Err(RasterError::Scene(
            "GIF output does not support audio tracks".into(),
        ));
    }
    validate_audio_tracks(&config.audio_tracks)
}

fn wait_for_ffmpeg(
    mut child: std::process::Child,
    control: &RenderControl,
    started: Instant,
) -> Result<std::process::Output, RasterError> {
    loop {
        if let Err(error) = control.check(started) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stderr = Vec::new();
                if let Some(mut pipe) = child.stderr.take() {
                    pipe.read_to_end(&mut stderr).map_err(|error| {
                        RasterError::ImageEncode(format!("FFmpeg stderr read error: {error}"))
                    })?;
                }
                return Ok(std::process::Output {
                    status,
                    stdout: Vec::new(),
                    stderr,
                });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RasterError::ImageEncode(format!(
                    "FFmpeg wait error: {error}"
                )));
            }
        }
    }
}

/// Build FFmpeg arguments for rawvideo stdin pipe and the selected codec.
pub fn build_pipe_ffmpeg_args(config: &PipeConfig) -> Vec<String> {
    let mut args = vec![
        "-y".into(), // overwrite output
        "-loglevel".into(),
        "error".into(),
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
    ];

    let audio_tracks = active_audio_tracks(config);
    for track in &audio_tracks {
        if track.looped {
            args.extend(["-stream_loop".into(), "-1".into()]);
        }
        args.extend(["-i".into(), track.src.clone()]);
    }

    if config.codec == VideoCodec::Gif {
        args.extend([
            "-filter_complex".into(),
            "[0:v]split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse[gif]".into(),
            "-map".into(),
            "[gif]".into(),
            "-an".into(),
        ]);
    } else {
        args.extend(["-map".into(), "0:v:0".into()]);
        if audio_tracks.is_empty() {
            args.push("-an".into());
        } else {
            let audio_codec = match config.codec {
                VideoCodec::Vp9 | VideoCodec::Av1 => "libopus",
                VideoCodec::ProRes => "pcm_s16le",
                VideoCodec::H264 | VideoCodec::H265 => "aac",
                VideoCodec::Gif => unreachable!(),
            };
            args.extend([
                "-filter_complex".into(),
                build_audio_filter(config, &audio_tracks),
                "-map".into(),
                "[aout]".into(),
                "-c:a".into(),
                audio_codec.into(),
            ]);
            if audio_codec != "pcm_s16le" {
                args.extend(["-b:a".into(), "192k".into()]);
            }
        }
    }

    match config.codec {
        VideoCodec::H264 => args.extend([
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
        ]),
        VideoCodec::H265 => args.extend([
            "-c:v".into(),
            "libx265".into(),
            "-tag:v".into(),
            "hvc1".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-crf".into(),
            config.crf.to_string(),
            "-preset".into(),
            config.preset.clone(),
            "-movflags".into(),
            "+faststart".into(),
        ]),
        VideoCodec::Vp9 => args.extend([
            "-c:v".into(),
            "libvpx-vp9".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-crf".into(),
            config.crf.min(63).to_string(),
            "-b:v".into(),
            "0".into(),
        ]),
        VideoCodec::Av1 => {
            if ffmpeg_has_encoder("libsvtav1") {
                args.extend([
                    "-c:v".into(),
                    "libsvtav1".into(),
                    "-pix_fmt".into(),
                    "yuv420p".into(),
                    "-crf".into(),
                    config.crf.min(63).to_string(),
                    "-preset".into(),
                    "8".into(),
                ]);
            } else {
                args.extend([
                    "-c:v".into(),
                    "libaom-av1".into(),
                    "-pix_fmt".into(),
                    "yuv420p".into(),
                    "-crf".into(),
                    config.crf.min(63).to_string(),
                    "-b:v".into(),
                    "0".into(),
                    "-cpu-used".into(),
                    "6".into(),
                ]);
            }
        }
        VideoCodec::ProRes => args.extend([
            "-c:v".into(),
            "prores_ks".into(),
            "-profile:v".into(),
            "3".into(),
            "-pix_fmt".into(),
            "yuv422p10le".into(),
        ]),
        VideoCodec::Gif => {}
    }
    args.extend([
        "-t".into(),
        format!("{:.9}", config.duration_in_frames as f64 / config.fps),
        config.output.to_string_lossy().to_string(),
    ]);
    args
}

fn ffmpeg_has_encoder(name: &str) -> bool {
    static ENCODERS: OnceLock<String> = OnceLock::new();
    ENCODERS
        .get_or_init(|| {
            Command::new("ffmpeg")
                .args(["-hide_banner", "-encoders"])
                .output()
                .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                .unwrap_or_default()
        })
        .split_whitespace()
        .any(|encoder| encoder == name)
}

fn validate_audio_tracks(tracks: &[AudioTrack]) -> Result<(), RasterError> {
    for track in tracks {
        canonical_local_path(&track.src)?;
        if !track.start_from.is_finite() || track.start_from < 0.0 {
            return Err(invalid_audio(
                track,
                "start_from must be finite and non-negative",
            ));
        }
        if !track.timeline_start.is_finite() || track.timeline_start < 0.0 {
            return Err(invalid_audio(
                track,
                "timeline_start must be finite and non-negative",
            ));
        }
        if track
            .duration
            .is_some_and(|duration| !duration.is_finite() || duration <= 0.0)
        {
            return Err(invalid_audio(track, "duration must be finite and positive"));
        }
        if !track.volume.is_finite() || !(0.0..=1.0).contains(&track.volume) {
            return Err(invalid_audio(track, "volume must be between 0.0 and 1.0"));
        }
        if !track.playback_rate.is_finite() || !(0.5..=2.0).contains(&track.playback_rate) {
            return Err(invalid_audio(
                track,
                "playback_rate must be between 0.5 and 2.0",
            ));
        }
    }
    Ok(())
}

fn invalid_audio(track: &AudioTrack, reason: &str) -> RasterError {
    RasterError::MediaAsset {
        path: track.src.clone(),
        reason: reason.into(),
    }
}

fn active_audio_tracks(config: &PipeConfig) -> Vec<&AudioTrack> {
    let range_start = config.start_frame as f64 / config.fps;
    let range_end = range_start + config.duration_in_frames as f64 / config.fps;
    config
        .audio_tracks
        .iter()
        .filter(|track| {
            let track_end = track
                .duration
                .map(|duration| track.timeline_start + duration)
                .unwrap_or(f64::INFINITY);
            track.timeline_start < range_end && track_end > range_start
        })
        .collect()
}

fn build_audio_filter(config: &PipeConfig, tracks: &[&AudioTrack]) -> String {
    let mut filters = Vec::with_capacity(tracks.len() + 1);
    let range_start = config.start_frame as f64 / config.fps;
    for (index, track) in tracks.iter().enumerate() {
        let skipped_timeline = (range_start - track.timeline_start).max(0.0);
        let source_start = track.start_from + skipped_timeline * track.playback_rate;
        let mut chain = format!(
            "[{}:a:0]atrim=start={:.9},asetpts=PTS-STARTPTS,atempo={:.9},volume={:.9}",
            index + 1,
            source_start,
            track.playback_rate,
            track.volume
        );
        if let Some(duration) = track.duration {
            let remaining = (duration - skipped_timeline).max(0.0);
            chain.push_str(&format!(",atrim=duration={remaining:.9}"));
        }
        let relative_start = (track.timeline_start - range_start).max(0.0);
        if relative_start > 0.0 {
            let delay_ms = (relative_start * 1000.0).round() as u64;
            chain.push_str(&format!(",adelay={delay_ms}:all=1"));
        }
        chain.push_str(&format!("[a{index}]"));
        filters.push(chain);
    }

    let labels = (0..tracks.len())
        .map(|index| format!("[a{index}]"))
        .collect::<String>();
    let output_duration = config.duration_in_frames as f64 / config.fps;
    if tracks.len() == 1 {
        filters.push(format!(
            "{labels}apad,atrim=duration={output_duration:.9},asetpts=N/SR/TB[aout]"
        ));
    } else {
        filters.push(format!(
            "{labels}amix=inputs={}:normalize=0:duration=longest,apad,atrim=duration={output_duration:.9},asetpts=N/SR/TB[aout]",
            tracks.len()
        ));
    }
    filters.join(";")
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
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "dioxuscut_{label}_{}_{}",
            std::process::id(),
            nonce
        ))
    }

    fn ffmpeg_available() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

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
        assert!(args.contains(&"-an".to_string()));
    }

    #[test]
    fn test_codec_specific_ffmpeg_args() {
        let cases = [
            (VideoCodec::H264, "out.mp4", "libx264"),
            (VideoCodec::H265, "out.mp4", "libx265"),
            (VideoCodec::Vp9, "out.webm", "libvpx-vp9"),
            (VideoCodec::ProRes, "out.mov", "prores_ks"),
        ];
        for (codec, output, encoder) in cases {
            let config = PipeConfig::new(64, 64, 30.0, 2, output).with_codec(codec);
            let args = build_pipe_ffmpeg_args(&config);
            assert!(args.contains(&encoder.to_string()), "missing {encoder}");
            assert_eq!(args.last(), Some(&output.to_string()));
        }

        let av1 = build_pipe_ffmpeg_args(
            &PipeConfig::new(64, 64, 30.0, 2, "out.webm").with_codec(VideoCodec::Av1),
        );
        assert!(av1
            .iter()
            .any(|arg| arg == "libsvtav1" || arg == "libaom-av1"));

        let gif = build_pipe_ffmpeg_args(
            &PipeConfig::new(64, 64, 30.0, 2, "out.gif").with_codec(VideoCodec::Gif),
        );
        assert!(gif.iter().any(|arg| arg.contains("palettegen")));
        assert!(gif.contains(&"-an".to_string()));
    }

    #[test]
    fn test_still_formats_write_decodable_images_and_report_progress() {
        let backend = TinySkiaBackend::headless();
        let temp = unique_temp_dir("stills");
        std::fs::create_dir_all(&temp).unwrap();

        for (format, extension) in [
            (StillImageFormat::Png, "png"),
            (StillImageFormat::Jpeg, "jpg"),
            (StillImageFormat::WebP, "webp"),
        ] {
            let output = temp.join(format!("frame.{extension}"));
            let progress = Arc::new(Mutex::new(Vec::new()));
            let captured = Arc::clone(&progress);
            let control = RenderControl::new().with_progress(move |event| {
                captured.lock().unwrap().push(event);
            });
            render_still_fallible(
                &backend,
                32,
                24,
                30.0,
                17,
                &output,
                format,
                &control,
                |frame| {
                    assert_eq!(frame, 17);
                    Ok::<_, std::convert::Infallible>(solid_scene(Color::rgb(12, 34, 56))(frame))
                },
            )
            .unwrap();

            let decoded = image::open(&output).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (32, 24));
            assert_eq!(
                *progress.lock().unwrap(),
                vec![RenderProgress {
                    completed_frames: 1,
                    total_frames: 1,
                    frame: 17,
                }]
            );
        }
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn test_frame_range_streams_absolute_frames_in_order() {
        if !ffmpeg_available() {
            eprintln!("Skipping frame-range pipe test: FFmpeg is unavailable");
            return;
        }
        let temp = unique_temp_dir("frame_range");
        std::fs::create_dir_all(&temp).unwrap();
        let output = temp.join("range.mp4");
        let rendered_frames = Arc::new(Mutex::new(Vec::new()));
        let rendered_capture = Arc::clone(&rendered_frames);
        let progress = Arc::new(Mutex::new(Vec::new()));
        let progress_capture = Arc::clone(&progress);
        let control = RenderControl::new().with_progress(move |event| {
            progress_capture.lock().unwrap().push(event);
        });
        let config = PipeConfig::new(32, 24, 30.0, 3, &output)
            .with_frame_start(10)
            .with_concurrency(2)
            .with_control(control);

        render_to_ffmpeg_pipe(&TinySkiaBackend::headless(), &config, move |frame| {
            rendered_capture.lock().unwrap().push(frame);
            solid_scene(Color::rgb(frame as u8, 20, 30))(frame)
        })
        .unwrap();

        let mut actual_frames = rendered_frames.lock().unwrap().clone();
        actual_frames.sort_unstable();
        assert_eq!(actual_frames, vec![10, 11, 12]);
        assert_eq!(
            progress.lock().unwrap().as_slice(),
            &[
                RenderProgress {
                    completed_frames: 1,
                    total_frames: 3,
                    frame: 10,
                },
                RenderProgress {
                    completed_frames: 2,
                    total_frames: 3,
                    frame: 11,
                },
                RenderProgress {
                    completed_frames: 3,
                    total_frames: 3,
                    frame: 12,
                },
            ]
        );
        assert!(std::fs::metadata(&output).unwrap().len() > 0);
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn test_gif_pipe_writes_a_real_animation() {
        if !ffmpeg_available() {
            eprintln!("Skipping GIF pipe test: FFmpeg is unavailable");
            return;
        }
        let temp = unique_temp_dir("gif");
        std::fs::create_dir_all(&temp).unwrap();
        let output = temp.join("animation.gif");
        let config = PipeConfig::new(32, 24, 10.0, 2, &output).with_codec(VideoCodec::Gif);
        render_to_ffmpeg_pipe(&TinySkiaBackend::headless(), &config, |frame| {
            solid_scene(Color::rgb((frame * 100) as u8, 20, 30))(frame)
        })
        .unwrap();
        let bytes = std::fs::read(&output).unwrap();
        assert!(bytes.starts_with(b"GIF8"));
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn test_available_video_codecs_write_real_containers() {
        if !ffmpeg_available() {
            eprintln!("Skipping codec pipe test: FFmpeg is unavailable");
            return;
        }
        let temp = unique_temp_dir("codecs");
        std::fs::create_dir_all(&temp).unwrap();
        let av1_encoder = if ffmpeg_has_encoder("libsvtav1") {
            "libsvtav1"
        } else {
            "libaom-av1"
        };
        let cases = [
            (VideoCodec::H264, "h264.mp4", "libx264"),
            (VideoCodec::H265, "h265.mp4", "libx265"),
            (VideoCodec::Vp9, "vp9.webm", "libvpx-vp9"),
            (VideoCodec::Av1, "av1.webm", av1_encoder),
            (VideoCodec::ProRes, "prores.mov", "prores_ks"),
        ];

        for (codec, file_name, encoder) in cases {
            if !ffmpeg_has_encoder(encoder) {
                eprintln!("Skipping {codec:?} pipe test: {encoder} is unavailable");
                continue;
            }
            let output = temp.join(file_name);
            let config = PipeConfig::new(64, 64, 1.0, 1, &output)
                .with_codec(codec)
                .with_concurrency(1)
                .with_quality(35, "fast");
            render_to_ffmpeg_pipe(
                &TinySkiaBackend::headless(),
                &config,
                solid_scene(Color::rgb(40, 80, 120)),
            )
            .unwrap_or_else(|error| panic!("{codec:?} encode failed: {error}"));

            let bytes = std::fs::read(&output).unwrap();
            assert!(!bytes.is_empty(), "{codec:?} output is empty");
            match codec {
                VideoCodec::H264 | VideoCodec::H265 | VideoCodec::ProRes => {
                    assert_eq!(&bytes[4..8], b"ftyp", "{codec:?} is not ISO BMFF")
                }
                VideoCodec::Vp9 | VideoCodec::Av1 => {
                    assert!(bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]))
                }
                VideoCodec::Gif => unreachable!(),
            }
        }
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn test_cancelled_and_timed_out_render_stop_before_ffmpeg() {
        let temp = unique_temp_dir("control");
        std::fs::create_dir_all(&temp).unwrap();
        let cancelled_output = temp.join("cancelled.mp4");
        let cancelled_control = RenderControl::new();
        cancelled_control.cancellation_token().cancel();
        let cancelled =
            PipeConfig::new(32, 24, 30.0, 2, &cancelled_output).with_control(cancelled_control);
        let error = render_to_ffmpeg_pipe(
            &TinySkiaBackend::headless(),
            &cancelled,
            solid_scene(Color::rgb(1, 2, 3)),
        )
        .unwrap_err();
        assert!(matches!(error, RasterError::Cancelled));
        assert!(!cancelled_output.exists());

        let timeout_output = temp.join("timeout.mp4");
        let timed_out = PipeConfig::new(32, 24, 30.0, 2, &timeout_output)
            .with_control(RenderControl::new().with_timeout(Duration::ZERO));
        let error = render_to_ffmpeg_pipe(
            &TinySkiaBackend::headless(),
            &timed_out,
            solid_scene(Color::rgb(1, 2, 3)),
        )
        .unwrap_err();
        assert!(matches!(error, RasterError::Timeout));
        assert!(!timeout_output.exists());
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn test_audio_filter_args_mix_tracks() {
        let mut first = AudioTrack::new("first.wav");
        first.start_from = 0.25;
        first.timeline_start = 0.5;
        first.volume = 0.75;
        let mut second = AudioTrack::new("second.wav");
        second.looped = true;
        second.playback_rate = 1.25;
        let config = PipeConfig::new(1920, 1080, 30.0, 90, "/tmp/out.mp4")
            .with_audio_tracks([first, second]);

        let args = build_pipe_ffmpeg_args(&config);
        let filter_index = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .unwrap();
        let filter = &args[filter_index + 1];
        assert!(filter.contains("adelay=500:all=1"));
        assert!(filter.contains("atempo=1.250000000"));
        assert!(filter.contains("amix=inputs=2"));
        assert!(args.contains(&"-stream_loop".to_string()));
        assert!(args.contains(&"[aout]".to_string()));
    }

    #[test]
    fn test_audio_filter_trims_against_selected_frame_range() {
        let mut track = AudioTrack::new("track.wav");
        track.start_from = 0.25;
        track.duration = Some(5.0);
        let config = PipeConfig::new(64, 64, 30.0, 30, "out.mp4")
            .with_frame_start(60)
            .with_audio_tracks([track]);
        let args = build_pipe_ffmpeg_args(&config);
        let filter_index = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .unwrap();
        let filter = &args[filter_index + 1];
        assert!(filter.contains("atrim=start=2.250000000"));
        assert!(filter.contains("atrim=duration=3.000000000"));
        assert!(filter.contains("atrim=duration=1.000000000"));
        assert!(!filter.contains("adelay="));
    }

    #[test]
    fn test_audio_track_validation_rejects_invalid_volume() {
        let mut track = AudioTrack::new(std::env::current_exe().unwrap().display().to_string());
        track.volume = 1.5;
        let error = validate_audio_tracks(&[track]).unwrap_err();
        assert!(error.to_string().contains("volume must be between"));
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
