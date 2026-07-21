#![cfg(feature = "rhai")]

use dioxuscut_cli::{execute_render_command, RenderBackend, RenderRequest};
use std::fs;
use std::path::PathBuf;

#[tokio::test]
async fn rhai_example_renders_a_real_mp4() {
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("Skipping Rhai render integration test: ffmpeg is not installed");
        return;
    }

    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp_dir = std::env::temp_dir().join(format!(
        "dioxuscut_rhai_render_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let output = temp_dir.join("rhai.mp4");
    let request = RenderRequest {
        composition: None,
        script: Some(workspace.join("examples/hello.rhai")),
        props: Some(workspace.join("examples/hello-props.json")),
        output: output.clone(),
        width: 64,
        height: 64,
        fps: 30.0,
        duration: 3,
        backend: RenderBackend::Native,
    };

    execute_render_command(&request).await.unwrap();
    let bytes = fs::read(&output).unwrap();
    assert!(bytes.len() > 32);
    assert!(bytes.windows(4).any(|window| window == b"ftyp"));
    let _ = fs::remove_dir_all(temp_dir);
}
