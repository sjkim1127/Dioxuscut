//! SVG path interpolation and length approximation utilities.
//!
//! Ported from Remotion's `@remotion/paths` helpers:
//! - [`evolve_path`] — draw-on animation via stroke-dash trick
//! - [`approximate_path_length`] — fast string-based path length estimation
//! - [`interpolate_path`] — linear interpolation between two SVG paths

pub use crate::evolve_path::{evolve_path, evolve_path_with_length};

// ---------------------------------------------------------------------------
// approximate_path_length
// ---------------------------------------------------------------------------

/// Computes an approximation of the total length of an SVG `d` path string.
///
/// Handles absolute and relative: `M`/`m`, `L`/`l`, `H`/`h`, `V`/`v`,
/// `C`/`c`, `Q`/`q`, `A`/`a`, `Z`/`z`.  Bezier curves and arcs are approximated
/// accurately.
pub fn approximate_path_length(path: &str) -> f64 {
    let tokens = tokenize_path(path);
    let mut idx = 0;
    let mut total = 0.0;
    let mut cx = 0.0f64;
    let mut cy = 0.0f64;
    let mut start_x = 0.0f64;
    let mut start_y = 0.0f64;

    while idx < tokens.len() {
        let cmd = tokens[idx].as_str();
        idx += 1;

        match cmd {
            "M" => {
                let x = next_f64(&tokens, &mut idx);
                let y = next_f64(&tokens, &mut idx);
                cx = x;
                cy = y;
                start_x = cx;
                start_y = cy;
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let x2 = next_f64(&tokens, &mut idx);
                    let y2 = next_f64(&tokens, &mut idx);
                    total += dist(cx, cy, x2, y2);
                    cx = x2;
                    cy = y2;
                }
            }
            "m" => {
                let dx = next_f64(&tokens, &mut idx);
                let dy = next_f64(&tokens, &mut idx);
                cx += dx;
                cy += dy;
                start_x = cx;
                start_y = cy;
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let dx2 = next_f64(&tokens, &mut idx);
                    let dy2 = next_f64(&tokens, &mut idx);
                    total += dist(cx, cy, cx + dx2, cy + dy2);
                    cx += dx2;
                    cy += dy2;
                }
            }
            "L" => {
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let x = next_f64(&tokens, &mut idx);
                    let y = next_f64(&tokens, &mut idx);
                    total += dist(cx, cy, x, y);
                    cx = x;
                    cy = y;
                }
            }
            "l" => {
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let dx = next_f64(&tokens, &mut idx);
                    let dy = next_f64(&tokens, &mut idx);
                    total += dist(0.0, 0.0, dx, dy);
                    cx += dx;
                    cy += dy;
                }
            }
            "H" => {
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let x = next_f64(&tokens, &mut idx);
                    total += (x - cx).abs();
                    cx = x;
                }
            }
            "h" => {
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let dx = next_f64(&tokens, &mut idx);
                    total += dx.abs();
                    cx += dx;
                }
            }
            "V" => {
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let y = next_f64(&tokens, &mut idx);
                    total += (y - cy).abs();
                    cy = y;
                }
            }
            "v" => {
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let dy = next_f64(&tokens, &mut idx);
                    total += dy.abs();
                    cy += dy;
                }
            }
            "C" => {
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let x1 = next_f64(&tokens, &mut idx);
                    let y1 = next_f64(&tokens, &mut idx);
                    let x2 = next_f64(&tokens, &mut idx);
                    let y2 = next_f64(&tokens, &mut idx);
                    let x = next_f64(&tokens, &mut idx);
                    let y = next_f64(&tokens, &mut idx);
                    total += cubic_len((cx, cy), (x1, y1), (x2, y2), (x, y));
                    cx = x;
                    cy = y;
                }
            }
            "c" => {
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let dx1 = next_f64(&tokens, &mut idx);
                    let dy1 = next_f64(&tokens, &mut idx);
                    let dx2 = next_f64(&tokens, &mut idx);
                    let dy2 = next_f64(&tokens, &mut idx);
                    let dx = next_f64(&tokens, &mut idx);
                    let dy = next_f64(&tokens, &mut idx);
                    total += cubic_len(
                        (cx, cy),
                        (cx + dx1, cy + dy1),
                        (cx + dx2, cy + dy2),
                        (cx + dx, cy + dy),
                    );
                    cx += dx;
                    cy += dy;
                }
            }
            "Q" => {
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let x1 = next_f64(&tokens, &mut idx);
                    let y1 = next_f64(&tokens, &mut idx);
                    let x = next_f64(&tokens, &mut idx);
                    let y = next_f64(&tokens, &mut idx);
                    total += quad_len(cx, cy, x1, y1, x, y);
                    cx = x;
                    cy = y;
                }
            }
            "q" => {
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let dx1 = next_f64(&tokens, &mut idx);
                    let dy1 = next_f64(&tokens, &mut idx);
                    let dx = next_f64(&tokens, &mut idx);
                    let dy = next_f64(&tokens, &mut idx);
                    total += quad_len(cx, cy, cx + dx1, cy + dy1, cx + dx, cy + dy);
                    cx += dx;
                    cy += dy;
                }
            }
            "A" => {
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let rx = next_f64(&tokens, &mut idx);
                    let ry = next_f64(&tokens, &mut idx);
                    let x_axis_rotation = next_f64(&tokens, &mut idx);
                    let large_arc_flag = next_f64(&tokens, &mut idx) != 0.0;
                    let sweep_flag = next_f64(&tokens, &mut idx) != 0.0;
                    let x = next_f64(&tokens, &mut idx);
                    let y = next_f64(&tokens, &mut idx);
                    total += crate::length::arc_segment_length(
                        (cx, cy),
                        (x, y),
                        rx,
                        ry,
                        x_axis_rotation,
                        large_arc_flag,
                        sweep_flag,
                    );
                    cx = x;
                    cy = y;
                }
            }
            "a" => {
                while idx < tokens.len() && is_number(&tokens[idx]) {
                    let rx = next_f64(&tokens, &mut idx);
                    let ry = next_f64(&tokens, &mut idx);
                    let x_axis_rotation = next_f64(&tokens, &mut idx);
                    let large_arc_flag = next_f64(&tokens, &mut idx) != 0.0;
                    let sweep_flag = next_f64(&tokens, &mut idx) != 0.0;
                    let dx = next_f64(&tokens, &mut idx);
                    let dy = next_f64(&tokens, &mut idx);
                    let x = cx + dx;
                    let y = cy + dy;
                    total += crate::length::arc_segment_length(
                        (cx, cy),
                        (x, y),
                        rx,
                        ry,
                        x_axis_rotation,
                        large_arc_flag,
                        sweep_flag,
                    );
                    cx = x;
                    cy = y;
                }
            }
            "Z" | "z" => {
                total += dist(cx, cy, start_x, start_y);
                cx = start_x;
                cy = start_y;
            }
            _ => {}
        }
    }

    total
}

