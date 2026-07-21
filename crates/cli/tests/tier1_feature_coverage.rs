//! Tier 1: Feature Coverage E2E Tests
//!
//! Tests CLI argument parsing for `--composition`, `--props`, `--output`,
//! `--width`, `--height`, `--fps`, `--duration`, `--port`, `--web-dir`, and `--server-url`.

use clap::Parser;
use dioxuscut_cli::{Cli, Commands};
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
            port,
            web_dir,
            server_url,
        } => {
            assert_eq!(composition, "HelloWorld");
            assert_eq!(props, None);
            assert_eq!(output, PathBuf::from("out.mp4"));
            assert_eq!(width, 1920);
            assert_eq!(height, 1080);
            assert!((fps - 30.0).abs() < f64::EPSILON);
            assert_eq!(duration, 150);
            assert_eq!(port, 0);
            assert_eq!(web_dir, None);
            assert_eq!(server_url, None);
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
        "--port",
        "9000",
        "--web-dir",
        "build",
        "--server-url",
        "http://127.0.0.1:9000",
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
            port,
            web_dir,
            server_url,
        } => {
            assert_eq!(composition, "CustomComposition");
            assert_eq!(props, Some(PathBuf::from("input_data.json")));
            assert_eq!(output, PathBuf::from("result_video.mp4"));
            assert_eq!(width, 1280);
            assert_eq!(height, 720);
            assert!((fps - 60.0).abs() < f64::EPSILON);
            assert_eq!(duration, 300);
            assert_eq!(port, 9000);
            assert_eq!(web_dir, Some(PathBuf::from("build")));
            assert_eq!(server_url, Some("http://127.0.0.1:9000".to_string()));
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
    assert!(result.is_err(), "Expected parsing error when --composition is missing");
}
