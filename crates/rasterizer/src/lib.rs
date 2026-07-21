//! Dioxuscut Rasterizer — Native browser-free frame rendering.
//!
//! Replaces Headless Chrome with a pure-Rust rasterizer pipeline.
//!
//! # Backends
//!
//! - [`TinySkiaBackend`]: CPU rasterizer using `tiny-skia`. Works everywhere — no GPU or browser required.
//!
//! # Example
//!
//! ```rust,no_run
//! use dioxuscut_rasterizer::{TinySkiaBackend, Scene, SceneNode, Color};
//! use dioxuscut_rasterizer::backend::{FrameConfig, RasterizerBackend};
//!
//! let backend = TinySkiaBackend::new();
//! let mut scene = Scene::new();
//! scene.push(SceneNode::Rect {
//!     x: 0.0, y: 0.0, w: 1920.0, h: 1080.0,
//!     fill: Color::rgb(15, 23, 42),
//!     stroke: None, stroke_width: 0.0, corner_radius: 0.0,
//! });
//!
//! let config = FrameConfig::new(1920, 1080, 0, 30.0);
//! let img = backend.render_frame(&scene, &config).unwrap();
//! img.save("frame_000001.png").unwrap();
//! ```

pub mod backend;
pub mod font;
pub mod scene;
pub mod render;
pub mod tiny_skia_backend;
#[cfg(feature = "gpu")]
pub mod wgpu_backend;

pub use backend::{FrameConfig, RasterError, RasterizerBackend};
pub use font::FontCache;
pub use scene::{Color, GradientStop, Scene, SceneNode, Transform2D};
pub use tiny_skia_backend::TinySkiaBackend;
pub use render::{NativeRenderConfig, render_all_frames, save_frame};
#[cfg(feature = "gpu")]
pub use wgpu_backend::WgpuBackend;
