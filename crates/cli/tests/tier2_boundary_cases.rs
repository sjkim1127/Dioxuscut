//! Tier 2: Boundary & Corner Cases E2E Tests
//!
//! Tests edge cases: empty composition name, missing props file, zero/odd resolutions,
//! zero FPS/duration, and malformed props JSON.

use dioxuscut_cli::{validate_render_params, ValidationError};
use std::fs;
use std::path::PathBuf;

#[test]
fn test_boundary_empty_composition() {
    let res = validate_render_params("", None, 1920, 1080, 30.0, 150);
    assert_eq!(res, Err(ValidationError::EmptyComposition));

    let res_whitespace = validate_render_params("   ", None, 1920, 1080, 30.0, 150);
    assert_eq!(res_whitespace, Err(ValidationError::EmptyComposition));
}

#[test]
fn test_boundary_missing_props_file() {
    let missing_path = PathBuf::from("/non_existent_directory/missing_props_file.json");
    let res = validate_render_params("HelloWorld", Some(&missing_path), 1920, 1080, 30.0, 150);
    assert_eq!(res, Err(ValidationError::PropsFileNotFound(missing_path)));
}

#[test]
fn test_boundary_zero_resolution() {
    let res_zero_w = validate_render_params("HelloWorld", None, 0, 1080, 30.0, 150);
    assert_eq!(
        res_zero_w,
        Err(ValidationError::InvalidZeroResolution(0, 1080))
    );

    let res_zero_h = validate_render_params("HelloWorld", None, 1920, 0, 30.0, 150);
    assert_eq!(
        res_zero_h,
        Err(ValidationError::InvalidZeroResolution(1920, 0))
    );
}

#[test]
fn test_boundary_odd_resolution() {
    let res_odd_w = validate_render_params("HelloWorld", None, 1921, 1080, 30.0, 150);
    assert_eq!(
        res_odd_w,
        Err(ValidationError::InvalidOddResolution(1921, 1080))
    );

    let res_odd_h = validate_render_params("HelloWorld", None, 1920, 1081, 30.0, 150);
    assert_eq!(
        res_odd_h,
        Err(ValidationError::InvalidOddResolution(1920, 1081))
    );
}

#[test]
fn test_boundary_zero_fps() {
    let res_zero_fps = validate_render_params("HelloWorld", None, 1920, 1080, 0.0, 150);
    assert_eq!(
        res_zero_fps,
        Err(ValidationError::InvalidFps("0".to_string()))
    );

    let res_neg_fps = validate_render_params("HelloWorld", None, 1920, 1080, -10.0, 150);
    assert_eq!(
        res_neg_fps,
        Err(ValidationError::InvalidFps("-10".to_string()))
    );
}

#[test]
fn test_boundary_zero_duration() {
    let res_zero_dur = validate_render_params("HelloWorld", None, 1920, 1080, 30.0, 0);
    assert_eq!(res_zero_dur, Err(ValidationError::InvalidDuration(0)));
}

#[tokio::test]
async fn test_boundary_valid_props_file_check() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dioxuscut_test_boundary_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let props_path = temp_dir.join("valid_props.json");
    fs::write(&props_path, r#"{"title": "Test Title"}"#).unwrap();

    let res = validate_render_params("HelloWorld", Some(&props_path), 1920, 1080, 30.0, 150);
    assert!(res.is_ok());

    let _ = fs::remove_dir_all(&temp_dir);
}
