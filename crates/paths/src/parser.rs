//! SVG path parser and serializer.

use crate::types::Instruction;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum PathParseError {
    #[error("Invalid SVG command '{0}' at index {1}")]
    InvalidCommand(char, usize),
    #[error("Unexpected end of path string")]
    UnexpectedEnd,
    #[error("Failed to parse number '{0}'")]
    ParseNumberError(String),
}

/// Parses an SVG path `d` string into a list of [`Instruction`]s.
pub fn parse_path(d: &str) -> Result<Vec<Instruction>, PathParseError> {
    let mut instructions = Vec::new();
    let tokens = tokenize(d)?;
    let mut idx = 0;

    let mut current_x = 0.0;
    let mut current_y = 0.0;
    let mut start_x = 0.0;
    let mut start_y = 0.0;
    let mut last_control_point: Option<(f64, f64)> = None;
    let mut last_cmd: Option<char> = None;

    while idx < tokens.len() {
        match tokens[idx].as_str() {
            "M" => {
                idx += 1;
                let x = parse_num(&tokens, &mut idx)?;
                let y = parse_num(&tokens, &mut idx)?;
                current_x = x;
                current_y = y;
                start_x = x;
                start_y = y;
                instructions.push(Instruction::MoveTo { x, y });
                last_cmd = None;
                last_control_point = None;

                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let x = parse_num(&tokens, &mut idx)?;
                    let y = parse_num(&tokens, &mut idx)?;
                    current_x = x;
                    current_y = y;
                    instructions.push(Instruction::LineTo { x, y });
                }
            }
            "m" => {
                idx += 1;
                let dx = parse_num(&tokens, &mut idx)?;
                let dy = parse_num(&tokens, &mut idx)?;
                current_x += dx;
                current_y += dy;
                start_x = current_x;
                start_y = current_y;
                instructions.push(Instruction::MoveTo {
                    x: current_x,
                    y: current_y,
                });
                last_cmd = None;
                last_control_point = None;

                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let dx = parse_num(&tokens, &mut idx)?;
                    let dy = parse_num(&tokens, &mut idx)?;
                    current_x += dx;
                    current_y += dy;
                    instructions.push(Instruction::LineTo {
                        x: current_x,
                        y: current_y,
                    });
                }
            }
            "L" => {
                idx += 1;
                last_cmd = None;
                last_control_point = None;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let x = parse_num(&tokens, &mut idx)?;
                    let y = parse_num(&tokens, &mut idx)?;
                    current_x = x;
                    current_y = y;
                    instructions.push(Instruction::LineTo { x, y });
                }
            }
            "l" => {
                idx += 1;
                last_cmd = None;
                last_control_point = None;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let dx = parse_num(&tokens, &mut idx)?;
                    let dy = parse_num(&tokens, &mut idx)?;
                    current_x += dx;
                    current_y += dy;
                    instructions.push(Instruction::LineTo {
                        x: current_x,
                        y: current_y,
                    });
                }
            }
            "H" => {
                idx += 1;
                last_cmd = None;
                last_control_point = None;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let x = parse_num(&tokens, &mut idx)?;
                    current_x = x;
                    instructions.push(Instruction::LineTo {
                        x: current_x,
                        y: current_y,
                    });
                }
            }
            "h" => {
                idx += 1;
                last_cmd = None;
                last_control_point = None;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let dx = parse_num(&tokens, &mut idx)?;
                    current_x += dx;
                    instructions.push(Instruction::LineTo {
                        x: current_x,
                        y: current_y,
                    });
                }
            }
            "V" => {
                idx += 1;
                last_cmd = None;
                last_control_point = None;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let y = parse_num(&tokens, &mut idx)?;
                    current_y = y;
                    instructions.push(Instruction::LineTo {
                        x: current_x,
                        y: current_y,
                    });
                }
            }
            "v" => {
                idx += 1;
                last_cmd = None;
                last_control_point = None;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let dy = parse_num(&tokens, &mut idx)?;
                    current_y += dy;
                    instructions.push(Instruction::LineTo {
                        x: current_x,
                        y: current_y,
                    });
                }
            }
            "C" => {
                idx += 1;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let x1 = parse_num(&tokens, &mut idx)?;
                    let y1 = parse_num(&tokens, &mut idx)?;
                    let x2 = parse_num(&tokens, &mut idx)?;
                    let y2 = parse_num(&tokens, &mut idx)?;
                    let x = parse_num(&tokens, &mut idx)?;
                    let y = parse_num(&tokens, &mut idx)?;
                    current_x = x;
                    current_y = y;
                    last_control_point = Some((x2, y2));
                    last_cmd = Some('C');
                    instructions.push(Instruction::CubicCurveTo {
                        x1,
                        y1,
                        x2,
                        y2,
                        x,
                        y,
                    });
                }
            }
            "c" => {
                idx += 1;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let dx1 = parse_num(&tokens, &mut idx)?;
                    let dy1 = parse_num(&tokens, &mut idx)?;
                    let dx2 = parse_num(&tokens, &mut idx)?;
                    let dy2 = parse_num(&tokens, &mut idx)?;
                    let dx = parse_num(&tokens, &mut idx)?;
                    let dy = parse_num(&tokens, &mut idx)?;
                    let x1 = current_x + dx1;
                    let y1 = current_y + dy1;
                    let x2 = current_x + dx2;
                    let y2 = current_y + dy2;
                    let x = current_x + dx;
                    let y = current_y + dy;
                    current_x = x;
                    current_y = y;
                    last_control_point = Some((x2, y2));
                    last_cmd = Some('C');
                    instructions.push(Instruction::CubicCurveTo {
                        x1,
                        y1,
                        x2,
                        y2,
                        x,
                        y,
                    });
                }
            }
            "S" => {
                idx += 1;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let x2 = parse_num(&tokens, &mut idx)?;
                    let y2 = parse_num(&tokens, &mut idx)?;
                    let x = parse_num(&tokens, &mut idx)?;
                    let y = parse_num(&tokens, &mut idx)?;
                    let (x1, y1) = match (last_cmd, last_control_point) {
                        (Some('C'), Some((px2, py2))) => {
                            (2.0 * current_x - px2, 2.0 * current_y - py2)
                        }
                        _ => (current_x, current_y),
                    };
                    current_x = x;
                    current_y = y;
                    last_control_point = Some((x2, y2));
                    last_cmd = Some('C');
                    instructions.push(Instruction::CubicCurveTo {
                        x1,
                        y1,
                        x2,
                        y2,
                        x,
                        y,
                    });
                }
            }
            "s" => {
                idx += 1;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let dx2 = parse_num(&tokens, &mut idx)?;
                    let dy2 = parse_num(&tokens, &mut idx)?;
                    let dx = parse_num(&tokens, &mut idx)?;
                    let dy = parse_num(&tokens, &mut idx)?;
                    let (x1, y1) = match (last_cmd, last_control_point) {
                        (Some('C'), Some((px2, py2))) => {
                            (2.0 * current_x - px2, 2.0 * current_y - py2)
                        }
                        _ => (current_x, current_y),
                    };
                    let x2 = current_x + dx2;
                    let y2 = current_y + dy2;
                    let x = current_x + dx;
                    let y = current_y + dy;
                    current_x = x;
                    current_y = y;
                    last_control_point = Some((x2, y2));
                    last_cmd = Some('C');
                    instructions.push(Instruction::CubicCurveTo {
                        x1,
                        y1,
                        x2,
                        y2,
                        x,
                        y,
                    });
                }
            }
            "Q" => {
                idx += 1;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let x1 = parse_num(&tokens, &mut idx)?;
                    let y1 = parse_num(&tokens, &mut idx)?;
                    let x = parse_num(&tokens, &mut idx)?;
                    let y = parse_num(&tokens, &mut idx)?;
                    current_x = x;
                    current_y = y;
                    last_control_point = Some((x1, y1));
                    last_cmd = Some('Q');
                    instructions.push(Instruction::QuadCurveTo { x1, y1, x, y });
                }
            }
            "q" => {
                idx += 1;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let dx1 = parse_num(&tokens, &mut idx)?;
                    let dy1 = parse_num(&tokens, &mut idx)?;
                    let dx = parse_num(&tokens, &mut idx)?;
                    let dy = parse_num(&tokens, &mut idx)?;
                    let x1 = current_x + dx1;
                    let y1 = current_y + dy1;
                    let x = current_x + dx;
                    let y = current_y + dy;
                    current_x = x;
                    current_y = y;
                    last_control_point = Some((x1, y1));
                    last_cmd = Some('Q');
                    instructions.push(Instruction::QuadCurveTo { x1, y1, x, y });
                }
            }
            "T" => {
                idx += 1;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let x = parse_num(&tokens, &mut idx)?;
                    let y = parse_num(&tokens, &mut idx)?;
                    let (x1, y1) = match (last_cmd, last_control_point) {
                        (Some('Q'), Some((px1, py1))) => {
                            (2.0 * current_x - px1, 2.0 * current_y - py1)
                        }
                        _ => (current_x, current_y),
                    };
                    current_x = x;
                    current_y = y;
                    last_control_point = Some((x1, y1));
                    last_cmd = Some('Q');
                    instructions.push(Instruction::QuadCurveTo { x1, y1, x, y });
                }
            }
            "t" => {
                idx += 1;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let dx = parse_num(&tokens, &mut idx)?;
                    let dy = parse_num(&tokens, &mut idx)?;
                    let (x1, y1) = match (last_cmd, last_control_point) {
                        (Some('Q'), Some((px1, py1))) => {
                            (2.0 * current_x - px1, 2.0 * current_y - py1)
                        }
                        _ => (current_x, current_y),
                    };
                    let x = current_x + dx;
                    let y = current_y + dy;
                    current_x = x;
                    current_y = y;
                    last_control_point = Some((x1, y1));
                    last_cmd = Some('Q');
                    instructions.push(Instruction::QuadCurveTo { x1, y1, x, y });
                }
            }
            "A" => {
                idx += 1;
                last_cmd = None;
                last_control_point = None;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let rx = parse_num(&tokens, &mut idx)?;
                    let ry = parse_num(&tokens, &mut idx)?;
                    let x_axis_rotation = parse_num(&tokens, &mut idx)?;
                    let large_arc_flag = parse_num(&tokens, &mut idx)? != 0.0;
                    let sweep_flag = parse_num(&tokens, &mut idx)? != 0.0;
                    let x = parse_num(&tokens, &mut idx)?;
                    let y = parse_num(&tokens, &mut idx)?;
                    current_x = x;
                    current_y = y;
                    instructions.push(Instruction::ArcTo {
                        rx: rx.abs(),
                        ry: ry.abs(),
                        x_axis_rotation,
                        large_arc_flag,
                        sweep_flag,
                        x,
                        y,
                    });
                }
            }
            "a" => {
                idx += 1;
                last_cmd = None;
                last_control_point = None;
                while idx < tokens.len() && is_num_token(&tokens[idx]) {
                    let rx = parse_num(&tokens, &mut idx)?;
                    let ry = parse_num(&tokens, &mut idx)?;
                    let x_axis_rotation = parse_num(&tokens, &mut idx)?;
                    let large_arc_flag = parse_num(&tokens, &mut idx)? != 0.0;
                    let sweep_flag = parse_num(&tokens, &mut idx)? != 0.0;
                    let dx = parse_num(&tokens, &mut idx)?;
                    let dy = parse_num(&tokens, &mut idx)?;
                    let x = current_x + dx;
                    let y = current_y + dy;
                    current_x = x;
                    current_y = y;
                    instructions.push(Instruction::ArcTo {
                        rx: rx.abs(),
                        ry: ry.abs(),
                        x_axis_rotation,
                        large_arc_flag,
                        sweep_flag,
                        x,
                        y,
                    });
                }
            }
            "Z" | "z" => {
                idx += 1;
                current_x = start_x;
                current_y = start_y;
                last_cmd = None;
                last_control_point = None;
                instructions.push(Instruction::ClosePath);
            }
            other => {
                return Err(PathParseError::ParseNumberError(other.to_string()));
            }
        }
    }

    Ok(instructions)
}

