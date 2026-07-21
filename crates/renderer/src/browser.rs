//! Headless Chrome frame extractor module.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use anyhow::{Context, Result};
use headless_chrome::{Browser, LaunchOptions};

use crate::render_frames::RenderConfig;

/// Captures frames from a web app running at `url` using Headless Chrome.
///
/// Screenshots are saved to `output_dir` named `frame_%06d.png` (`frame_000000.png`, `frame_000001.png`, etc.).
pub async fn capture_frames(
    url: &str,
    output_dir: &Path,
    config: &RenderConfig,
) -> Result<Vec<PathBuf>> {
    let url = url.to_string();
    let output_dir = output_dir.to_path_buf();
    let config = config.clone();

    tokio::task::spawn_blocking(move || {
        capture_frames_sync(&url, &output_dir, &config)
    })
    .await
    .map_err(|e| anyhow::anyhow!("JoinError in spawn_blocking: {e}"))?
}

fn capture_frames_sync(
    url: &str,
    output_dir: &Path,
    config: &RenderConfig,
) -> Result<Vec<PathBuf>> {
    fs::create_dir_all(output_dir)
        .with_context(|| format!("Failed to create output directory {}", output_dir.display()))?;

    tracing::info!("Launching Headless Chrome browser...");
    let chrome_args: Vec<&std::ffi::OsStr> = vec![
        std::ffi::OsStr::new("--no-sandbox"),
        std::ffi::OsStr::new("--disable-gpu"),
        std::ffi::OsStr::new("--disable-dev-shm-usage"),
        std::ffi::OsStr::new("--disable-setuid-sandbox"),
    ];

    let options = LaunchOptions::default_builder()
        .headless(true)
        .sandbox(false)
        .args(chrome_args)
        .idle_browser_timeout(Duration::from_secs(120))
        .window_size(Some((config.width, config.height)))
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build browser launch options: {e}"))?;

    let browser = Browser::new(options)
        .map_err(|e| anyhow::anyhow!("Failed to launch browser: {e}"))?;

    let tab = browser
        .new_tab()
        .map_err(|e| anyhow::anyhow!("Failed to open new tab: {e}"))?;

    tracing::info!("Navigating to web app URL: {}", url);
    tab.navigate_to(url)
        .map_err(|e| anyhow::anyhow!("Failed to navigate to {url}: {e}"))?;

    let _ = tab.wait_for_element("body");
    std::thread::sleep(Duration::from_millis(500));

    let frames_range = config.effective_range();
    let total_frames = config.duration_in_frames;
    let mut captured_paths = Vec::new();

    let viewport_clip = headless_chrome::protocol::cdp::Page::Viewport {
        x: 0.0,
        y: 0.0,
        width: config.width as f64,
        height: config.height as f64,
        scale: 1.0,
    };

    tracing::info!(
        "Starting frame capture for {} frames (range: {:?})",
        total_frames,
        frames_range
    );

    for frame_idx in frames_range {
        let js = format!(
            "window.DIOXUSCUT_FRAME = {frame_idx}; if (window.__DIOXUSCUT_SET_FRAME) window.__DIOXUSCUT_SET_FRAME({frame_idx});"
        );

        tab.evaluate(&js, false)
            .map_err(|e| anyhow::anyhow!("JS evaluation failed at frame {frame_idx}: {e}"))?;

        std::thread::sleep(Duration::from_millis(30));

        let png_data = tab
            .capture_screenshot(
                headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Png,
                None,
                Some(viewport_clip.clone()),
                true,
            )
            .map_err(|e| anyhow::anyhow!("Failed to capture screenshot for frame {frame_idx}: {e}"))?;

        let file_name = format!("frame_{frame_idx:06}.png");
        let frame_path = output_dir.join(&file_name);
        fs::write(&frame_path, png_data)
            .with_context(|| format!("Failed to write frame file {}", frame_path.display()))?;

        tracing::info!("Captured frame {}/{}", frame_idx, total_frames);
        captured_paths.push(frame_path);
    }

    tracing::info!(
        "Completed frame capture: {} frames saved to {}",
        captured_paths.len(),
        output_dir.display()
    );

    Ok(captured_paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::spawn_server;

    #[tokio::test]
    async fn test_capture_frames_headless_chrome() {
        let temp_dir = std::env::temp_dir().join(format!(
            "dioxuscut_test_capture_frames_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        let index_file = temp_dir.join("index.html");
        fs::write(
            &index_file,
            r#"<!DOCTYPE html>
<html>
<head><title>Test</title></head>
<body>
<div id="app">Frame content</div>
<script>
  window.DIOXUSCUT_FRAME = 0;
  window.__DIOXUSCUT_SET_FRAME = function(f) {
    document.getElementById('app').innerText = 'Frame ' + f;
  };
</script>
</body>
</html>"#,
        )
        .unwrap();

        let server_handle = spawn_server(0, &temp_dir).await.unwrap();
        let frames_out = temp_dir.join("output_frames");

        let config = RenderConfig::new(
            server_handle.url().to_string(),
            &frames_out,
            640,
            480,
            30.0,
            2,
        );

        let paths = capture_frames(server_handle.url(), &frames_out, &config)
            .await
            .expect("capture_frames failed");

        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], frames_out.join("frame_000000.png"));
        assert_eq!(paths[1], frames_out.join("frame_000001.png"));

        for p in &paths {
            assert!(p.exists());
            let content = fs::read(p).unwrap();
            assert!(content.len() > 8);
            assert_eq!(&content[0..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        }

        server_handle.stop().await.unwrap();
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
