//! `<Player>` component — interactive video player with playback controls.
//!
//! Equivalent to `@remotion/player`'s `<Player>`.
//!
//! # Example
//! ```rust,ignore
//! use dioxuscut_player::Player;
//!
//! fn App() -> Element {
//!     rsx! {
//!         Player {
//!             component: MyComposition,
//!             width: 1920,
//!             height: 1080,
//!             fps: 30.0,
//!             duration_in_frames: 150,
//!             controls: true,
//!         }
//!     }
//! }
//! ```

use dioxus::prelude::*;
use dioxuscut_core::Composition;

use crate::controls::Controls;

/// Props for the `<Player>` component.
#[derive(Props, Clone, PartialEq)]
pub struct PlayerProps {
    /// Composition width.
    pub width: u32,
    /// Composition height.
    pub height: u32,
    /// Frames per second.
    pub fps: f64,
    /// Total frame count.
    pub duration_in_frames: u32,
    /// Show playback controls below the canvas.
    #[props(default = true)]
    pub controls: bool,
    /// Initial frame to start at.
    #[props(default = 0)]
    pub initial_frame: u32,
    /// Whether the player should loop.
    #[props(default = true)]
    pub loop_playback: bool,
    /// The composition content to render.
    pub children: Element,
}

/// State tracked internally by the player.
#[derive(Clone, Debug, PartialEq)]
struct PlayerState {
    frame: u32,
    playing: bool,
    duration: u32,
}

/// Interactive video player component.
///
/// Wraps a `<Composition>` and adds play/pause/seek controls driven by
/// Dioxus reactive state.
#[component]
pub fn Player(props: PlayerProps) -> Element {
    let duration = props.duration_in_frames;
    let fps = props.fps;
    let loop_playback = props.loop_playback;
    let tick_duration = frame_duration(fps);

    let mut state = use_signal(|| PlayerState {
        frame: props.initial_frame,
        playing: false,
        duration,
    });

    // Advance frame on each animation tick when playing
    use_future(move || async move {
        loop {
            tokio::time::sleep(tick_duration).await;
            let s = state.read();
            if s.playing {
                let (next_frame, keep_playing) = advance_frame(s.frame, s.duration, loop_playback);
                drop(s);
                let mut next_state = state.write();
                next_state.frame = next_frame;
                next_state.playing = keep_playing;
            }
        }
    });

    let current_frame = state.read().frame;
    let is_playing = state.read().playing;

    let on_play_pause = move |_| {
        let was_playing = state.read().playing;
        state.write().playing = !was_playing;
    };

    let on_seek = move |f: u32| {
        state.write().frame = f.min(duration.saturating_sub(1));
    };

    rsx! {
        div {
            class: "dioxuscut-player",
            style: "display: flex; flex-direction: column; align-items: center; gap: 8px; font-family: sans-serif;",

            // Composition viewport
            div {
                style: "position: relative; width: {props.width}px; height: {props.height}px; overflow: hidden; border-radius: 8px; box-shadow: 0 4px 24px rgba(0,0,0,0.4);",

                Composition {
                    id: "player-composition",
                    width: props.width,
                    height: props.height,
                    fps,
                    duration_in_frames: duration,
                    frame: current_frame,
                    {props.children}
                }

                // Frame counter overlay
                div {
                    style: "position: absolute; top: 8px; right: 10px; color: rgba(255,255,255,0.7); font-size: 11px; font-family: monospace; background: rgba(0,0,0,0.4); padding: 2px 6px; border-radius: 4px; pointer-events: none;",
                    "{current_frame} / {duration.saturating_sub(1)}"
                }
            }

            // Controls
            if props.controls {
                Controls {
                    frame: current_frame,
                    duration,
                    playing: is_playing,
                    on_play_pause,
                    on_seek,
                }
            }
        }
    }
}

fn frame_duration(fps: f64) -> tokio::time::Duration {
    let safe_fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    };
    tokio::time::Duration::from_secs_f64(1.0 / safe_fps)
}

fn advance_frame(frame: u32, duration: u32, loop_playback: bool) -> (u32, bool) {
    if duration == 0 {
        return (0, false);
    }

    let next = frame.saturating_add(1);
    if next < duration {
        (next, true)
    } else if loop_playback {
        (0, true)
    } else {
        (duration - 1, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_duration_respects_fps() {
        assert_eq!(frame_duration(25.0), tokio::time::Duration::from_millis(40));
        assert_eq!(
            frame_duration(0.0),
            tokio::time::Duration::from_secs_f64(1.0 / 30.0)
        );
    }

    #[test]
    fn playback_stops_on_the_last_frame_when_looping_is_disabled() {
        assert_eq!(advance_frame(8, 10, false), (9, true));
        assert_eq!(advance_frame(9, 10, false), (9, false));
    }

    #[test]
    fn playback_wraps_when_looping_is_enabled() {
        assert_eq!(advance_frame(9, 10, true), (0, true));
    }
}
