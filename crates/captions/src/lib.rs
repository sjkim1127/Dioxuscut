//! Dioxuscut Captions — Subtitle parsing, word timing, and kinetic animated subtitles.
//!
//! Ported from `@remotion/captions`:
//! - [`parse_srt`] / [`serialize_srt`]
//! - [`ensure_max_characters_per_line`]
//! - [`create_tiktok_style_captions`]
//! - [`TikTokCaptions`]

pub mod line_wrapper;
pub mod scene;
pub mod srt_parser;
pub mod tiktok_captions;
pub mod types;

pub use line_wrapper::ensure_max_characters_per_line;
pub use scene::SceneCaptions;
pub use srt_parser::{format_srt_timestamp, parse_srt, serialize_srt, CaptionParseError};
pub use tiktok_captions::{create_tiktok_style_captions, TikTokCaptions, TikTokCaptionsProps};
pub use types::{CaptionPage, CaptionToken};
