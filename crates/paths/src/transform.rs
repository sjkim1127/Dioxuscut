//! SVG path geometric transformations.

use crate::parser::{parse_path, serialize_instructions};
use crate::types::{Instruction, Point};

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
            Instruction::CubicCurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
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
            Instruction::ArcTo { x, y, .. } => {
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
            Instruction::CubicCurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
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
            Instruction::ArcTo { rx, ry, x, y, .. } => {
                *rx *= sx.abs();
                *ry *= sy.abs();
                *x *= sx;
                *y *= sy;
            }
            Instruction::ClosePath => {}
        }
    }

    serialize_instructions(&instructions)
}

/// Rotates an SVG path by `angle_rad` around center `(cx, cy)`.
pub fn rotate_path(path: &str, angle_rad: f64, cx: f64, cy: f64) -> String {
    let mut instructions = match parse_path(path) {
        Ok(insts) => insts,
        Err(_) => return path.to_string(),
    };
    let cos = angle_rad.cos();
    let sin = angle_rad.sin();
    let rotate_pt = |x: &mut f64, y: &mut f64| {
        let dx = *x - cx;
        let dy = *y - cy;
        *x = cx + dx * cos - dy * sin;
        *y = cy + dx * sin + dy * cos;
    };

    for inst in &mut instructions {
        match inst {
            Instruction::MoveTo { x, y } | Instruction::LineTo { x, y } => {
                rotate_pt(x, y);
            }
            Instruction::CubicCurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                rotate_pt(x1, y1);
                rotate_pt(x2, y2);
                rotate_pt(x, y);
            }
            Instruction::QuadCurveTo { x1, y1, x, y } => {
                rotate_pt(x1, y1);
                rotate_pt(x, y);
            }
            Instruction::ArcTo {
                x,
                y,
                x_axis_rotation,
                ..
            } => {
                rotate_pt(x, y);
                *x_axis_rotation = (*x_axis_rotation + angle_rad.to_degrees()).rem_euclid(360.0);
            }
            Instruction::ClosePath => {}
        }
    }

    serialize_instructions(&instructions)
}

/// Reverses the direction of an SVG path so drawing animation runs backwards.
pub fn reverse_path(path: &str) -> String {
    let instructions = match parse_path(path) {
        Ok(insts) => insts,
        Err(_) => return path.to_string(),
    };

    if instructions.is_empty() {
        return path.to_string();
    }

    // Split instructions into subpaths by MoveTo
    let mut subpaths: Vec<Vec<(Point, Instruction)>> = Vec::new();
    let mut current_subpath: Vec<(Point, Instruction)> = Vec::new();
    let mut current_pt = Point::new(0.0, 0.0);
    let mut start_pt = Point::new(0.0, 0.0);

    for inst in instructions {
        match inst {
            Instruction::MoveTo { x, y } => {
                if !current_subpath.is_empty() {
                    subpaths.push(current_subpath);
                    current_subpath = Vec::new();
                }
                current_pt = Point::new(x, y);
                start_pt = current_pt;
                current_subpath.push((current_pt, inst));
            }
            Instruction::LineTo { x, y } => {
                current_subpath.push((current_pt, inst));
                current_pt = Point::new(x, y);
            }
            Instruction::CubicCurveTo { x, y, .. } => {
                current_subpath.push((current_pt, inst));
                current_pt = Point::new(x, y);
            }
            Instruction::QuadCurveTo { x, y, .. } => {
                current_subpath.push((current_pt, inst));
                current_pt = Point::new(x, y);
            }
            Instruction::ArcTo { x, y, .. } => {
                current_subpath.push((current_pt, inst));
                current_pt = Point::new(x, y);
            }
            Instruction::ClosePath => {
                current_subpath.push((current_pt, inst));
                current_pt = start_pt;
            }
        }
    }
    if !current_subpath.is_empty() {
        subpaths.push(current_subpath);
    }

    let mut reversed_instructions = Vec::new();

    for subpath in subpaths {
        if subpath.is_empty() {
            continue;
        }

        let has_close = matches!(subpath.last(), Some((_, Instruction::ClosePath)));
        let end_idx = if has_close {
            subpath.len() - 1
        } else {
            subpath.len()
        };
        let segments = &subpath[..end_idx];

        if segments.is_empty() {
            continue;
        }

        // Determine the starting point of the reversed path
        let last_target = match segments.last().unwrap().1 {
            Instruction::MoveTo { x, y }
            | Instruction::LineTo { x, y }
            | Instruction::CubicCurveTo { x, y, .. }
            | Instruction::QuadCurveTo { x, y, .. }
            | Instruction::ArcTo { x, y, .. } => Point::new(x, y),
            Instruction::ClosePath => segments[0].0,
        };

        reversed_instructions.push(Instruction::MoveTo {
            x: last_target.x,
            y: last_target.y,
        });

        // Walk segments backwards
        for (prev_pt, inst) in segments.iter().skip(1).rev() {
            match *inst {
                Instruction::MoveTo { .. } => {}
                Instruction::LineTo { .. } => {
                    reversed_instructions.push(Instruction::LineTo {
                        x: prev_pt.x,
                        y: prev_pt.y,
                    });
                }
                Instruction::CubicCurveTo { x1, y1, x2, y2, .. } => {
                    reversed_instructions.push(Instruction::CubicCurveTo {
                        x1: x2,
                        y1: y2,
                        x2: x1,
                        y2: y1,
                        x: prev_pt.x,
                        y: prev_pt.y,
                    });
                }
                Instruction::QuadCurveTo { x1, y1, .. } => {
                    reversed_instructions.push(Instruction::QuadCurveTo {
                        x1,
                        y1,
                        x: prev_pt.x,
                        y: prev_pt.y,
                    });
                }
                Instruction::ArcTo {
                    rx,
                    ry,
                    x_axis_rotation,
                    large_arc_flag,
                    sweep_flag,
                    ..
                } => {
                    reversed_instructions.push(Instruction::ArcTo {
                        rx,
                        ry,
                        x_axis_rotation,
                        large_arc_flag,
                        sweep_flag: !sweep_flag,
                        x: prev_pt.x,
                        y: prev_pt.y,
                    });
                }
                Instruction::ClosePath => {}
            }
        }

        if has_close {
            reversed_instructions.push(Instruction::ClosePath);
        }
    }

    serialize_instructions(&reversed_instructions)
}

