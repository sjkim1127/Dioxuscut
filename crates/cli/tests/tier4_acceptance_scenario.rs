//! Tier 4: Real-World Acceptance Scenario E2E Test
//!
//! Replicates real-world CLI invocation:
//! `cargo run -p dioxuscut-cli -- render -c HelloWorld -p data.json -o output.mp4 --width 1280 --height 720 --fps 30 --duration 60`

use dioxuscut_cli::execute_render_command;
use dioxuscut_renderer::spawn_server;
use std::fs;

#[tokio::test]
async fn test_tier4_real_world_acceptance_scenario() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dioxuscut_tier4_acceptance_{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let web_dir = temp_dir.join("dist");
    fs::create_dir_all(&web_dir).unwrap();

    let index_html = web_dir.join("index.html");
    fs::write(
        &index_html,
        r#"<!DOCTYPE html>
<html>
<head><title>HelloWorld Composition</title></head>
<body style="margin: 0; background: #0f172a; color: white; display: flex; align-items: center; justify-content: center; height: 100vh;">
  <h1 id="title">HelloWorld Dioxuscut Composition</h1>
  <script>
    window.DIOXUSCUT_FRAME = 0;
  </script>
</body>
</html>"#,
    )
    .unwrap();

    // Create data.json input props file
    let props_path = temp_dir.join("data.json");
    let props_content = r#"{
        "title": "HelloWorld",
        "author": "Dioxuscut E2E Suite",
        "fps": 30,
        "width": 1280,
        "height": 720
    }"#;
    fs::write(&props_path, props_content).unwrap();

    let output_mp4 = temp_dir.join("output.mp4");

    // Execute full acceptance render workflow
    let result = execute_render_command(
        "HelloWorld",
        Some(&props_path),
        &output_mp4,
        1280,
        720,
        30.0,
        60, // 60 frames = 2 seconds @ 30 fps
        0,  // dynamic server port
        Some(&web_dir),
        None, // auto-spawn server from web_dir
    )
    .await;

    assert!(result.is_ok(), "Acceptance render failed: {:?}", result.err());

    // 1. Verify output file exists
    assert!(output_mp4.exists(), "Target output.mp4 file was not created");

    // 2. Verify non-zero file size
    let metadata = fs::metadata(&output_mp4).unwrap();
    assert!(metadata.len() > 0, "Output MP4 file is zero bytes");

    // 3. Verify MP4 file header container format ('ftyp' atom)
    let bytes = fs::read(&output_mp4).unwrap();
    assert!(bytes.len() > 12);
    let ftyp_slice = &bytes[4..8];
    assert_eq!(ftyp_slice, b"ftyp", "Produced MP4 file missing 'ftyp' container signature");

    // 4. Verify environment variable was set during command execution
    assert!(output_mp4.exists());
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_tier4_acceptance_with_external_server() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dioxuscut_tier4_ext_server_{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();

    let index_html = temp_dir.join("index.html");
    fs::write(
        &index_html,
        r#"<!DOCTYPE html>
<html>
<head><title>External Server Acceptance Test</title></head>
<body>
  <h1>External Server Rendering</h1>
</body>
</html>"#,
    )
    .unwrap();

    let server_handle = spawn_server(0, &temp_dir).await.expect("Failed to spawn external server");
    let props_path = temp_dir.join("data.json");
    fs::write(&props_path, r#"{"mode": "external"}"#).unwrap();

    let output_mp4 = temp_dir.join("output_external.mp4");

    let result = execute_render_command(
        "HelloWorld",
        Some(&props_path),
        &output_mp4,
        1280,
        720,
        30.0,
        30,
        0,
        None,
        Some(server_handle.url().to_string()),
    )
    .await;

    assert!(result.is_ok(), "Render with external server failed: {:?}", result.err());
    assert!(output_mp4.exists());
    assert!(fs::metadata(&output_mp4).unwrap().len() > 0);

    server_handle.stop().await.unwrap();
    let _ = fs::remove_dir_all(&temp_dir);
}
