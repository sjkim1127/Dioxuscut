//! Color interpolation — Rust port of Remotion's `interpolateColors()`.
//!
//! Supports sRGB linear interpolation between CSS-style hex or rgba colors.

/// A color in sRGB space with premultiplied-alpha aware interpolation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Rgba {
    pub fn new(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }

    /// Parse a hex color string like `"#ff0000"` or `"#ff0000ff"`.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.strip_prefix('#')?;
        match hex.len() {
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f64 / 255.0;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f64 / 255.0;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f64 / 255.0;
                Some(Self::new(r, g, b, 1.0))
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f64 / 255.0;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f64 / 255.0;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f64 / 255.0;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()? as f64 / 255.0;
                Some(Self::new(r, g, b, a))
            }
            _ => None,
        }
    }

    /// Encode as CSS `rgba(r, g, b, a)` string.
    pub fn to_css_rgba(self) -> String {
        format!(
            "rgba({}, {}, {}, {:.4})",
            (self.r * 255.0).round() as u8,
            (self.g * 255.0).round() as u8,
            (self.b * 255.0).round() as u8,
            self.a,
        )
    }

    /// Linear blend in sRGB space.
    pub fn lerp(self, other: Rgba, t: f64) -> Rgba {
        Rgba {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }
}

/// Interpolate between two hex colors based on `t ∈ [0.0, 1.0]`.
///
/// Returns a CSS `rgba(...)` string, matching Remotion's `interpolateColors()`.
///
/// # Example
/// ```rust
/// use dioxuscut_animation::interpolate_colors::interpolate_colors;
///
/// let color = interpolate_colors("#000000", "#ffffff", 0.5);
/// assert_eq!(color, "rgba(128, 128, 128, 1.0000)");
/// ```
pub fn interpolate_colors(from: &str, to: &str, t: f64) -> String {
    let a = Rgba::from_hex(from).unwrap_or(Rgba::new(0.0, 0.0, 0.0, 1.0));
    let b = Rgba::from_hex(to).unwrap_or(Rgba::new(1.0, 1.0, 1.0, 1.0));
    let t_clamped = t.clamp(0.0, 1.0);
    a.lerp(b, t_clamped).to_css_rgba()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parse() {
        let c = Rgba::from_hex("#ff8000").unwrap();
        assert!((c.r - 1.0).abs() < 0.005);
        assert!((c.g - 0.502).abs() < 0.005);
        assert!((c.b - 0.0).abs() < 0.005);
    }

    #[test]
    fn lerp_midpoint() {
        let color = interpolate_colors("#000000", "#ffffff", 0.5);
        assert!(color.starts_with("rgba(128,") || color.contains("128, 128, 128"));
    }

    #[test]
    fn clamp_at_bounds() {
        assert_eq!(
            interpolate_colors("#ff0000", "#0000ff", 0.0),
            "rgba(255, 0, 0, 1.0000)"
        );
        assert_eq!(
            interpolate_colors("#ff0000", "#0000ff", 1.0),
            "rgba(0, 0, 255, 1.0000)"
        );
    }
}
