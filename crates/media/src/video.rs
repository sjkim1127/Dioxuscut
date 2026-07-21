//! `<Video>` component — HTML video element synchronized with the composition frame.
//!
//! Equivalent to Remotion's `<Video>`.

use dioxus::prelude::*;
use dioxuscut_core::hooks::use_current_frame;
use dioxuscut_core::hooks::use_video_config;

/// Props for `<Video>`.
#[derive(Props, Clone, PartialEq)]
pub struct VideoProps {
    /// Video source URL.
    pub src: String,
    /// Volume (0.0–1.0). Default: `1.0`.
    #[props(default = 1.0)]
    pub volume: f64,
    /// Playback rate multiplier. Default: `1.0`.
    #[props(default = 1.0)]
    pub playback_rate: f64,
    /// Start time offset into the source video (seconds).
    #[props(default = 0.0)]
    pub start_from: f64,
    /// Extra inline style.
    #[props(default)]
    pub style: Option<String>,
    /// Mute the video.
    #[props(default = false)]
    pub muted: bool,
}

/// A video element synchronized to the composition timeline.
///
/// During rendering/export, the video is seeked to the correct time.
/// During preview (Player), it plays normally.
#[component]
pub fn Video(props: VideoProps) -> Element {
    let frame = use_current_frame();
    let config = use_video_config();

    // Compute the source video timestamp for this frame
    let time_in_seconds = props.start_from + (frame as f64 / config.fps);

    let base_style = "width: 100%; height: 100%; object-fit: cover;";
    let style = match &props.style {
        Some(extra) => format!("{base_style} {extra}"),
        None => base_style.to_string(),
    };

    rsx! {
        video {
            src: "{props.src}",
            style: "{style}",
            muted: props.muted,
            // Data attributes carry rendering metadata
            "data-remotion-seek": "{time_in_seconds}",
            "data-remotion-volume": "{props.volume}",
            "data-remotion-playback-rate": "{props.playback_rate}",
        }
    }
}