fn is_svg_cmd(c: char) -> bool {
    matches!(
        c,
        'M' | 'm'
            | 'L'
            | 'l'
            | 'H'
            | 'h'
            | 'V'
            | 'v'
            | 'C'
            | 'c'
            | 'S'
            | 's'
            | 'Q'
            | 'q'
            | 'T'
            | 't'
            | 'A'
            | 'a'
            | 'Z'
            | 'z'
    )
}

fn is_num_token(s: &str) -> bool {
    s.parse::<f64>().is_ok()
}

fn tokenize(d: &str) -> Result<Vec<String>, PathParseError> {
    let mut tokens = Vec::new();
    let mut current_token = String::new();

    let chars: Vec<char> = d.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if is_svg_cmd(c) {
            if !current_token.trim().is_empty() {
                tokens.push(current_token.trim().to_string());
                current_token = String::new();
            }
            tokens.push(c.to_string());
            i += 1;
        } else if c == ',' || c.is_whitespace() {
            if !current_token.trim().is_empty() {
                tokens.push(current_token.trim().to_string());
                current_token = String::new();
            }
            i += 1;
        } else if ((c == '-' || c == '+')
            && !current_token.is_empty()
            && !current_token.ends_with('e')
            && !current_token.ends_with('E'))
            || (c == '.' && current_token.contains('.'))
        {
            tokens.push(current_token.trim().to_string());
            current_token = String::new();
            current_token.push(c);
            i += 1;
        } else {
            current_token.push(c);
            i += 1;
        }
    }

    if !current_token.trim().is_empty() {
        tokens.push(current_token.trim().to_string());
    }

    Ok(tokens)
}

