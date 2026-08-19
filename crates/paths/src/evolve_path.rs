//! SVG path evolution for line drawing animations.

use crate::length::get_length;
use crate::types::EvolvedPath;

/// Animates an SVG path from invisible to full length.
///
/// Ported from Remotion's `evolvePath(progress, path)`.
///
/// Returns an [`EvolvedPath`] struct containing CSS `stroke_dasharray` and `stroke_dashoffset`.
pub fn evolve_path(progress: f64, path: &str) -> EvolvedPath {
    let length = get_length(path);
    evolve_path_with_length(progress, length)
}

/// Computes stroke-dash properties for a draw-on animation given pre-calculated total length.
///
/// # Arguments
/// * `progress` - Animation progress in `[0, 1]`. Values outside this range are clamped.
/// * `length` - Total length of the path in pixels.
pub fn evolve_path_with_length(progress: f64, length: f64) -> EvolvedPath {
    let clamped_p = progress.clamp(0.0, 1.0);

    if clamped_p == 0.0 {
        let extended_length = length * 1.5;
        return EvolvedPath {
            stroke_dasharray: format!("{extended_length:.4} {extended_length:.4}"),
            stroke_dashoffset: extended_length,
        };
    }

    let stroke_dasharray = format!("{length:.4} {length:.4}");
    let stroke_dashoffset = length - clamped_p * length;

    EvolvedPath {
        stroke_dasharray,
        stroke_dashoffset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evolve_path_stages() {
        let path = "M 0 0 L 100 0 Z"; // length 200
        let ev_zero = evolve_path(0.0, path);
        assert_eq!(ev_zero.stroke_dashoffset, 300.0);
        assert_eq!(ev_zero.stroke_dasharray, "300.0000 300.0000");

        let ev_half = evolve_path(0.5, path);
        assert_eq!(ev_half.stroke_dasharray, "200.0000 200.0000");
        assert_eq!(ev_half.stroke_dashoffset, 100.0);

        let ev_full = evolve_path(1.0, path);
        assert_eq!(ev_full.stroke_dasharray, "200.0000 200.0000");
        assert_eq!(ev_full.stroke_dashoffset, 0.0);
    }

    #[test]
    fn test_evolve_path_clamping() {
        let path = "M 0 0 L 100 0"; // length 100
        let ev_neg = evolve_path(-0.5, path);
        assert_eq!(ev_neg.stroke_dashoffset, 150.0);

        let ev_over = evolve_path(1.5, path);
        assert_eq!(ev_over.stroke_dashoffset, 0.0);
    }

    #[test]
    fn test_evolve_path_with_length() {
        let ep0 = evolve_path_with_length(0.0, 100.0);
        assert_eq!(ep0.stroke_dashoffset, 150.0);
        assert_eq!(ep0.stroke_dasharray, "150.0000 150.0000");

        let ep_half = evolve_path_with_length(0.5, 100.0);
        assert_eq!(ep_half.stroke_dashoffset, 50.0);
        assert_eq!(ep_half.stroke_dasharray, "100.0000 100.0000");

        let ep_full = evolve_path_with_length(1.0, 100.0);
        assert_eq!(ep_full.stroke_dashoffset, 0.0);
    }

    #[test]
    fn test_evolve_path_with_arc() {
        let path = "M 100 100 L 100 0 A 100 100 0 0 1 200 100 Z";
        let ev_half = evolve_path(0.5, path);
        assert!(ev_half.stroke_dashoffset > 0.0);
    }
}
