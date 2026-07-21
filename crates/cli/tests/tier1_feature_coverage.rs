//! Tier 1: Feature Coverage E2E Tests
//!
//! Tests CLI argument parsing for `--composition`, `--props`, `--output`,
//! `--width`, `--height`, `--fps`, `--duration`, and `--backend`.

use clap::Parser;
use dioxuscut_cli::{Cli, Commands, RenderBackend};
use std::path::PathBuf;

#[test]
fn test_cli_flag_defaults() {
    let args = vec!["dioxuscut", "render", "-c", "HelloWorld"];
    let cli = Cli::try_parse_from(args).expect("Failed to parse CLI args");

    match cli.command {
        Commands::Render {
            composition,
            props,
            output,
            width,
            height,
            fps,
            duration,
            backend,
        } => {
            assert_eq!(composition, "HelloWorld");
            assert_eq!(props, None);
            assert_eq!(output, PathBuf::from("out.mp4"));
            assert_eq!(width, 1920);
            assert_eq!(height, 1080);
            assert!((fps - 30.0).abs() < f64::EPSILON);
            assert_eq!(duration, 150);
            assert_eq!(backend, RenderBackend::Native);
        }
    }
}

#[test]
fn test_cli_flag_custom_values() {
    let args = vec![
        "dioxuscut",
        "render",
        "-c",
        "CustomComposition",
        "-p",
        "input_data.json",
        "-o",
        "result_video.mp4",
        "--width",
        "1280",
        "--height",
        "720",
        "--fps",
        "60.0",
        "--duration",
        "300",
    ];

    let cli = Cli::try_parse_from(args).expect("Failed to parse custom CLI args");

    match cli.command {
        Commands::Render {
            composition,
            props,
            output,
            width,
            height,
            fps,
            duration,
            backend,
        } => {
            assert_eq!(composition, "CustomComposition");
            assert_eq!(props, Some(PathBuf::from("input_data.json")));
            assert_eq!(output, PathBuf::from("result_video.mp4"));
            assert_eq!(width, 1280);
            assert_eq!(height, 720);
            assert!((fps - 60.0).abs() < f64::EPSILON);
            assert_eq!(duration, 300);
            assert_eq!(backend, RenderBackend::Native);
        }
    }
}

#[test]
fn test_cli_short_flags() {
    let args = vec![
        "dioxuscut",
        "render",
        "-c",
        "ShortFlagComp",
        "-p",
        "props.json",
        "-o",
        "out_short.mp4",
    ];

    let cli = Cli::try_parse_from(args).expect("Failed to parse short CLI args");

    match cli.command {
        Commands::Render {
            composition,
            props,
            output,
            ..
        } => {
            assert_eq!(composition, "ShortFlagComp");
            assert_eq!(props, Some(PathBuf::from("props.json")));
            assert_eq!(output, PathBuf::from("out_short.mp4"));
        }
    }
}

#[test]
fn test_cli_missing_required_composition() {
    let args = vec!["dioxuscut", "render"];
    let result = Cli::try_parse_from(args);
    assert!(
        result.is_err(),
        "Expected parsing error when --composition is missing"
    );
}
