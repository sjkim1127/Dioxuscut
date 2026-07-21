//! Dioxuscut Shapes — procedural SVG motion graphics components.
//!
//! Provides components and path generators corresponding to `@remotion/shapes`:
//! - [`Circle`] / [`make_circle`]
//! - [`Rect`] / [`make_rect`]
//! - [`Triangle`] / [`make_triangle`]
//! - [`Star`] / [`make_star`]
//! - [`Polygon`] / [`make_polygon`]
//! - [`Pie`] / [`make_pie`]
//! - [`Arrow`] / [`make_arrow`]
//! - [`RenderSvg`]

pub mod arrow;
pub mod circle;
pub mod pie;
pub mod polygon;
pub mod rect;
pub mod render_svg;
pub mod star;
pub mod triangle;

pub use arrow::{make_arrow, Arrow, ArrowProps};
pub use circle::{make_circle, Circle, CircleProps};
pub use pie::{make_pie, Pie, PieProps};
pub use polygon::{make_polygon, Polygon, PolygonProps};
pub use rect::{make_rect, Rect, RectProps};
pub use render_svg::{RenderSvg, RenderSvgProps};
pub use star::{make_star, Star, StarProps};
pub use triangle::{make_triangle, Triangle, TriangleProps};
