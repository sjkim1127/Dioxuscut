//! Bounding box computation for SVG paths.

use crate::parser::parse_path;
use crate::point_at_length::get_point_at_length;
use crate::types::{BoundingBox, Instruction, Point};

#[derive(Debug)]
struct BoundsTracker {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    has_points: bool,
}

impl BoundsTracker {
    fn new() -> Self {
        Self {
            min_x: f64::INFINITY,
            min_y: f64::INFINITY,
            max_x: f64::NEG_INFINITY,
            max_y: f64::NEG_INFINITY,
            has_points: false,
        }
    }

    fn update(&mut self, x: f64, y: f64) {
        if x.is_finite() && y.is_finite() {
            self.min_x = self.min_x.min(x);
            self.min_y = self.min_y.min(y);
            self.max_x = self.max_x.max(x);
            self.max_y = self.max_y.max(y);
            self.has_points = true;
        }
    }

    fn update_x(&mut self, x: f64) {
        if x.is_finite() {
            self.min_x = self.min_x.min(x);
            self.max_x = self.max_x.max(x);
            self.has_points = true;
        }
    }

    fn update_y(&mut self, y: f64) {
        if y.is_finite() {
            self.min_y = self.min_y.min(y);
            self.max_y = self.max_y.max(y);
            self.has_points = true;
        }
    }

    fn into_bounding_box(self) -> Option<BoundingBox> {
        if !self.has_points {
            None
        } else {
            Some(BoundingBox::new(
                self.min_x,
                self.min_y,
                (self.max_x - self.min_x).max(0.0),
                (self.max_y - self.min_y).max(0.0),
            ))
        }
    }
}

/// Calculates the axis-aligned bounding box of an SVG path string.
///
/// Returns `None` if the path contains no points or cannot be parsed.
pub fn get_bounding_box(path: &str) -> Option<BoundingBox> {
    let instructions = parse_path(path).ok()?;
    get_instructions_bounding_box(&instructions)
}

/// Calculates the bounding box of a slice of [`Instruction`]s.
pub fn get_instructions_bounding_box(instructions: &[Instruction]) -> Option<BoundingBox> {
    if instructions.is_empty() {
        return None;
    }

    let mut tracker = BoundsTracker::new();
    let mut curr = Point::new(0.0, 0.0);
    let mut start_point = Point::new(0.0, 0.0);

    for inst in instructions {
        match inst {
            Instruction::MoveTo { x, y } => {
                tracker.update(*x, *y);
                curr = Point::new(*x, *y);
                start_point = curr;
            }
            Instruction::LineTo { x, y } => {
                tracker.update(*x, *y);
                curr = Point::new(*x, *y);
            }
            Instruction::ClosePath => {
                curr = start_point;
            }
            Instruction::QuadCurveTo { x1, y1, x, y } => {
                tracker.update(curr.x, curr.y);
                tracker.update(*x, *y);

                // Quadratic extrema: B'(t) = 0
                // t = (P0 - P1) / (P0 - 2P1 + P2)
                for (p0, p1, p2, is_x) in [(curr.x, *x1, *x, true), (curr.y, *y1, *y, false)] {
                    let denom = p0 - 2.0 * p1 + p2;
                    if denom.abs() > 1e-9 {
                        let t = (p0 - p1) / denom;
                        if (0.0..=1.0).contains(&t) {
                            let val =
                                (1.0 - t) * (1.0 - t) * p0 + 2.0 * (1.0 - t) * t * p1 + t * t * p2;
                            if is_x {
                                tracker.update_x(val);
                            } else {
                                tracker.update_y(val);
                            }
                        }
                    }
                }

                curr = Point::new(*x, *y);
            }
            Instruction::CubicCurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                tracker.update(curr.x, curr.y);
                tracker.update(*x, *y);

                // Cubic extrema: A t^2 + B t + C = 0
                for (p0, p1, p2, p3, is_x) in
                    [(curr.x, *x1, *x2, *x, true), (curr.y, *y1, *y2, *y, false)]
                {
                    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
                    let b = 2.0 * (p0 - 2.0 * p1 + p2);
                    let c = p1 - p0;

                    let evaluate_t = |t: f64| -> f64 {
                        let omt = 1.0 - t;
                        omt * omt * omt * p0
                            + 3.0 * omt * omt * t * p1
                            + 3.0 * omt * t * t * p2
                            + t * t * t * p3
                    };

                    if a.abs() < 1e-9 {
                        if b.abs() > 1e-9 {
                            let t = -c / b;
                            if (0.0..=1.0).contains(&t) {
                                let val = evaluate_t(t);
                                if is_x {
                                    tracker.update_x(val);
                                } else {
                                    tracker.update_y(val);
                                }
                            }
                        }
                    } else {
                        let disc = b * b - 4.0 * a * c;
                        if disc >= 0.0 {
                            let sqrt_disc = disc.sqrt();
                            let t1 = (-b - sqrt_disc) / (2.0 * a);
                            let t2 = (-b + sqrt_disc) / (2.0 * a);
                            if (0.0..=1.0).contains(&t1) {
                                let val = evaluate_t(t1);
                                if is_x {
                                    tracker.update_x(val);
                                } else {
                                    tracker.update_y(val);
                                }
                            }
                            if (0.0..=1.0).contains(&t2) {
                                let val = evaluate_t(t2);
                                if is_x {
                                    tracker.update_x(val);
                                } else {
                                    tracker.update_y(val);
                                }
                            }
                        }
                    }
                }

                curr = Point::new(*x, *y);
            }
            Instruction::ArcTo { x, y, .. } => {
                tracker.update(curr.x, curr.y);
                tracker.update(*x, *y);
                // Sample points along arc for accurate bbox
                let single_arc_insts = vec![
                    Instruction::MoveTo {
                        x: curr.x,
                        y: curr.y,
                    },
                    inst.clone(),
                ];
                for step in 1..8 {
                    let progress = step as f64 / 8.0;
                    let pt = get_point_at_length(
                        &crate::parser::serialize_instructions(&single_arc_insts),
                        progress * crate::length::get_instructions_length(&single_arc_insts),
                    );
                    tracker.update(pt.x, pt.y);
                }
                curr = Point::new(*x, *y);
            }
        }
    }

    tracker.into_bounding_box()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_bounding_box() {
        let path = "M 10 20 L 110 20 L 110 70 L 10 70 Z";
        let bbox = get_bounding_box(path).expect("bbox should exist");
        assert!((bbox.x - 10.0).abs() < 1e-5);
        assert!((bbox.y - 20.0).abs() < 1e-5);
        assert!((bbox.width - 100.0).abs() < 1e-5);
        assert!((bbox.height - 50.0).abs() < 1e-5);
    }

    #[test]
    fn test_cubic_curve_bounding_box() {
        // Curve bulging out beyond endpoints
        let path = "M 0 0 C 50 100 150 100 200 0";
        let bbox = get_bounding_box(path).expect("bbox should exist");
        assert!((bbox.x - 0.0).abs() < 1e-5);
        assert!((bbox.width - 200.0).abs() < 1e-5);
        assert!(bbox.height > 50.0); // Curve bulges up towards 75
    }
}
