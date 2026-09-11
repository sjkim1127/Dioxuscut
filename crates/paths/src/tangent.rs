//! SVG path tangent vector computation.

use crate::length::get_length;
use crate::point_at_length::get_point_at_length;
use crate::types::Point;

/// Returns the normalized 2D unit tangent vector `(dx, dy)` at a given arc `length` along the path.
///
/// Returns `None` if the path cannot be parsed or has zero length.
pub fn get_tangent_at_length(path: &str, length: f64) -> Option<Point> {
    let total_len = get_length(path);
    if total_len <= 1e-9 || !length.is_finite() {
        return None;
    }

    let clamped_len = length.clamp(0.0, total_len);
    let eps = (total_len * 1e-4).clamp(1e-5, 0.01);

    let (s1, s2) = if clamped_len <= eps {
        (0.0, 2.0 * eps.min(total_len))
    } else if clamped_len >= total_len - eps {
        ((total_len - 2.0 * eps).max(0.0), total_len)
    } else {
        (clamped_len - eps, clamped_len + eps)
    };

    let p1 = get_point_at_length(path, s1);
    let p2 = get_point_at_length(path, s2);

    let dx = p2.x - p1.x;
    let dy = p2.y - p1.y;
    let norm = (dx * dx + dy * dy).sqrt();

    if norm < 1e-9 {
        None
    } else {
        Some(Point::new(dx / norm, dy / norm))
    }
}

/// Returns the angle in radians of the tangent vector at `length` along the path.
pub fn get_tangent_angle_at_length(path: &str, length: f64) -> Option<f64> {
    let tangent = get_tangent_at_length(path, length)?;
    Some(tangent.y.atan2(tangent.x))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_horizontal_line_tangent() {
        let path = "M 0 0 L 100 0";
        let tangent = get_tangent_at_length(path, 50.0).unwrap();
        assert!((tangent.x - 1.0).abs() < 1e-4);
        assert!(tangent.y.abs() < 1e-4);
        let angle = get_tangent_angle_at_length(path, 50.0).unwrap();
        assert!(angle.abs() < 1e-4);
    }

    #[test]
    fn test_vertical_line_tangent() {
        let path = "M 0 0 L 0 100";
        let tangent = get_tangent_at_length(path, 50.0).unwrap();
        assert!(tangent.x.abs() < 1e-4);
        assert!((tangent.y - 1.0).abs() < 1e-4);
        let angle = get_tangent_angle_at_length(path, 50.0).unwrap();
        assert!((angle - std::f64::consts::FRAC_PI_2).abs() < 1e-4);
    }
}
