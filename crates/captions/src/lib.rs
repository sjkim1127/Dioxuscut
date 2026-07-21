//! Dioxuscut Captions — Subtitle parsing, word timing, and kinetic animated subtitles.
//!
//! Ported from `@remotion/captions`:
//! - [`parse_srt`] / [`serialize_srt`]
//! - [`ensure_max_characters_per_line`]
//! - [`create_tiktok_style_captions`]
//! - [`TikTokCaptions`]

pub mod types;
pub mod srt_parser;
pub mod line_wrapper;
pub mod tiktok_captions;

pub use types::{CaptionToken, CaptionPage};
pub use srt_parser::{parse_srt, serialize_srt, format_srt_timestamp, CaptionParseError};
pub use line_wrapper::ensure_max_characters_per_line;
pub use tiktok_captions::{create_tiktok_style_captions, TikTokCaptions, TikTokCaptionsProps};
