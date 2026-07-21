//! Dioxuscut Paths — SVG path parsing, metrics, and stroke evolution utilities.
//!
//! Ported from `@remotion/paths`:
//! - [`parse_path`] / [`serialize_instructions`]
//! - [`get_length`] / [`get_instructions_length`]
//! - [`evolve_path`]
//! - [`translate_path`] / [`scale_path`]
//! - [`get_point_at_length`]

pub mod types;
pub mod parser;
pub mod length;
pub mod evolve_path;
pub mod transform;
pub mod point_at_length;

pub use types::{Instruction, Point, EvolvedPath};
pub use parser::{parse_path, serialize_instructions, PathParseError};
pub use length::{get_length, get_instructions_length};
pub use evolve_path::evolve_path;
pub use transform::{translate_path, scale_path};
pub use point_at_length::get_point_at_length;
