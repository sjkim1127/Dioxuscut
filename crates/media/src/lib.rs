//! # dioxuscut-media
//!
//! Media components for Dioxuscut — Img, Video, Audio.
//!
//! These are Dioxus component wrappers around HTML media elements,
//! synchronized with the composition timeline.

pub mod audio;
pub mod img;
pub mod scene;
pub mod video;

pub use audio::{Audio, AudioProps};
pub use img::{ImageFit, Img, ImgProps};
pub use scene::{SceneAudio, SceneImage, SceneVideo};
pub use video::{Video, VideoProps};
