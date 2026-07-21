//! SVG path types and data structures.

use serde::{Deserialize, Serialize};

/// 2D point coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Represents an individual SVG path command instruction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Instruction {
    /// `M x y` or `m x y`
    MoveTo { x: f64, y: f64 },
    /// `L x y` or `l x y`
    LineTo { x: f64, y: f64 },
    /// `C x1 y1 x2 y2 x y` or `c x1 y1 x2 y2 x y` (Cubic Bezier)
    CubicCurveTo {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        x: f64,
        y: f64,
    },
    /// `Q x1 y1 x y` or `q x1 y1 x y` (Quadratic Bezier)
    QuadCurveTo { x1: f64, y1: f64, x: f64, y: f64 },
    /// `Z` or `z`
    ClosePath,
}

/// Calculated SVG stroke properties for line drawing animation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvolvedPath {
    /// `stroke-dasharray` CSS property value.
    pub stroke_dasharray: String,
    /// `stroke-dashoffset` CSS property value.
    pub stroke_dashoffset: f64,
}
