//! # Dioxuscut Studio
//!
//! Desktop video editor application — Remotion Studio equivalent.
//!
//! Provides:
//! - Composition preview with the `<Player>` component
//! - Timeline panel (planned)
//! - Render queue (planned)
//! - Properties panel (planned)

use dioxus::prelude::*;
use dioxus_desktop::{Config, LogicalSize, WindowBuilder};
use dioxuscut_core::AbsoluteFill;
use dioxuscut_player::Player;

// ─── Window config ────────────────────────────────────────────────────────────
const WINDOW_WIDTH: u32 = 1600;
const WINDOW_HEIGHT: u32 = 960;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("info,studio=debug")
        .init();

    let window = WindowBuilder::new()
        .with_title("Dioxuscut Studio")
        .with_inner_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
        .with_resizable(true);

    dioxus_desktop::launch::launch(
        StudioApp,
        vec![],
        vec![Box::new(Config::new().with_window(window))],
    );
}

// ─── App shell ────────────────────────────────────────────────────────────────

#[component]
fn StudioApp() -> Element {
    rsx! {
        div {
            style: "
                display: grid;
                grid-template-rows: 48px 1fr 200px;
                grid-template-columns: 240px 1fr 280px;
                height: 100vh;
                background: #0d0d14;
                color: #e8e8f0;
                font-family: 'Inter', system-ui, sans-serif;
                overflow: hidden;
            ",

            // ── Top bar ────────────────────────────────────────────────────
            div {
                style: "
                    grid-column: 1 / -1;
                    background: #16161f;
                    border-bottom: 1px solid rgba(255,255,255,0.07);
                    display: flex; align-items: center; gap: 16px; padding: 0 20px;
                ",
                span {
                    style: "font-size: 15px; font-weight: 700; color: #6c63ff; letter-spacing: -0.02em;",
                    "🦀 Dioxuscut Studio"
                }
                span { style: "color: rgba(255,255,255,0.2);", "│" }
                span {
                    style: "font-size: 13px; color: rgba(255,255,255,0.5);",
                    "Untitled Project"
                }
                div { style: "flex: 1;" }
                button {
                    style: "
                        background: #6c63ff; color: white; border: none;
                        padding: 6px 16px; border-radius: 6px; font-size: 13px;
                        cursor: pointer; font-weight: 600;
                    ",
                    "▶ Render"
                }
            }

            // ── Left panel: Compositions list ──────────────────────────────
            div {
                style: "
                    background: #12121b;
                    border-right: 1px solid rgba(255,255,255,0.07);
                    padding: 16px;
                    overflow-y: auto;
                ",
                h3 {
                    style: "font-size: 11px; text-transform: uppercase; letter-spacing: 0.08em; color: rgba(255,255,255,0.35); margin: 0 0 12px;",
                    "Compositions"
                }
                CompositionListItem { name: "HelloWorld", selected: true }
                CompositionListItem { name: "TitleCard", selected: false }
                CompositionListItem { name: "Outro", selected: false }
            }

            // ── Centre: Preview ────────────────────────────────────────────
            div {
                style: "
                    display: flex; flex-direction: column;
                    align-items: center; justify-content: center;
                    background: #0a0a12; padding: 24px; gap: 16px;
                    overflow: auto;
                ",
                Player {
                    width: 960,
                    height: 540,
                    fps: 30.0,
                    duration_in_frames: 180,
                    controls: true,
                    PreviewComposition {}
                }
            }

            // ── Right panel: Properties ────────────────────────────────────
            div {
                style: "
                    background: #12121b;
                    border-left: 1px solid rgba(255,255,255,0.07);
                    padding: 16px; overflow-y: auto;
                ",
                h3 {
                    style: "font-size: 11px; text-transform: uppercase; letter-spacing: 0.08em; color: rgba(255,255,255,0.35); margin: 0 0 12px;",
                    "Properties"
                }
                PropertyRow { label: "Width",    value: "1920px" }
                PropertyRow { label: "Height",   value: "1080px" }
                PropertyRow { label: "FPS",      value: "30" }
                PropertyRow { label: "Duration", value: "6.0s (180f)" }
                PropertyRow { label: "Codec",    value: "H.264" }
            }

            // ── Bottom: Timeline ───────────────────────────────────────────
            div {
                style: "
                    grid-column: 1 / -1;
                    background: #10101a;
                    border-top: 1px solid rgba(255,255,255,0.07);
                    padding: 16px;
                    overflow-x: auto;
                ",
                TimelinePanel {}
            }
        }
    }
}

