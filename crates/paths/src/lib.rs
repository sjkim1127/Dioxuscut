//! Dioxuscut Paths — SVG path parsing, metrics, and stroke evolution utilities.
//!
//! Ported from `@remotion/paths`:
//! - [`parse_path`] / [`serialize_instructions`]
//! - [`get_length`] / [`get_instructions_length`]
//! - [`evolve_path`] / [`evolve_path_with_length`]
//! - [`translate_path`] / [`scale_path`]
//! - [`get_point_at_length`]
//! - [`approximate_path_length`]
//! - [`interpolate_path`]

pub mod evolve_path;
pub mod interpolate;
pub mod length;
pub mod parser;
pub mod point_at_length;
pub mod transform;
pub mod types;

pub use evolve_path::{evolve_path, evolve_path_with_length};
pub use interpolate::{approximate_path_length, interpolate_path};
pub use length::{arc_segment_length, get_instructions_length, get_length};
pub use parser::{parse_path, serialize_instructions, PathParseError};
pub use point_at_length::get_point_at_length;
pub use transform::{scale_path, translate_path};
pub use types::{EvolvedPath, Instruction, Point};
