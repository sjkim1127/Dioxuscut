//! Safe Area layout utilities for vertical short-form video (TikTok, Instagram Reels, YouTube Shorts).
//!
//! Provides platform-specific margins so UI overlays (buttons, titles, audio banners) do not
//! occlude critical captions, avatars, or graphic elements.
//! Equivalent to Remotion's layout utilities / Safe Areas.

use serde::{Deserialize, Serialize};

/// Target platform for safe area calculation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    /// TikTok mobile app vertical overlay (Following/FYP header, right action buttons, bottom caption/sound).
    TikTok,
    /// Instagram Reels vertical overlay (header bar, right like/share buttons, bottom audio/handle).
    InstagramReels,
    /// YouTube Shorts vertical overlay (top header, right buttons, bottom channel/title bar).
    YouTubeShorts,
    /// Standard SMPTE Action Safe (90% active screen, 5% insets).
    ActionSafe,
    /// Standard SMPTE Title Safe (80% active screen, 10% insets).
    TitleSafe,
}

/// Margin insets in pixels for a safe area.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SafeAreaInsets {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

impl SafeAreaInsets {
    /// Creates a new `SafeAreaInsets` instance.
    pub fn new(top: f32, bottom: f32, left: f32, right: f32) -> Self {
        Self {
            top: top.max(0.0),
            bottom: bottom.max(0.0),
            left: left.max(0.0),
            right: right.max(0.0),
        }
    }

    /// Returns the unobstructed inner bounding rectangle `(x, y, width, height)`.
    pub fn to_rect(&self, comp_width: f32, comp_height: f32) -> (f32, f32, f32, f32) {
        let x = self.left;
        let y = self.top;
        let width = (comp_width - self.left - self.right).max(0.0);
        let height = (comp_height - self.top - self.bottom).max(0.0);
        (x, y, width, height)
    }
}

/// Calculates the platform safe area insets scaled to the composition resolution.
pub fn get_safe_area_insets(platform: Platform, width: f32, height: f32) -> SafeAreaInsets {
    let scale_y = height / 1920.0;
    let scale_x = width / 1080.0;

    match platform {
        Platform::TikTok => SafeAreaInsets::new(
            120.0 * scale_y,
            340.0 * scale_y,
            40.0 * scale_x,
            120.0 * scale_x,
        ),
        Platform::InstagramReels => SafeAreaInsets::new(
            140.0 * scale_y,
            360.0 * scale_y,
            40.0 * scale_x,
            120.0 * scale_x,
        ),
        Platform::YouTubeShorts => SafeAreaInsets::new(
            120.0 * scale_y,
            380.0 * scale_y,
            40.0 * scale_x,
            140.0 * scale_x,
        ),
        Platform::ActionSafe => {
            SafeAreaInsets::new(height * 0.05, height * 0.05, width * 0.05, width * 0.05)
        }
        Platform::TitleSafe => {
            SafeAreaInsets::new(height * 0.10, height * 0.10, width * 0.10, width * 0.10)
        }
    }
}

/// Estimates single-line text dimensions `(width, height)` given text and font size.
pub fn measure_text_approx(text: &str, font_size: f32) -> (f32, f32) {
    let char_count = text.chars().count() as f32;
    // Standard proportional sans-serif average character aspect ratio is ~0.55
    let width = char_count * (font_size * 0.55);
    let height = font_size * 1.25;
    (width, height)
}

/// Calculates the optimal font size to fit text within `max_width` and `max_height`.
///
/// Ported from Remotion's `@remotion/layout-utils` `fitText()`.
pub fn fit_text(
    text: &str,
    max_width: f32,
    max_height: f32,
    min_font_size: f32,
    max_font_size: f32,
) -> f32 {
    let char_count = text.chars().count().max(1) as f32;
    // Width constraint: char_count * (font_size * 0.55) <= max_width
    let font_size_from_w = max_width / (char_count * 0.55);
    // Height constraint: font_size * 1.25 <= max_height
    let font_size_from_h = max_height / 1.25;

    font_size_from_w
        .min(font_size_from_h)
        .clamp(min_font_size, max_font_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tiktok_safe_area_1080x1920() {
        let insets = get_safe_area_insets(Platform::TikTok, 1080.0, 1920.0);
        assert_eq!(insets.top, 120.0);
        assert_eq!(insets.bottom, 340.0);
        assert_eq!(insets.left, 40.0);
        assert_eq!(insets.right, 120.0);

        let (x, y, w, h) = insets.to_rect(1080.0, 1920.0);
        assert_eq!(x, 40.0);
        assert_eq!(y, 120.0);
        assert_eq!(w, 1080.0 - 40.0 - 120.0);
        assert_eq!(h, 1920.0 - 120.0 - 340.0);
    }

    #[test]
    fn test_scaled_safe_area_720x1280() {
        let insets = get_safe_area_insets(Platform::TikTok, 720.0, 1280.0);
        let scale_y = 1280.0 / 1920.0;
        let scale_x = 720.0 / 1080.0;
        assert!((insets.top - 120.0 * scale_y).abs() < 1e-4);
        assert!((insets.left - 40.0 * scale_x).abs() < 1e-4);
    }

    #[test]
    fn test_fit_text() {
        let size = fit_text("HELLO WORLD", 300.0, 80.0, 12.0, 72.0);
        assert!((12.0..=72.0).contains(&size));
        let (w, h) = measure_text_approx("HELLO WORLD", size);
        assert!(w <= 300.5);
        assert!(h <= 80.5);
    }
}
