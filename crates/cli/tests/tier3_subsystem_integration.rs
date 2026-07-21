//! Tier 3: Subsystem Integration E2E Tests
//!
//! Validates contracts between HTTP server manager, Headless Chrome CDP renderer,
//! and FFmpeg MP4 encoding compiler.

use dioxuscut_renderer::{encode_frames, render_frames, spawn_server, EncodeConfig, RenderConfig};
use std::fs;

#[tokio::test]
async fn test_subsystem_http_server_lifecycle() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dioxuscut_tier3_server_{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let index_file = temp_dir.join("index.html");
    fs::write(&index_file, "<html><body><h1>Dioxuscut Subsystem Test</h1></body></html>").unwrap();

    let server_handle = spawn_server(0, &temp_dir).await.expect("Failed to spawn server");
    assert!(server_handle.port() > 0);
    assert!(server_handle.url().starts_with("http://127.0.0.1:"));

    // Verify health endpoint
    let health_url = format!("{}/health", server_handle.url());
    let resp = reqwest::get(&health_url).await.expect("Health check GET failed");
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, "OK");

    // Clean server stop
    server_handle.stop().await.expect("Server failed to stop cleanly");
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_subsystem_headless_chrome_frame_capture() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dioxuscut_tier3_chrome_{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let index_file = temp_dir.join("index.html");
    fs::write(
        &index_file,
        r#"<!DOCTYPE html>
<html>
<head><title>Frame Capture Test</title></head>
<body>
<div id="content" style="background-color: #123456; width: 640px; height: 360px;">Frame Test</div>
<script>
  window.DIOXUSCUT_FRAME = 0;
</script>
</body>
</html>"#,
    )
    .unwrap();

    let server_handle = spawn_server(0, &temp_dir).await.expect("Failed to spawn server");
    let frames_dir = temp_dir.join("frames");

    let config = RenderConfig::new(
        server_handle.url().to_string(),
        &frames_dir,
        640,
        360,
        30.0,
        3, // 3 frames: 0, 1, 2
    );

    let frame_paths = render_frames(&config).await.expect("render_frames failed");
    assert_eq!(frame_paths.len(), 3);

    for path in &frame_paths {
        assert!(path.exists(), "Frame PNG file missing at {}", path.display());
        let metadata = fs::metadata(path).unwrap();
        assert!(metadata.len() > 0, "Frame PNG file is empty at {}", path.display());

        // Validate PNG Magic Header Signature: \x89 PNG \r \n \x1a \n
        let bytes = fs::read(path).unwrap();
        assert!(bytes.len() >= 8);
        assert_eq!(&bytes[0..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    server_handle.stop().await.unwrap();
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_subsystem_ffmpeg_mp4_encoding() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dioxuscut_tier3_ffmpeg_{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let index_file = temp_dir.join("index.html");
    fs::write(
        &index_file,
        r#"<!DOCTYPE html>
<html>
<head><title>Encoding Test</title></head>
<body style="background-color: red;">
<h1>FFmpeg Encoding Test</h1>
</body>
</html>"#,
    )
    .unwrap();

    let server_handle = spawn_server(0, &temp_dir).await.expect("Failed to spawn server");
    let frames_dir = temp_dir.join("frames");

    let render_config = RenderConfig::new(
        server_handle.url().to_string(),
        &frames_dir,
        640,
        360,
        30.0,
        5, // 5 frames
    );

    // 1. Capture real PNG screenshots via Headless Chrome
    let _frame_paths = render_frames(&render_config).await.expect("Frame rendering failed");

    // 2. Encode rendered PNG sequence into MP4 video via FFmpeg
    let output_mp4 = temp_dir.join("output_test.mp4");
    let encode_cfg = EncodeConfig::h264(&frames_dir, &output_mp4, 30.0);

    encode_frames(&encode_cfg).await.expect("FFmpeg encoding failed");

    assert!(output_mp4.exists(), "Output MP4 file does not exist");
    let metadata = fs::metadata(&output_mp4).unwrap();
    assert!(metadata.len() > 0, "Output MP4 file is empty");

    // Verify MP4 container header signature ("ftyp" atom)
    let bytes = fs::read(&output_mp4).unwrap();
    assert!(bytes.len() > 12);
    let ftyp_slice = &bytes[4..8];
    assert_eq!(ftyp_slice, b"ftyp", "MP4 file missing 'ftyp' container header");

    server_handle.stop().await.unwrap();
    let _ = fs::remove_dir_all(&temp_dir);
}
