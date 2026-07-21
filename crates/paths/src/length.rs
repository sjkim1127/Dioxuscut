//! SVG path length calculation.

use crate::parser::parse_path;
use crate::types::Instruction;

/// Calculates total length of an SVG path string in pixels.
pub fn get_length(path: &str) -> f64 {
    let instructions = match parse_path(path) {
        Ok(insts) => insts,
        Err(_) => return 0.0,
    };

    get_instructions_length(&instructions)
}

/// Calculates total length of a list of [`Instruction`]s.
pub fn get_instructions_length(instructions: &[Instruction]) -> f64 {
    let mut total_length = 0.0;
    let mut current_x = 0.0;
    let mut current_y = 0.0;
    let mut start_x = 0.0;
    let mut start_y = 0.0;

    for inst in instructions {
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
                total_length += (dx * dx + dy * dy).sqrt();
                current_x = *x;
                current_y = *y;
            }
            Instruction::ClosePath => {
                let dx = start_x - current_x;
                let dy = start_y - current_y;
                total_length += (dx * dx + dy * dy).sqrt();
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
                total_length +=
                    cubic_bezier_length(current_x, current_y, *x1, *y1, *x2, *y2, *x, *y);
                current_x = *x;
                current_y = *y;
            }
            Instruction::QuadCurveTo { x1, y1, x, y } => {
                total_length += quad_bezier_length(current_x, current_y, *x1, *y1, *x, *y);
                current_x = *x;
                current_y = *y;
            }
        }
    }

    total_length
}

fn cubic_bezier_length(
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    x3: f64,
    y3: f64,
) -> f64 {
    let steps = 16;
    let mut length = 0.0;
    let mut prev_x = x0;
    let mut prev_y = y0;

    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let mt = 1.0 - t;

        let px =
            mt * mt * mt * x0 + 3.0 * mt * mt * t * x1 + 3.0 * mt * t * t * x2 + t * t * t * x3;

        let py =
            mt * mt * mt * y0 + 3.0 * mt * mt * t * y1 + 3.0 * mt * t * t * y2 + t * t * t * y3;

        let dx = px - prev_x;
        let dy = py - prev_y;
        length += (dx * dx + dy * dy).sqrt();

        prev_x = px;
        prev_y = py;
    }

    length
}

fn quad_bezier_length(x0: f64, y0: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let steps = 16;
    let mut length = 0.0;
    let mut prev_x = x0;
    let mut prev_y = y0;

    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        let mt = 1.0 - t;

        let px = mt * mt * x0 + 2.0 * mt * t * x1 + t * t * x2;
        let py = mt * mt * y0 + 2.0 * mt * t * y1 + t * t * y2;

        let dx = px - prev_x;
        let dy = py - prev_y;
        length += (dx * dx + dy * dy).sqrt();

        prev_x = px;
        prev_y = py;
    }

    length
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_path_length() {
        let d = "M 0 0 L 100 0 L 100 100 L 0 100 Z";
        let len = get_length(d);
        assert_eq!(len, 400.0);
    }
}
