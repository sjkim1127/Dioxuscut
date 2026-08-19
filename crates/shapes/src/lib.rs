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
//! - [`Heart`] / [`make_heart`]
//! - [`Callout`] / [`make_callout`]
//! - [`Spark`] / [`make_spark`]
//! - [`ShapeOutput`]
//! - [`RenderSvg`]

pub mod arrow;
pub mod callout;
pub mod circle;
pub mod heart;
pub mod pie;
pub mod polygon;
pub mod rect;
pub mod render_svg;
pub mod scene;
pub mod shape_output;
pub mod spark;
pub mod star;
pub mod triangle;

pub use arrow::{make_arrow, Arrow, ArrowProps};
pub use callout::{make_callout, Callout, CalloutDirection, CalloutProps};
pub use circle::{make_circle, Circle, CircleProps};
pub use heart::{make_heart, Heart, HeartProps};
pub use pie::{make_pie, Pie, PieProps};
pub use polygon::{make_polygon, Polygon, PolygonProps};
pub use rect::{make_rect, Rect, RectProps};
pub use render_svg::{RenderSvg, RenderSvgProps};
pub use scene::SceneShape;
pub use shape_output::ShapeOutput;
pub use spark::{make_spark, Spark, SparkProps};
pub use star::{make_star, Star, StarProps};
pub use triangle::{make_triangle, Triangle, TriangleProps};
