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

pub mod render_svg;
pub mod circle;
pub mod rect;
pub mod triangle;
pub mod star;
pub mod polygon;
pub mod pie;
pub mod arrow;

pub use render_svg::{RenderSvg, RenderSvgProps};
pub use circle::{Circle, CircleProps, make_circle};
pub use rect::{Rect, RectProps, make_rect};
pub use triangle::{Triangle, TriangleProps, make_triangle};
pub use star::{Star, StarProps, make_star};
pub use polygon::{Polygon, PolygonProps, make_polygon};
pub use pie::{Pie, PieProps, make_pie};
pub use arrow::{Arrow, ArrowProps, make_arrow};
