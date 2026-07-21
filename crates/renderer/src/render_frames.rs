//! Frame rendering — drives a headless composition through all frames.
//!
//! In a full implementation this would use a headless browser or
//! a native Skia backend. Currently provides the scaffolding and types.

use thiserror::Error;

/// Error type for the renderer.
#[derive(Debug, Error)]
pub enum RenderError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Encode error: {0}")]
    Encode(String),
    #[error("Frame {0} failed: {1}")]
    FrameFailed(u32, String),
}

/// Configuration for a render job.
#[derive(Debug, Clone)]
pub struct RenderConfig {
    /// URL of the Dioxus app to capture.
    pub url: String,
    /// Output directory for individual frame PNGs.
    pub output_dir: std::path::PathBuf,
    /// Width of the video.
    pub width: u32,
    /// Height of the video.
    pub height: u32,
    /// Frames per second.
    pub fps: f64,
    /// Total duration in frames.
    pub duration_in_frames: u32,
    /// Frame range to render (`None` = all frames).
    pub frame_range: Option<std::ops::RangeInclusive<u32>>,
    /// Concurrency — how many frames to render in parallel.
    pub concurrency: usize,
}

impl RenderConfig {
    /// Create a new render config with sensible defaults.
    pub fn new(url: String, output_dir: impl Into<std::path::PathBuf>, width: u32, height: u32, fps: f64, duration_in_frames: u32) -> Self {
        Self {
            url,
            output_dir: output_dir.into(),
            width,
            height,
            fps,
            duration_in_frames,
            frame_range: None,
            concurrency: num_cpus(),
        }
    }

    /// Returns the frame range, defaulting to all frames.
    pub fn effective_range(&self) -> std::ops::RangeInclusive<u32> {
        self.frame_range
            .clone()
            .unwrap_or(0..=self.duration_in_frames.saturating_sub(1))
    }
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// Render all frames of a composition to PNG files.
///
/// Currently a placeholder — full implementation requires a headless
/// rendering backend (e.g., headless Chromium or native Skia).
///
/// Each frame is written to `{output_dir}/frame-{:06}.png`.
pub async fn render_frames(config: &RenderConfig) -> Result<Vec<std::path::PathBuf>, RenderError> {
    use std::fs;
    use headless_chrome::{Browser, LaunchOptions};

    fs::create_dir_all(&config.output_dir)?;

    let range = config.effective_range();
    let mut paths = Vec::new();

    tracing::info!(
        "Rendering frames {}..={} at {}x{} @ {} fps",
        range.start(),
        range.end(),
        config.width,
        config.height,
        config.fps,
    );

    let browser = Browser::new(
        LaunchOptions::default_builder()
            .window_size(Some((config.width, config.height)))
            .build()
            .map_err(|e| RenderError::Encode(e.to_string()))?
    ).map_err(|e| RenderError::Encode(e.to_string()))?;

    let tab = browser.new_tab().map_err(|e| RenderError::Encode(e.to_string()))?;

    tracing::info!("Navigating to {}", config.url);
    tab.navigate_to(&config.url).map_err(|e| RenderError::Encode(e.to_string()))?;
    tab.wait_until_navigated().map_err(|e| RenderError::Encode(e.to_string()))?;

    // Optionally wait for the Dioxus web app to initialize
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    for frame in range {
        let path = config.output_dir.join(format!("frame-{frame:06}.png"));

        let js = format!("window.DIOXUSCUT_FRAME = {};", frame);
        tab.evaluate(&js, false).map_err(|e| RenderError::Encode(e.to_string()))?;

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        let png_data = tab.capture_screenshot(
            headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Png,
            None,
            None,
            true
        ).map_err(|e| RenderError::Encode(e.to_string()))?;
        
        fs::write(&path, png_data)?;
        tracing::debug!("Rendered frame {frame} → {}", path.display());
        paths.push(path);
    }

    Ok(paths)
}