/// Resets a path so its top-left bounding box corner is at `(0, 0)`.
pub fn reset_path(path: &str) -> String {
    if let Some(bbox) = crate::bounding_box::get_bounding_box(path) {
        translate_path(path, -bbox.x, -bbox.y)
    } else {
        path.to_string()
    }
}

/// Warps an SVG path by transforming each coordinate with custom mapping `f`.
pub fn warp_path<F>(path: &str, mut f: F) -> String
where
    F: FnMut(f64, f64) -> (f64, f64),
{
    let mut instructions = match parse_path(path) {
        Ok(insts) => insts,
        Err(_) => return path.to_string(),
    };

    for inst in &mut instructions {
        match inst {
            Instruction::MoveTo { x, y } | Instruction::LineTo { x, y } => {
                let (nx, ny) = f(*x, *y);
                *x = nx;
                *y = ny;
            }
            Instruction::CubicCurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let (nx1, ny1) = f(*x1, *y1);
                let (nx2, ny2) = f(*x2, *y2);
                let (nx, ny) = f(*x, *y);
                *x1 = nx1;
                *y1 = ny1;
                *x2 = nx2;
                *y2 = ny2;
                *x = nx;
                *y = ny;
            }
            Instruction::QuadCurveTo { x1, y1, x, y } => {
                let (nx1, ny1) = f(*x1, *y1);
                let (nx, ny) = f(*x, *y);
                *x1 = nx1;
                *y1 = ny1;
                *x = nx;
                *y = ny;
            }
            Instruction::ArcTo { x, y, .. } => {
                let (nx, ny) = f(*x, *y);
                *x = nx;
                *y = ny;
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

    #[test]
    fn test_rotate_path() {
        let path = "M 10 0 L 20 0";
        // 90 degrees rotation around (0, 0)
        let rotated = rotate_path(path, std::f64::consts::FRAC_PI_2, 0.0, 0.0);
        let insts = parse_path(&rotated).unwrap();
        match &insts[0] {
            Instruction::MoveTo { x, y } => {
                assert!(x.abs() < 1e-4);
                assert!((y - 10.0).abs() < 1e-4);
            }
            _ => panic!("expected MoveTo"),
        }
    }

    #[test]
    fn test_reverse_path() {
        let path = "M 0 0 L 10 20 L 30 40";
        let rev = reverse_path(path);
        let insts = parse_path(&rev).unwrap();
        assert_eq!(insts[0], Instruction::MoveTo { x: 30.0, y: 40.0 });
        assert_eq!(insts[1], Instruction::LineTo { x: 10.0, y: 20.0 });
        assert_eq!(insts[2], Instruction::LineTo { x: 0.0, y: 0.0 });
    }

    #[test]
    fn test_reset_path() {
        let path = "M 15 25 L 35 45";
        let reset = reset_path(path);
        let insts = parse_path(&reset).unwrap();
        assert_eq!(insts[0], Instruction::MoveTo { x: 0.0, y: 0.0 });
        assert_eq!(insts[1], Instruction::LineTo { x: 20.0, y: 20.0 });
    }

    #[test]
    fn test_warp_path() {
        let path = "M 10 10 L 20 20";
        let warped = warp_path(path, |x, y| (x * 2.0, y + 5.0));
        let insts = parse_path(&warped).unwrap();
        assert_eq!(insts[0], Instruction::MoveTo { x: 20.0, y: 15.0 });
        assert_eq!(insts[1], Instruction::LineTo { x: 40.0, y: 25.0 });
    }

    #[test]
    fn test_translate_and_scale_arc() {
        let path = "M 0 0 A 10 20 0 0 1 10 20";
        let translated = translate_path(path, 5.0, 5.0);
        assert_eq!(
            translated,
            "M 5.0000 5.0000 A 10.0000 20.0000 0.0000 0 1 15.0000 25.0000"
        );

        let scaled = scale_path(path, 2.0, 3.0);
        assert_eq!(
            scaled,
            "M 0.0000 0.0000 A 20.0000 60.0000 0.0000 0 1 20.0000 60.0000"
        );
    }

    #[test]
    fn test_transform_invalid_path_returns_original() {
        let invalid = "not a valid path";
        assert_eq!(translate_path(invalid, 10.0, 10.0), invalid);
        assert_eq!(scale_path(invalid, 2.0, 2.0), invalid);
    }
}