// ---------------------------------------------------------------------------
// interpolate_path
// ---------------------------------------------------------------------------

/// Linearly interpolates between two SVG path strings.
///
/// Both paths should have the same command structure (same number of numeric
/// arguments).  If the numeric argument count differs the function falls back
/// to returning `from` when `progress < 0.5` and `to` otherwise.
pub fn interpolate_path(from: &str, to: &str, progress: f64) -> String {
    let t = progress.clamp(0.0, 1.0);
    let (from_nums, template) = extract_nums_and_template(from);
    let (to_nums, _) = extract_nums_and_template(to);

    if from_nums.len() != to_nums.len() {
        return if t < 0.5 {
            from.to_string()
        } else {
            to.to_string()
        };
    }

    let interp: Vec<f64> = from_nums
        .iter()
        .zip(to_nums.iter())
        .map(|(a, b)| a * (1.0 - t) + b * t)
        .collect();

    reconstruct(&template, &interp)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn tokenize_path(d: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    for c in d.chars() {
        if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
            if !current.trim().is_empty() {
                tokens.push(current.trim().to_string());
                current.clear();
            }
            tokens.push(c.to_string());
        } else if c == ',' || c.is_whitespace() {
            if !current.trim().is_empty() {
                tokens.push(current.trim().to_string());
                current.clear();
            }
        } else if ((c == '-' || c == '+')
            && !current.is_empty()
            && !current.ends_with('e')
            && !current.ends_with('E'))
            || (c == '.' && current.contains('.'))
        {
            tokens.push(current.trim().to_string());
            current.clear();
            current.push(c);
        } else {
            current.push(c);
        }
    }
    if !current.trim().is_empty() {
        tokens.push(current.trim().to_string());
    }
    tokens
}

fn is_number(s: &str) -> bool {
    s.parse::<f64>().is_ok()
}

fn next_f64(tokens: &[String], idx: &mut usize) -> f64 {
    if *idx < tokens.len() {
        let v = tokens[*idx].parse::<f64>().unwrap_or(0.0);
        *idx += 1;
        v
    } else {
        0.0
    }
}

fn dist(x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    let dx = x1 - x0;
    let dy = y1 - y0;
    (dx * dx + dy * dy).sqrt()
}

fn cubic_len(
    (x0, y0): (f64, f64),
    (x1, y1): (f64, f64),
    (x2, y2): (f64, f64),
    (x3, y3): (f64, f64),
) -> f64 {
    const N: usize = 10;
    let (mut len, mut px, mut py) = (0.0, x0, y0);
    for i in 1..=N {
        let t = i as f64 / N as f64;
        let mt = 1.0 - t;
        let qx =
            mt * mt * mt * x0 + 3.0 * mt * mt * t * x1 + 3.0 * mt * t * t * x2 + t * t * t * x3;
        let qy =
            mt * mt * mt * y0 + 3.0 * mt * mt * t * y1 + 3.0 * mt * t * t * y2 + t * t * t * y3;
        len += dist(px, py, qx, qy);
        px = qx;
        py = qy;
    }
    len
}

