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
                    let ratio = if seg_len > 0.0 {
                        remaining / seg_len
                    } else {
                        0.0
                    };
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
                    let ratio = if seg_len > 0.0 {
                        remaining / seg_len
                    } else {
                        0.0
                    };
                    return Point::new(current_x + dx * ratio, current_y + dy * ratio);
                }

                accumulated_dist += seg_len;
                current_x = start_x;
                current_y = start_y;
            }
            Instruction::CubicCurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
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
            Instruction::ArcTo {
                rx,
                ry,
                x_axis_rotation,
                large_arc_flag,
                sweep_flag,
                x,
                y,
            } => {
                let (x1, y1) = (current_x, current_y);
                let (x2, y2) = (*x, *y);

                let mut r_x = rx.abs();
                let mut r_y = ry.abs();

                if (x1 - x2).abs() < 1e-9 && (y1 - y2).abs() < 1e-9 {
                    current_x = *x;
                    current_y = *y;
                    continue;
                }

                if r_x < 1e-9 || r_y < 1e-9 {
                    let dx = x2 - x1;
                    let dy = y2 - y1;
                    let seg_len = (dx * dx + dy * dy).sqrt();
                    if accumulated_dist + seg_len >= target_dist {
                        let remaining = target_dist - accumulated_dist;
                        let ratio = if seg_len > 0.0 {
                            remaining / seg_len
                        } else {
                            0.0
                        };
                        return Point::new(x1 + dx * ratio, y1 + dy * ratio);
                    }
                    accumulated_dist += seg_len;
                    current_x = *x;
                    current_y = *y;
                    continue;
                }

                let phi = x_axis_rotation.to_radians();
                let cos_phi = phi.cos();
                let sin_phi = phi.sin();

                let dx = (x1 - x2) / 2.0;
                let dy = (y1 - y2) / 2.0;
                let x1_prime = cos_phi * dx + sin_phi * dy;
                let y1_prime = -sin_phi * dx + cos_phi * dy;

                let lambda =
                    (x1_prime * x1_prime) / (r_x * r_x) + (y1_prime * y1_prime) / (r_y * r_y);
                if lambda > 1.0 {
                    let sqrt_lambda = lambda.sqrt();
                    r_x *= sqrt_lambda;
                    r_y *= sqrt_lambda;
                }

                let rx_sq = r_x * r_x;
                let ry_sq = r_y * r_y;
                let x1_p_sq = x1_prime * x1_prime;
                let y1_p_sq = y1_prime * y1_prime;

                let num = (rx_sq * ry_sq - rx_sq * y1_p_sq - ry_sq * x1_p_sq).max(0.0);
                let den = rx_sq * y1_p_sq + ry_sq * x1_p_sq;
                let sq = if den < 1e-9 { 0.0 } else { num / den };
                let mut s = sq.sqrt();
                if large_arc_flag == sweep_flag {
                    s = -s;
                }
                let cx_prime = s * (r_x * y1_prime / r_y);
                let cy_prime = s * (-r_y * x1_prime / r_x);

                let cx = cos_phi * cx_prime - sin_phi * cy_prime + (x1 + x2) / 2.0;
                let cy = sin_phi * cx_prime + cos_phi * cy_prime + (y1 + y2) / 2.0;

                let v1 = ((x1_prime - cx_prime) / r_x, (y1_prime - cy_prime) / r_y);
                let v2 = ((-x1_prime - cx_prime) / r_x, (-y1_prime - cy_prime) / r_y);

                let theta1 = crate::length::vector_angle((1.0, 0.0), v1);
                let mut delta_theta = crate::length::vector_angle(v1, v2);

                let two_pi = std::f64::consts::TAU;
                if !sweep_flag && delta_theta > 0.0 {
                    delta_theta -= two_pi;
                } else if *sweep_flag && delta_theta < 0.0 {
                    delta_theta += two_pi;
                }

                let num_steps = ((delta_theta.abs() / (std::f64::consts::PI / 64.0)).ceil()
                    as usize)
                    .clamp(32, 256);
                let mut prev_x = x1;
                let mut prev_y = y1;

                for i in 1..=num_steps {
                    let t = i as f64 / num_steps as f64;
                    let theta = theta1 + t * delta_theta;
                    let cos_theta = theta.cos();
                    let sin_theta = theta.sin();

                    let px = cx + r_x * cos_theta * cos_phi - r_y * sin_theta * sin_phi;
                    let py = cy + r_x * cos_theta * sin_phi + r_y * sin_theta * cos_phi;

                    let seg_dx = px - prev_x;
                    let seg_dy = py - prev_y;
                    let step_len = (seg_dx * seg_dx + seg_dy * seg_dy).sqrt();

                    if accumulated_dist + step_len >= target_dist {
                        let remaining = target_dist - accumulated_dist;
                        let ratio = if step_len > 0.0 {
                            remaining / step_len
                        } else {
                            0.0
                        };
                        return Point::new(prev_x + seg_dx * ratio, prev_y + seg_dy * ratio);
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

    #[test]
    fn test_point_at_length_arc() {
        // Quarter circle from (100, 0) to (0, 100)
        let path = "M 100 0 A 100 100 0 0 1 0 100";
        let pt_start = get_point_at_length(path, 0.0);
        assert!((pt_start.x - 100.0).abs() < 0.1);
        assert!((pt_start.y - 0.0).abs() < 0.1);

        let pt_end = get_point_at_length(path, 200.0);
        assert!((pt_end.x - 0.0).abs() < 0.5);
        assert!((pt_end.y - 100.0).abs() < 0.5);
    }

    #[test]
    fn test_point_at_length_clamp_negative() {
        let path = "M 10 20 L 100 20";
        let pt = get_point_at_length(path, -50.0);
        assert_eq!(pt, Point::new(10.0, 20.0));
    }

    #[test]
    fn test_point_at_length_invalid() {
        let pt = get_point_at_length("invalid path", 10.0);
        assert_eq!(pt, Point::new(0.0, 0.0));
    }
}