// ─── Composition list item ─────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct CompositionListItemProps {
    name: String,
    selected: bool,
}

#[component]
fn CompositionListItem(props: CompositionListItemProps) -> Element {
    let bg = if props.selected {
        "rgba(108, 99, 255, 0.15)"
    } else {
        "transparent"
    };
    let border = if props.selected {
        "1px solid rgba(108, 99, 255, 0.4)"
    } else {
        "1px solid transparent"
    };
    let color = if props.selected {
        "#c4bfff"
    } else {
        "rgba(255,255,255,0.6)"
    };

    rsx! {
        div {
            style: "
                padding: 8px 10px;
                border-radius: 6px;
                font-size: 13px;
                cursor: pointer;
                background: {bg};
                border: {border};
                color: {color};
                margin-bottom: 4px;
                transition: all 0.15s;
            ",
            "▶ {props.name}"
        }
    }
}

// ─── Property row ──────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
struct PropertyRowProps {
    label: String,
    value: String,
}

#[component]
fn PropertyRow(props: PropertyRowProps) -> Element {
    rsx! {
        div {
            style: "display: flex; justify-content: space-between; padding: 8px 0; border-bottom: 1px solid rgba(255,255,255,0.05); font-size: 13px;",
            span { style: "color: rgba(255,255,255,0.45);", "{props.label}" }
            span { style: "color: rgba(255,255,255,0.85); font-family: monospace; font-size: 12px;", "{props.value}" }
        }
    }
}

// ─── Timeline panel ────────────────────────────────────────────────────────────

#[component]
fn TimelinePanel() -> Element {
    let tracks = [
        ("Scene 1: Title", 0, 60, "#6c63ff"),
        ("Scene 2: Logo", 50, 70, "#22c55e"),
        ("Scene 3: Stats", 110, 70, "#f59e0b"),
    ];
    let total: u32 = 180;

    rsx! {
        div {
            h3 {
                style: "font-size: 11px; text-transform: uppercase; letter-spacing: 0.08em; color: rgba(255,255,255,0.35); margin: 0 0 12px;",
                "Timeline — 6.0s (180 frames)"
            }
            div {
                style: "display: flex; flex-direction: column; gap: 6px;",
                for (name, from, dur, color) in tracks {
                    div {
                        key: "{name}",
                        style: "display: flex; align-items: center; gap: 12px;",
                        // Track label
                        div {
                            style: "font-size: 12px; color: rgba(255,255,255,0.5); width: 140px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                            "{name}"
                        }
                        // Track bar
                        div {
                            style: "flex: 1; height: 24px; background: rgba(255,255,255,0.05); border-radius: 4px; position: relative;",
                            div {
                                style: "
                                    position: absolute;
                                    left: {from as f64 / total as f64 * 100.0:.1}%;
                                    width: {dur as f64 / total as f64 * 100.0:.1}%;
                                    height: 100%;
                                    background: {color};
                                    opacity: 0.7;
                                    border-radius: 4px;
                                    display: flex; align-items: center; padding: 0 6px;
                                    font-size: 11px; color: white; white-space: nowrap; overflow: hidden;
                                ",
                                "{from}f"
                            }
                        }
                    }
                }
            }
        }
    }
}

// ─── Preview composition (placeholder) ────────────────────────────────────────

#[component]
fn PreviewComposition() -> Element {
    use dioxuscut_core::hooks::use_current_frame;
    let frame = use_current_frame();

    rsx! {
        AbsoluteFill {
            style: "background: linear-gradient(135deg, #1a0533 0%, #0d1b4b 50%, #001a33 100%); display: flex; align-items: center; justify-content: center; flex-direction: column; gap: 12px;",
            div {
                style: "font-size: 56px; font-weight: 800; color: white; letter-spacing: -0.04em;",
                "🦀 Dioxuscut"
            }
            div {
                style: "color: rgba(200, 180, 255, 0.8); font-size: 20px;",
                "Frame {frame} / 179"
            }
            div {
                style: "color: rgba(255,255,255,0.3); font-size: 13px; font-family: monospace;",
                "Open apps/example/src/main.rs to load your composition"
            }
        }
    }
}
