//! `<Audio>` component — audio element synchronized with the composition frame.
//!
//! Equivalent to Remotion's `<Audio>`.

use dioxuscut_core::hooks::use_current_frame;
use dioxuscut_core::hooks::use_video_config;
use dioxus::prelude::*;

/// Props for `<Audio>`.
#[derive(Props, Clone, PartialEq)]
pub struct AudioProps {
    /// Audio source URL.
    pub src: String,
    /// Volume (0.0–1.0). Default: `1.0`.
    #[props(default = 1.0)]
    pub volume: f64,
    /// Start time offset into the audio file (seconds).
    #[props(default = 0.0)]
    pub start_from: f64,
    /// Playback rate multiplier. Default: `1.0`.
    #[props(default = 1.0)]
    pub playback_rate: f64,
    /// Mute the audio (useful during preview).
    #[props(default = false)]
    pub muted: bool,
}

/// An audio element synchronized to the composition timeline.
#[component]
pub fn Audio(props: AudioProps) -> Element {
    let frame = use_current_frame();
    let config = use_video_config();

    let time_in_seconds = props.start_from + (frame as f64 / config.fps);

    rsx! {
        audio {
            src: "{props.src}",
            muted: props.muted,
            "data-remotion-seek": "{time_in_seconds}",
            "data-remotion-volume": "{props.volume}",
            "data-remotion-playback-rate": "{props.playback_rate}",
        }
    }
}
