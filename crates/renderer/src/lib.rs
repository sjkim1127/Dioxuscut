//! # dioxuscut-renderer
//!
//! Headless frame rendering and video encoding for Dioxuscut.
//!
//! Equivalent to `@remotion/renderer`.
//!
//! ## Architecture
//!
//! 1. [`RenderConfig`] — defines what to render and where to write output
//! 2. [`render_frames`] — renders each frame to an in-memory bitmap/PNG
//! 3. [`encode`] — stitches frames into a video file via FFmpeg

pub mod browser;
pub mod encode;
pub mod render_frames;
pub mod server;

pub use browser::capture_frames;
pub use encode::{EncodeConfig, build_ffmpeg_args, cleanup_frames, encode_frames, encode_mp4};
pub use render_frames::{RenderConfig, RenderError, render_frames};
pub use server::{ServeMode, ServerConfig, ServerError, ServerHandle, spawn_server, spawn_server_with_config};

