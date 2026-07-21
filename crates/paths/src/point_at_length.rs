//! Point calculation at specific distance along an SVG path.

use crate::parser::parse_path;
use crate::types::{Instruction, Point};

/// Returns the `(x, y)` [`Point`] at a specific `distance` in pixels along an SVG path.
pub fn get_point_at_length(path: &str, distance: f64) -> Point {
    let instructions = match parse_path(path) {
        Ok(insts) => insts,
        Err(_) => return Point::new(0.0, 0.0),
    };

    let target_dist = distance.max(0.0);
    let mut accumulated_dist = 0.0;

    let mut current_x = 0.0;
    let mut current_y = 0.0;
    let mut start_x = 0.0;
    let mut start_y = 0.0;

    for inst in &instructions {
        match inst {
            Instruction::MoveTo { x, y } => {
                current_x = *x;
                current_y = *y;
                start_x = *x;
                start_y = *y;
            }
            Instruction::LineTo { x, y } => {
                let dx = x - current_x;
                let dy = y - current_y;
                let seg_len = (dx * dx + dy * dy).sqrt();

                if accumulated_dist + seg_len >= target_dist {
                    let remaining = target_dist - accumulated_dist;
                    let ratio = if seg_len > 0.0 { remaining / seg_len } else { 0.0 };
                    return Point::new(current_x + dx * ratio, current_y + dy * ratio);
                }

                accumulated_dist += seg_len;
                current_x = *x;
                current_y = *y;
            }
            Instruction::ClosePath => {
                let dx = start_x - current_x;
                let dy = start_y - current_y;
                let seg_len = (dx * dx + dy * dy).sqrt();

                if accumulated_dist + seg_len >= target_dist {
                    let remaining = target_dist - accumulated_dist;
                    let ratio = if seg_len > 0.0 { remaining / seg_len } else { 0.0 };
                    return Point::new(current_x + dx * ratio, current_y + dy * ratio);
                }

                accumulated_dist += seg_len;
                current_x = start_x;
                current_y = start_y;
            }
            Instruction::CubicCurveTo { x1, y1, x2, y2, x, y } => {
                let steps = 16;
                let mut prev_x = current_x;
                let mut prev_y = current_y;

                for i in 1..=steps {
                    let t = i as f64 / steps as f64;
                    let mt = 1.0 - t;

                    let px = mt * mt * mt * current_x
                        + 3.0 * mt * mt * t * x1
                        + 3.0 * mt * t * t * x2
                        + t * t * t * x;

                    let py = mt * mt * mt * current_y
                        + 3.0 * mt * mt * t * y1
                        + 3.0 * mt * t * t * y2
                        + t * t * t * y;

                    let dx = px - prev_x;
                    let dy = py - prev_y;
                    let step_len = (dx * dx + dy * dy).sqrt();

                    if accumulated_dist + step_len >= target_dist {
                        return Point::new(px, py);
                    }

                    accumulated_dist += step_len;
                    prev_x = px;
                    prev_y = py;
                }

                current_x = *x;
                current_y = *y;
            }
            Instruction::QuadCurveTo { x1, y1, x, y } => {
                let steps = 16;
                let mut prev_x = current_x;
                let mut prev_y = current_y;

                for i in 1..=steps {
                    let t = i as f64 / steps as f64;
                    let mt = 1.0 - t;

                    let px = mt * mt * current_x + 2.0 * mt * t * x1 + t * t * x;
                    let py = mt * mt * current_y + 2.0 * mt * t * y1 + t * t * y;

                    let dx = px - prev_x;
                    let dy = py - prev_y;
                    let step_len = (dx * dx + dy * dy).sqrt();

                    if accumulated_dist + step_len >= target_dist {
                        return Point::new(px, py);
                    }

                    accumulated_dist += step_len;
                    prev_x = px;
                    prev_y = py;
                }

                current_x = *x;
                current_y = *y;
            }
        }
    }

    Point::new(current_x, current_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_point_at_length_line() {
        let path = "M 0 0 L 100 0";
        let pt_mid = get_point_at_length(path, 50.0);
        assert_eq!(pt_mid, Point::new(50.0, 0.0));

        let pt_end = get_point_at_length(path, 100.0);
        assert_eq!(pt_end, Point::new(100.0, 0.0));
    }
}
