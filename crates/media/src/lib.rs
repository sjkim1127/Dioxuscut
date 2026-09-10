//! # dioxuscut-media
//!
//! Media components for Dioxuscut — Img, Video, Audio.
//!
//! These are Dioxus component wrappers around HTML media elements,
//! synchronized with the composition timeline.

pub mod audio;
pub mod audio_viz;
pub mod gif;
pub mod img;
pub mod metadata;
pub mod scene;
pub mod video;

pub use audio::{Audio, AudioProps};
pub use audio_viz::{
    create_smooth_svg_path, get_waveform_portion, load_audio_data, visualize_audio, AudioData,
    AudioVizError, VisualizeFor,
};
pub use gif::{Gif, GifProps};
pub use img::{ImageFit, Img, ImgProps};
pub use metadata::{
    get_audio_metadata, get_video_metadata, static_file, AudioMetadata, MediaMetadataError,
    VideoMetadata,
};
pub use scene::{SceneAudio, SceneImage, SceneVideo};
pub use video::{Video, VideoProps};
