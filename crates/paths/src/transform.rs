//! SVG path geometric transformations.

use crate::parser::{parse_path, serialize_instructions};
use crate::types::Instruction;

/// Translates (offsets) an SVG path string by `(dx, dy)`.
pub fn translate_path(path: &str, dx: f64, dy: f64) -> String {
    let mut instructions = match parse_path(path) {
        Ok(insts) => insts,
        Err(_) => return path.to_string(),
    };

    for inst in &mut instructions {
        match inst {
            Instruction::MoveTo { x, y } => {
                *x += dx;
                *y += dy;
            }
            Instruction::LineTo { x, y } => {
                *x += dx;
                *y += dy;
            }
            Instruction::CubicCurveTo { x1, y1, x2, y2, x, y } => {
                *x1 += dx;
                *y1 += dy;
                *x2 += dx;
                *y2 += dy;
                *x += dx;
                *y += dy;
            }
            Instruction::QuadCurveTo { x1, y1, x, y } => {
                *x1 += dx;
                *y1 += dy;
                *x += dx;
                *y += dy;
            }
            Instruction::ClosePath => {}
        }
    }

    serialize_instructions(&instructions)
}

/// Scales an SVG path string by factors `(sx, sy)`.
pub fn scale_path(path: &str, sx: f64, sy: f64) -> String {
    let mut instructions = match parse_path(path) {
        Ok(insts) => insts,
        Err(_) => return path.to_string(),
    };

    for inst in &mut instructions {
        match inst {
            Instruction::MoveTo { x, y } => {
                *x *= sx;
                *y *= sy;
            }
            Instruction::LineTo { x, y } => {
                *x *= sx;
                *y *= sy;
            }
            Instruction::CubicCurveTo { x1, y1, x2, y2, x, y } => {
                *x1 *= sx;
                *y1 *= sy;
                *x2 *= sx;
                *y2 *= sy;
                *x *= sx;
                *y *= sy;
            }
            Instruction::QuadCurveTo { x1, y1, x, y } => {
                *x1 *= sx;
                *y1 *= sy;
                *x *= sx;
                *y *= sy;
            }
            Instruction::ClosePath => {}
        }
    }

    serialize_instructions(&instructions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_translate_path() {
        let path = "M 0 0 L 10 10 Z";
        let res = translate_path(path, 5.0, 15.0);
        assert_eq!(res, "M 5.0000 15.0000 L 15.0000 25.0000 Z");
    }

    #[test]
    fn test_scale_path() {
        let path = "M 10 20 L 30 40 Z";
        let res = scale_path(path, 2.0, 0.5);
        assert_eq!(res, "M 20.0000 10.0000 L 60.0000 20.0000 Z");
    }
}