fn parse_num(tokens: &[String], idx: &mut usize) -> Result<f64, PathParseError> {
    if *idx >= tokens.len() {
        return Err(PathParseError::UnexpectedEnd);
    }
    let val = tokens[*idx]
        .parse::<f64>()
        .map_err(|_| PathParseError::ParseNumberError(tokens[*idx].clone()))?;
    *idx += 1;
    Ok(val)
}

/// Serializes instructions back into a standardized SVG `d` path string.
pub fn serialize_instructions(instructions: &[Instruction]) -> String {
    let mut out = Vec::new();

    for inst in instructions {
        match inst {
            Instruction::MoveTo { x, y } => out.push(format!("M {x:.4} {y:.4}")),
            Instruction::LineTo { x, y } => out.push(format!("L {x:.4} {y:.4}")),
            Instruction::CubicCurveTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => out.push(format!("C {x1:.4} {y1:.4} {x2:.4} {y2:.4} {x:.4} {y:.4}")),
            Instruction::QuadCurveTo { x1, y1, x, y } => {
                out.push(format!("Q {x1:.4} {y1:.4} {x:.4} {y:.4}"))
            }
            Instruction::ArcTo {
                rx,
                ry,
                x_axis_rotation,
                large_arc_flag,
                sweep_flag,
                x,
                y,
            } => out.push(format!(
                "A {rx:.4} {ry:.4} {x_axis_rotation:.4} {} {} {x:.4} {y:.4}",
                if *large_arc_flag { 1 } else { 0 },
                if *sweep_flag { 1 } else { 0 }
            )),
            Instruction::ClosePath => out.push("Z".to_string()),
        }
    }

    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_and_serialize_line() {
        let d = "M 10 20 L 100 200 Z";
        let insts = parse_path(d).unwrap();
        assert_eq!(insts.len(), 3);
        assert_eq!(insts[0], Instruction::MoveTo { x: 10.0, y: 20.0 });
        assert_eq!(insts[1], Instruction::LineTo { x: 100.0, y: 200.0 });
        assert_eq!(insts[2], Instruction::ClosePath);

        let res = serialize_instructions(&insts);
        assert_eq!(res, "M 10.0000 20.0000 L 100.0000 200.0000 Z");
    }

    #[test]
    fn test_parse_arc() {
        let d = "M 100 100 L 100 0 A 100 100 0 0 1 200 100 Z";
        let insts = parse_path(d).unwrap();
        assert_eq!(insts.len(), 4);
        assert_eq!(insts[0], Instruction::MoveTo { x: 100.0, y: 100.0 });
        assert_eq!(insts[1], Instruction::LineTo { x: 100.0, y: 0.0 });
        assert_eq!(
            insts[2],
            Instruction::ArcTo {
                rx: 100.0,
                ry: 100.0,
                x_axis_rotation: 0.0,
                large_arc_flag: false,
                sweep_flag: true,
                x: 200.0,
                y: 100.0,
            }
        );
        assert_eq!(insts[3], Instruction::ClosePath);

        let res = serialize_instructions(&insts);
        assert_eq!(
            res,
            "M 100.0000 100.0000 L 100.0000 0.0000 A 100.0000 100.0000 0.0000 0 1 200.0000 100.0000 Z"
        );
    }

    #[test]
    fn test_parse_relative_and_hv() {
        let d = "m 10 20 h 30 v 40 l -10 -10 z";
        let insts = parse_path(d).unwrap();
        assert_eq!(insts.len(), 5);
        assert_eq!(insts[0], Instruction::MoveTo { x: 10.0, y: 20.0 });
        assert_eq!(insts[1], Instruction::LineTo { x: 40.0, y: 20.0 });
        assert_eq!(insts[2], Instruction::LineTo { x: 40.0, y: 60.0 });
        assert_eq!(insts[3], Instruction::LineTo { x: 30.0, y: 50.0 });
        assert_eq!(insts[4], Instruction::ClosePath);
    }

    #[test]
    fn test_parse_chained_beziers_and_smooth() {
        let d = "M 0 0 C 10 20 30 40 50 60 S 70 80 90 100 Q 110 120 130 140 T 150 160";
        let insts = parse_path(d).unwrap();
        assert_eq!(insts.len(), 5);
        assert_eq!(insts[0], Instruction::MoveTo { x: 0.0, y: 0.0 });
        assert_eq!(
            insts[1],
            Instruction::CubicCurveTo {
                x1: 10.0,
                y1: 20.0,
                x2: 30.0,
                y2: 40.0,
                x: 50.0,
                y: 60.0,
            }
        );
        // S reflects (30, 40) across (50, 60): 2*50 - 30 = 70, 2*60 - 40 = 80
        assert_eq!(
            insts[2],
            Instruction::CubicCurveTo {
                x1: 70.0,
                y1: 80.0,
                x2: 70.0,
                y2: 80.0,
                x: 90.0,
                y: 100.0,
            }
        );
        assert_eq!(
            insts[3],
            Instruction::QuadCurveTo {
                x1: 110.0,
                y1: 120.0,
                x: 130.0,
                y: 140.0,
            }
        );
        // T reflects (110, 120) across (130, 140): 2*130 - 110 = 150, 2*140 - 120 = 160
        assert_eq!(
            insts[4],
            Instruction::QuadCurveTo {
                x1: 150.0,
                y1: 160.0,
                x: 150.0,
                y: 160.0,
            }
        );
    }

    #[test]
    fn test_parse_exponential_and_compact_coordinates() {
        let d = "M1e1-2e1L.5.25";
        let insts = parse_path(d).unwrap();
        assert_eq!(insts.len(), 2);
        assert_eq!(insts[0], Instruction::MoveTo { x: 10.0, y: -20.0 });
        assert_eq!(insts[1], Instruction::LineTo { x: 0.5, y: 0.25 });
    }

    #[test]
    fn test_parse_error_invalid_command() {
        let res = parse_path("X 10 20");
        assert!(res.is_err());
    }
}
