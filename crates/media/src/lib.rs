//! # dioxuscut-media
//!
//! Media components for Dioxuscut — Img, Video, Audio.
//!
//! These are Dioxus component wrappers around HTML media elements,
//! synchronized with the composition timeline.

pub mod audio;
pub mod img;
pub mod video;

pub use audio::{Audio, AudioProps};
pub use img::{Img, ImgProps, ImageFit};
pub use video::{Video, VideoProps};
