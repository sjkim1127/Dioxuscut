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

    let mut state = use_signal(|| PlayerState {
        frame: props.initial_frame,
        playing: false,
        duration,
    });

    // Advance frame on each animation tick when playing
    use_future(move || async move {
        loop {
            // ~16ms per tick (≈60Hz polling)
            tokio::time::sleep(tokio::time::Duration::from_millis(16)).await;
            let s = state.read();
            if s.playing {
                let next_frame = s.frame + 1;
                let looped = if next_frame >= s.duration {
                    0
                } else {
                    next_frame
                };
                drop(s);
                state.write().frame = looped;
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
