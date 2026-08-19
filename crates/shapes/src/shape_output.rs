//! Shape output container for procedural SVG shape generators.

use serde::{Deserialize, Serialize};

/// Standard output structure returned by procedural shape generators.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeOutput {
    /// SVG path `d` instruction string.
    pub path: String,
    /// Bounding box width in pixels.
    pub width: f64,
    /// Bounding box height in pixels.
    pub height: f64,
    /// Recommended transform origin (e.g. `"50 50"` or `"100 100"`).
    pub transform_origin: String,
}

impl ShapeOutput {
    /// Creates a new `ShapeOutput`.
    pub fn new(
        path: impl Into<String>,
        width: f64,
        height: f64,
        transform_origin: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            width,
            height,
            transform_origin: transform_origin.into(),
        }
    }
}

impl From<ShapeOutput> for (String, f64, f64) {
    fn from(shape: ShapeOutput) -> Self {
        (shape.path, shape.width, shape.height)
    }
}

impl From<&ShapeOutput> for (String, f64, f64) {
    fn from(shape: &ShapeOutput) -> Self {
        (shape.path.clone(), shape.width, shape.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shape_output_new_and_from_tuple() {
        let shape = ShapeOutput::new("M 0 0 L 10 10 Z", 10.0, 10.0, "5 5");
        assert_eq!(shape.path, "M 0 0 L 10 10 Z");
        assert_eq!(shape.width, 10.0);
        assert_eq!(shape.height, 10.0);
        assert_eq!(shape.transform_origin, "5 5");

        let tuple: (String, f64, f64) = shape.clone().into();
        assert_eq!(tuple.0, "M 0 0 L 10 10 Z");
        assert_eq!(tuple.1, 10.0);
        assert_eq!(tuple.2, 10.0);

        let tuple_ref: (String, f64, f64) = (&shape).into();
        assert_eq!(tuple_ref.0, "M 0 0 L 10 10 Z");
        assert_eq!(tuple_ref.1, 10.0);
        assert_eq!(tuple_ref.2, 10.0);
    }

    #[test]
    fn test_shape_output_serde() {
        let shape = ShapeOutput::new("M 0 0 L 20 20 Z", 20.0, 20.0, "10 10");
        let serialized = serde_json::to_string(&shape).unwrap();
        let deserialized: ShapeOutput = serde_json::from_str(&serialized).unwrap();
        assert_eq!(shape, deserialized);
    }
}
