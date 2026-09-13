//! # dioxuscut-media
//!
//! Media components for Dioxuscut — Img, Video, Audio.
//!
//! These are Dioxus component wrappers around HTML media elements,
//! synchronized with the composition timeline.

pub mod audio;
pub mod audio_viz;
pub mod canvas_image;
pub mod ducking;
pub mod gif;
pub mod img;
pub mod metadata;
pub mod scene;
pub mod sample;
pub mod video;

pub use audio::{Audio, AudioProps, Html5Audio};
pub use audio_viz::{
    create_smooth_svg_path, get_waveform_bars, get_waveform_portion, get_waveform_portion_channel,
    load_audio_data, visualize_audio, visualize_audio_waveform, AudioData, AudioVizError,
    VisualizeAudioWaveformOptions, VisualizeFor, WaveformBarsOptions,
};
pub use canvas_image::{CanvasImage, CanvasImageProps};
pub use ducking::{
    calculate_ducking_envelope, merge_speech_intervals, DuckingOptions, SpeechInterval,
};
pub use gif::{Gif, GifProps};
pub use img::{ImageFit, Img, ImgProps};
pub use metadata::{
    get_audio_metadata, get_image_dimensions, get_video_metadata, parse_media, read_media_range,
    static_file, AudioMetadata, ImageDimensions, MediaMetadataError, ParsedMediaMetadata,
    VideoMetadata, MAX_MEDIA_RANGE_BYTES,
};
pub use scene::{SceneAudio, SceneGif, SceneImage, SceneVideo};
pub use sample::EncodedSample;
pub use video::{Html5Video, OffthreadVideo, Video, VideoProps};
