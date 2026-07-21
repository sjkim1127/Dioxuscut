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

    while idx < tokens.len() {
        match tokens[idx].as_str() {
            "M" => {
                idx += 1;
                let x = parse_num(&tokens, &mut idx)?;
                let y = parse_num(&tokens, &mut idx)?;
                current_x = x;
                current_y = y;
                instructions.push(Instruction::MoveTo { x, y });
            }
            "m" => {
                idx += 1;
                let dx = parse_num(&tokens, &mut idx)?;
                let dy = parse_num(&tokens, &mut idx)?;
                current_x += dx;
                current_y += dy;
                instructions.push(Instruction::MoveTo {
                    x: current_x,
                    y: current_y,
                });
            }
            "L" => {
                idx += 1;
                let x = parse_num(&tokens, &mut idx)?;
                let y = parse_num(&tokens, &mut idx)?;
                current_x = x;
                current_y = y;
                instructions.push(Instruction::LineTo { x, y });
            }
            "l" => {
                idx += 1;
                let dx = parse_num(&tokens, &mut idx)?;
                let dy = parse_num(&tokens, &mut idx)?;
                current_x += dx;
                current_y += dy;
                instructions.push(Instruction::LineTo {
                    x: current_x,
                    y: current_y,
                });
            }
            "H" => {
                idx += 1;
                let x = parse_num(&tokens, &mut idx)?;
                current_x = x;
                instructions.push(Instruction::LineTo {
                    x: current_x,
                    y: current_y,
                });
            }
            "h" => {
                idx += 1;
                let dx = parse_num(&tokens, &mut idx)?;
                current_x += dx;
                instructions.push(Instruction::LineTo {
                    x: current_x,
                    y: current_y,
                });
            }
            "V" => {
                idx += 1;
                let y = parse_num(&tokens, &mut idx)?;
                current_y = y;
                instructions.push(Instruction::LineTo {
                    x: current_x,
                    y: current_y,
                });
            }
            "v" => {
                idx += 1;
                let dy = parse_num(&tokens, &mut idx)?;
                current_y += dy;
                instructions.push(Instruction::LineTo {
                    x: current_x,
                    y: current_y,
                });
            }
            "C" => {
                idx += 1;
                let x1 = parse_num(&tokens, &mut idx)?;
                let y1 = parse_num(&tokens, &mut idx)?;
                let x2 = parse_num(&tokens, &mut idx)?;
                let y2 = parse_num(&tokens, &mut idx)?;
                let x = parse_num(&tokens, &mut idx)?;
                let y = parse_num(&tokens, &mut idx)?;
                current_x = x;
                current_y = y;
                instructions.push(Instruction::CubicCurveTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                });
            }
            "c" => {
                idx += 1;
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
                instructions.push(Instruction::CubicCurveTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                });
            }
            "Q" => {
                idx += 1;
                let x1 = parse_num(&tokens, &mut idx)?;
                let y1 = parse_num(&tokens, &mut idx)?;
                let x = parse_num(&tokens, &mut idx)?;
                let y = parse_num(&tokens, &mut idx)?;
                current_x = x;
                current_y = y;
                instructions.push(Instruction::QuadCurveTo { x1, y1, x, y });
            }
            "q" => {
                idx += 1;
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
                instructions.push(Instruction::QuadCurveTo { x1, y1, x, y });
            }
            "Z" | "z" => {
                idx += 1;
                instructions.push(Instruction::ClosePath);
            }
            other => {
                return Err(PathParseError::ParseNumberError(other.to_string()));
            }
        }
    }

    Ok(instructions)
}

fn tokenize(d: &str) -> Result<Vec<String>, PathParseError> {
    let mut tokens = Vec::new();
    let mut current_token = String::new();

    let chars: Vec<char> = d.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c.is_alphabetic() {
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
        } else if c == '-'
            && !current_token.is_empty()
            && !current_token.ends_with('e')
            && !current_token.ends_with('E')
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
}