fn quad_len(x0: f64, y0: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    const N: usize = 10;
    let (mut len, mut px, mut py) = (0.0, x0, y0);
    for i in 1..=N {
        let t = i as f64 / N as f64;
        let mt = 1.0 - t;
        let qx = mt * mt * x0 + 2.0 * mt * t * x1 + t * t * x2;
        let qy = mt * mt * y0 + 2.0 * mt * t * y1 + t * t * y2;
        len += dist(px, py, qx, qy);
        px = qx;
        py = qy;
    }
    len
}

// ---------------------------------------------------------------------------
// Template extraction / reconstruction for interpolate_path
// ---------------------------------------------------------------------------

enum Segment {
    Literal(String),
    Number,
}

fn extract_nums_and_template(path: &str) -> (Vec<f64>, Vec<Segment>) {
    let mut nums: Vec<f64> = Vec::new();
    let mut template: Vec<Segment> = Vec::new();
    let mut literal = String::new();
    let chars: Vec<char> = path.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        let is_num_start = c.is_ascii_digit()
            || (c == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
            || (c == '-'
                && i + 1 < chars.len()
                && (chars[i + 1].is_ascii_digit() || chars[i + 1] == '.')
                // only treat '-' as number start if preceded by non-digit
                && (literal.is_empty()
                    || !literal.ends_with(|ch: char| ch.is_ascii_digit() || ch == '.')));
        if is_num_start {
            if !literal.is_empty() {
                template.push(Segment::Literal(literal.clone()));
                literal.clear();
            }
            let start = i;
            if chars[i] == '-' {
                i += 1;
            }
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i < chars.len() && chars[i] == '.' {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                i += 1;
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    i += 1;
                }
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let num_str: String = chars[start..i].iter().collect();
            if let Ok(v) = num_str.parse::<f64>() {
                nums.push(v);
                template.push(Segment::Number);
            } else {
                template.push(Segment::Literal(num_str));
            }
        } else {
            literal.push(c);
            i += 1;
        }
    }
    if !literal.is_empty() {
        template.push(Segment::Literal(literal));
    }
    (nums, template)
}

fn reconstruct(template: &[Segment], numbers: &[f64]) -> String {
    let mut out = String::new();
    let mut ni = 0;
    for seg in template {
        match seg {
            Segment::Literal(s) => out.push_str(s),
            Segment::Number => {
                if ni < numbers.len() {
                    let v = numbers[ni];
                    if v.fract() == 0.0 && v.abs() < 1e10 {
                        out.push_str(&format!("{}", v as i64));
                    } else {
                        out.push_str(&format!("{:.4}", v));
                    }
                    ni += 1;
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn approximate_path_length_line() {
        // 3-4-5 right triangle
        let len = approximate_path_length("M 0 0 L 3 4");
        assert!((len - 5.0).abs() < 0.01, "expected ~5, got {len}");
    }

    #[test]
    fn approximate_path_length_rect() {
        let len = approximate_path_length("M 0 0 L 100 0 L 100 100 L 0 100 Z");
        assert!((len - 400.0).abs() < 0.1, "expected ~400, got {len}");
    }

    #[test]
    fn approximate_path_length_h_v() {
        let len = approximate_path_length("M 0 0 H 50 V 50");
        assert!((len - 100.0).abs() < 0.01, "expected ~100, got {len}");
    }

    #[test]
    fn approximate_path_length_arc() {
        let len = approximate_path_length("M 100 0 A 100 100 0 0 1 0 100");
        let expected = 0.5 * PI * 100.0;
        assert!(
            (len - expected).abs() < 0.5,
            "expected ~{expected}, got {len}"
        );
    }

    #[test]
    fn interpolate_path_midpoint() {
        let from = "M 0 0 L 100 0";
        let to = "M 0 0 L 0 100";
        let mid = interpolate_path(from, to, 0.5);
        assert!(mid.contains("50"), "mid path should contain 50: {mid}");
    }

    #[test]
    fn interpolate_path_incompatible_falls_back() {
        let from = "M 0 0 L 10 0";
        let to = "M 0 0 L 10 0 L 20 0";
        let r_lo = interpolate_path(from, to, 0.3);
        let r_hi = interpolate_path(from, to, 0.7);
        assert_eq!(r_lo, from);
        assert_eq!(r_hi, to);
    }

    #[test]
    fn interpolate_path_endpoints() {
        let from = "M 10 20 L 30 40";
        let to = "M 50 60 L 70 80";
        assert_eq!(interpolate_path(from, to, 0.0), from);
        assert_eq!(interpolate_path(from, to, 1.0), to);
    }

    #[test]
    fn interpolate_path_beziers_and_negative() {
        let from = "M -10 -20 C 0 10 20 30 40 50";
        let to = "M 10 20 C 20 30 40 50 60 70";
        let mid = interpolate_path(from, to, 0.5);
        assert_eq!(mid, "M 0 0 C 10 20 30 40 50 60");
    }

    #[test]
    fn approximate_path_length_parity_with_get_length() {
        let path = "M 0 0 L 100 0 L 100 100 A 50 50 0 0 1 50 150 Z";
        let approx = approximate_path_length(path);
        let actual = crate::length::get_length(path);
        assert!(
            (approx - actual).abs() < 1.0,
            "approx {approx} vs actual {actual}"
        );
    }
}
