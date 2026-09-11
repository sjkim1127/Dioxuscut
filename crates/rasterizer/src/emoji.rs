//! Color Emoji rendering and text grapheme segmentation.
//!
//! Provides:
//! - Unicode emoji detection (`is_emoji_char`, `is_emoji_grapheme`)
//! - Text run segmentation (`split_text_and_emojis`) into [`TextRun`]
//! - 32-bit RGBA procedural vector rendering for top viral video emojis
//!   (🔥, 🚀, 💡, 🤖, ❤️, 👍, ⚡, 🎉, 🎬, 📈, 💰, 🚨, 😱, 🤯, 👏, 💯, ✨, 🎯, 👑, ⭐)
//! - Standalone and inline emoji sprite generation with full alpha blending

use image::RgbaImage;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};
use unicode_segmentation::UnicodeSegmentation;

/// A segment of text that is either plain text or a full-color emoji.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextRun {
    Text(String),
    Emoji(String),
}

/// Checks whether a char belongs to common Unicode emoji code blocks.
pub fn is_emoji_char(c: char) -> bool {
    let u = c as u32;
    matches!(
        u,
        0x1F300..=0x1F5FF // Misc Symbols and Pictographs (🔥, 🚀, 💡, 💰, 🎬, 📈, 🚨, skin tones)
        | 0x1F600..=0x1F64F // Emoticons (😀, 😱, 🤯, etc.)
        | 0x1F680..=0x1F6FF // Transport and Map (🚀, 🛸, etc.)
        | 0x1F700..=0x1F77F // Alchemical Symbols
        | 0x1F780..=0x1F7FF // Geometric Shapes Extended
        | 0x1F800..=0x1F8FF // Supplemental Arrows-C
        | 0x1F900..=0x1F9FF // Supplemental Symbols and Pictographs (🤖, 🧠, 🦾, 🥳, etc.)
        | 0x1FA00..=0x1FA6F // Chess Symbols
        | 0x1FA70..=0x1FAFF // Symbols and Pictographs Extended-A (🪙, 🪓, 🪞, etc.)
        | 0x2600..=0x26FF   // Misc Symbols (⚡, ⚠️, ⚓, ☕, etc.)
        | 0x2700..=0x27BF   // Dingbats (✨, 🎯, ⭐, ✏️, etc.)
        | 0x1F1E6..=0x1F1FF // Regional Indicator Symbols (Flags)
        | 0xFE0F            // Variation Selector-16 (Emoji presentation)
        | 0x200D            // Zero-Width Joiner (ZWJ)
    )
}

/// Checks whether a grapheme cluster contains an emoji sequence.
pub fn is_emoji_grapheme(grapheme: &str) -> bool {
    if grapheme.is_empty() {
        return false;
    }
    // Check if any character in the cluster is an emoji character
    grapheme.chars().any(is_emoji_char)
}

/// Splits a text string into consecutive runs of plain text and emojis.
pub fn split_text_and_emojis(text: &str) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut current_text = String::new();

    for grapheme in text.graphemes(true) {
        if is_emoji_grapheme(grapheme) {
            if !current_text.is_empty() {
                runs.push(TextRun::Text(std::mem::take(&mut current_text)));
            }
            runs.push(TextRun::Emoji(grapheme.to_string()));
        } else {
            current_text.push_str(grapheme);
        }
    }

    if !current_text.is_empty() {
        runs.push(TextRun::Text(current_text));
    }

    runs
}

type EmojiCache = Mutex<HashMap<(String, u32), Arc<RgbaImage>>>;

/// Global cache of rendered emoji sprites keyed by `(emoji_cluster, size_px)`.
fn emoji_cache() -> &'static EmojiCache {
    static CACHE: OnceLock<EmojiCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Renders a color emoji to an RGBA image of size `(size_px, size_px)`.
pub fn render_emoji(emoji: &str, size_px: u32) -> Option<Arc<RgbaImage>> {
    let size = size_px.clamp(8, 1024);
    let key = (emoji.to_string(), size);

    {
        let cache = emoji_cache().lock().unwrap();
        if let Some(img) = cache.get(&key) {
            return Some(Arc::clone(img));
        }
    }

    let rendered = render_emoji_internal(emoji, size)?;
    let arc = Arc::new(rendered);

    let mut cache = emoji_cache().lock().unwrap();
    cache.insert(key, Arc::clone(&arc));
    Some(arc)
}

/// Internal procedural/vector renderer for known emojis.
fn render_emoji_internal(emoji: &str, size: u32) -> Option<RgbaImage> {
    let mut pixmap = Pixmap::new(size, size)?;
    let s = size as f32;
    let norm = Transform::from_scale(s / 100.0, s / 100.0);

    let clean_emoji = emoji.trim_matches(['\u{FE0F}', '\u{FE0E}'].as_ref());

    match clean_emoji {
        // 🔥 Fire
        "🔥" => {
            draw_fire(&mut pixmap, norm);
        }
        // 🚀 Rocket
        "🚀" => {
            draw_rocket(&mut pixmap, norm);
        }
        // 💡 Lightbulb
        "💡" => {
            draw_lightbulb(&mut pixmap, norm);
        }
        // ⚡ Lightning / Zap
        "⚡" => {
            draw_lightning(&mut pixmap, norm);
        }
        // ❤️ Red Heart
        "❤️" | "❤" => {
            draw_heart(&mut pixmap, norm);
        }
        // ⭐ Star
        "⭐" | "★" => {
            draw_star(&mut pixmap, norm);
        }
        // 🤖 Robot
        "🤖" => {
            draw_robot(&mut pixmap, norm);
        }
        // 💯 100 Points
        "💯" => {
            draw_hundred(&mut pixmap, norm);
        }
        // 🎯 Target / Bullseye
        "🎯" => {
            draw_target(&mut pixmap, norm);
        }
        // 👑 Crown
        "👑" => {
            draw_crown(&mut pixmap, norm);
        }
        // 💰 Money Bag
        "💰" => {
            draw_money_bag(&mut pixmap, norm);
        }
        // 📈 Chart Increasing
        "📈" => {
            draw_chart(&mut pixmap, norm);
        }
        // 🎬 Clapperboard
        "🎬" => {
            draw_clapper(&mut pixmap, norm);
        }
        // ✨ Sparkles
        "✨" => {
            draw_sparkles(&mut pixmap, norm);
        }
        // 🚨 Siren
        "🚨" => {
            draw_siren(&mut pixmap, norm);
        }
        // 👍 Thumbs Up
        "👍" => {
            draw_thumbs_up(&mut pixmap, norm);
        }
        // 💎 Diamond
        "💎" => {
            draw_diamond(&mut pixmap, norm);
        }
        // Generic / Fallback emoji glyph: cheerful expressive circle badge
        _ => {
            draw_fallback_emoji(&mut pixmap, norm, emoji);
        }
    }

    let rgba_data = pixmap.take();
    RgbaImage::from_raw(size, size, rgba_data)
}

// ----------------------------------------------------------------------------
// Procedural Vector Drawings (100x100 virtual coordinate space)
// ----------------------------------------------------------------------------

fn paint_fill(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(r, g, b, a);
    paint.anti_alias = true;
    paint
}

fn add_rect(pb: &mut PathBuilder, x: f32, y: f32, w: f32, h: f32) {
    if let Some(r) = Rect::from_xywh(x, y, w, h) {
        pb.push_rect(r);
    }
}

fn draw_fire(pixmap: &mut Pixmap, ts: Transform) {
    // Outer flame (Deep Red/Orange)
    let mut pb = PathBuilder::new();
    pb.move_to(50.0, 95.0);
    pb.cubic_to(20.0, 95.0, 10.0, 70.0, 15.0, 52.0);
    pb.cubic_to(18.0, 38.0, 30.0, 25.0, 38.0, 15.0);
    pb.cubic_to(40.0, 26.0, 48.0, 32.0, 52.0, 25.0);
    pb.cubic_to(58.0, 14.0, 55.0, 8.0, 50.0, 5.0);
    pb.cubic_to(65.0, 10.0, 88.0, 35.0, 85.0, 62.0);
    pb.cubic_to(82.0, 82.0, 70.0, 95.0, 50.0, 95.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 69, 0, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Mid flame (Bright Orange)
    let mut pb = PathBuilder::new();
    pb.move_to(50.0, 93.0);
    pb.cubic_to(28.0, 93.0, 22.0, 75.0, 26.0, 60.0);
    pb.cubic_to(30.0, 48.0, 40.0, 38.0, 45.0, 28.0);
    pb.cubic_to(47.0, 38.0, 54.0, 42.0, 58.0, 36.0);
    pb.cubic_to(65.0, 28.0, 62.0, 20.0, 57.0, 16.0);
    pb.cubic_to(70.0, 22.0, 80.0, 45.0, 76.0, 68.0);
    pb.cubic_to(73.0, 83.0, 65.0, 93.0, 50.0, 93.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 140, 0, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Inner core flame (Vibrant Yellow)
    let mut pb = PathBuilder::new();
    pb.move_to(50.0, 90.0);
    pb.cubic_to(36.0, 90.0, 32.0, 78.0, 35.0, 68.0);
    pb.cubic_to(38.0, 58.0, 46.0, 50.0, 50.0, 40.0);
    pb.cubic_to(54.0, 50.0, 62.0, 58.0, 65.0, 68.0);
    pb.cubic_to(68.0, 78.0, 64.0, 90.0, 50.0, 90.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 235, 59, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
}

fn draw_rocket(pixmap: &mut Pixmap, ts: Transform) {
    // Rocket exhaust flame
    let mut pb = PathBuilder::new();
    pb.move_to(32.0, 68.0);
    pb.line_to(15.0, 85.0);
    pb.line_to(22.0, 74.0);
    pb.line_to(8.0, 92.0);
    pb.line_to(26.0, 78.0);
    pb.line_to(32.0, 68.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 87, 34, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Left fin (Red)
    let mut pb = PathBuilder::new();
    pb.move_to(35.0, 50.0);
    pb.line_to(18.0, 65.0);
    pb.line_to(30.0, 75.0);
    pb.line_to(45.0, 65.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(229, 57, 53, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Right fin (Red)
    let mut pb = PathBuilder::new();
    pb.move_to(50.0, 35.0);
    pb.line_to(65.0, 18.0);
    pb.line_to(75.0, 30.0);
    pb.line_to(65.0, 45.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(229, 57, 53, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Main fuselage (Silver/White)
    let mut pb = PathBuilder::new();
    pb.move_to(85.0, 15.0);
    pb.cubic_to(60.0, 18.0, 35.0, 40.0, 30.0, 70.0);
    pb.line_to(70.0, 30.0);
    pb.cubic_to(75.0, 25.0, 82.0, 18.0, 85.0, 15.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(245, 245, 250, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Nose cone (Red)
    let mut pb = PathBuilder::new();
    pb.move_to(85.0, 15.0);
    pb.cubic_to(78.0, 16.0, 70.0, 22.0, 66.0, 26.0);
    pb.line_to(74.0, 34.0);
    pb.cubic_to(78.0, 30.0, 84.0, 22.0, 85.0, 15.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(229, 57, 53, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Porthole / Window (Cyan)
    let mut pb = PathBuilder::new();
    pb.push_circle(58.0, 42.0, 8.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(38, 198, 218, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
}

fn draw_lightbulb(pixmap: &mut Pixmap, ts: Transform) {
    // Glass bulb (Bright Yellow)
    let mut pb = PathBuilder::new();
    pb.move_to(35.0, 65.0);
    pb.cubic_to(20.0, 55.0, 15.0, 35.0, 28.0, 20.0);
    pb.cubic_to(40.0, 5.0, 60.0, 5.0, 72.0, 20.0);
    pb.cubic_to(85.0, 35.0, 80.0, 55.0, 65.0, 65.0);
    pb.line_to(65.0, 75.0);
    pb.line_to(35.0, 75.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 214, 0, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Screw base (Metallic Gray)
    let mut pb = PathBuilder::new();
    add_rect(&mut pb, 38.0, 76.0, 24.0, 5.0);
    add_rect(&mut pb, 41.0, 83.0, 18.0, 5.0);
    add_rect(&mut pb, 44.0, 90.0, 12.0, 4.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(158, 158, 158, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Filament glow (Amber)
    let mut pb = PathBuilder::new();
    pb.move_to(44.0, 65.0);
    pb.line_to(46.0, 45.0);
    pb.line_to(54.0, 45.0);
    pb.line_to(56.0, 65.0);
    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width: 3.0,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint_fill(255, 143, 0, 255), &stroke, ts, None);
    }
}

fn draw_lightning(pixmap: &mut Pixmap, ts: Transform) {
    let mut pb = PathBuilder::new();
    pb.move_to(58.0, 5.0);
    pb.line_to(20.0, 52.0);
    pb.line_to(48.0, 52.0);
    pb.line_to(38.0, 95.0);
    pb.line_to(80.0, 42.0);
    pb.line_to(52.0, 42.0);
    pb.line_to(58.0, 5.0);
    pb.close();
    if let Some(path) = pb.finish() {
        // Drop shadow / edge
        let shadow_ts = ts.pre_translate(2.0, 2.0);
        pixmap.fill_path(
            &path,
            &paint_fill(239, 108, 0, 255),
            FillRule::Winding,
            shadow_ts,
            None,
        );
        // Main electric yellow
        pixmap.fill_path(
            &path,
            &paint_fill(255, 235, 59, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
}

fn draw_heart(pixmap: &mut Pixmap, ts: Transform) {
    let mut pb = PathBuilder::new();
    pb.move_to(50.0, 85.0);
    pb.cubic_to(15.0, 60.0, 5.0, 35.0, 18.0, 18.0);
    pb.cubic_to(30.0, 5.0, 45.0, 12.0, 50.0, 24.0);
    pb.cubic_to(55.0, 12.0, 70.0, 5.0, 82.0, 18.0);
    pb.cubic_to(95.0, 35.0, 85.0, 60.0, 50.0, 85.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(229, 28, 35, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Glossy highlight
    let mut pb = PathBuilder::new();
    pb.move_to(25.0, 22.0);
    pb.cubic_to(28.0, 15.0, 35.0, 12.0, 42.0, 15.0);
    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width: 3.5,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint_fill(255, 255, 255, 160), &stroke, ts, None);
    }
}

fn draw_star(pixmap: &mut Pixmap, ts: Transform) {
    let mut pb = PathBuilder::new();
    let cx = 50.0;
    let cy = 50.0;
    let r_out = 44.0;
    let r_in = 18.0;
    let points = 5;

    for i in 0..(points * 2) {
        let angle =
            (i as f32) * std::f32::consts::PI / (points as f32) - std::f32::consts::FRAC_PI_2;
        let r = if i % 2 == 0 { r_out } else { r_in };
        let x = cx + angle.cos() * r;
        let y = cy + angle.sin() * r;
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.close();
    if let Some(path) = pb.finish() {
        // Outline shadow
        let shadow_ts = ts.pre_translate(1.5, 1.5);
        pixmap.fill_path(
            &path,
            &paint_fill(245, 127, 23, 255),
            FillRule::Winding,
            shadow_ts,
            None,
        );
        // Golden star fill
        pixmap.fill_path(
            &path,
            &paint_fill(255, 215, 0, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
}

fn draw_robot(pixmap: &mut Pixmap, ts: Transform) {
    // Antenna
    let mut pb = PathBuilder::new();
    pb.push_circle(50.0, 12.0, 5.0);
    pb.move_to(50.0, 17.0);
    pb.line_to(50.0, 28.0);
    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width: 4.0,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint_fill(120, 144, 156, 255), &stroke, ts, None);
    }

    // Head (Metallic Cyan-Gray)
    let mut pb = PathBuilder::new();
    add_rect(&mut pb, 20.0, 28.0, 60.0, 52.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(144, 164, 174, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Ears
    let mut pb = PathBuilder::new();
    add_rect(&mut pb, 12.0, 42.0, 8.0, 18.0);
    add_rect(&mut pb, 80.0, 42.0, 8.0, 18.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(176, 190, 197, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Glowing Eyes (Bright Cyan)
    let mut pb = PathBuilder::new();
    pb.push_circle(36.0, 48.0, 6.5);
    pb.push_circle(64.0, 48.0, 6.5);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(0, 229, 255, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Mouth / Grille
    let mut pb = PathBuilder::new();
    add_rect(&mut pb, 32.0, 66.0, 36.0, 6.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(55, 71, 79, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
}

fn draw_hundred(pixmap: &mut Pixmap, ts: Transform) {
    // Red 100 with double underline
    let mut pb = PathBuilder::new();
    // '1'
    pb.move_to(18.0, 32.0);
    pb.line_to(28.0, 22.0);
    pb.line_to(28.0, 68.0);
    pb.move_to(20.0, 68.0);
    pb.line_to(36.0, 68.0);

    // First '0'
    pb.push_circle(50.0, 45.0, 16.0);

    // Second '0'
    pb.push_circle(78.0, 45.0, 16.0);

    // Double underlines
    pb.move_to(14.0, 78.0);
    pb.line_to(86.0, 78.0);
    pb.move_to(14.0, 86.0);
    pb.line_to(86.0, 86.0);

    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width: 5.5,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint_fill(229, 28, 35, 255), &stroke, ts, None);
    }
}

fn draw_target(pixmap: &mut Pixmap, ts: Transform) {
    // Outer red ring
    let mut pb = PathBuilder::new();
    pb.push_circle(50.0, 50.0, 42.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(229, 28, 35, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
    // Inner white ring
    let mut pb = PathBuilder::new();
    pb.push_circle(50.0, 50.0, 30.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 255, 255, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
    // Inner red ring
    let mut pb = PathBuilder::new();
    pb.push_circle(50.0, 50.0, 18.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(229, 28, 35, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
    // Bullseye center (Yellow)
    let mut pb = PathBuilder::new();
    pb.push_circle(50.0, 50.0, 8.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 214, 0, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
}

fn draw_crown(pixmap: &mut Pixmap, ts: Transform) {
    // Golden crown with jewels
    let mut pb = PathBuilder::new();
    pb.move_to(15.0, 75.0);
    pb.line_to(12.0, 35.0);
    pb.line_to(32.0, 50.0);
    pb.line_to(50.0, 20.0);
    pb.line_to(68.0, 50.0);
    pb.line_to(88.0, 35.0);
    pb.line_to(85.0, 75.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 193, 7, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Jewels (Red, Cyan, Emerald)
    let mut pb = PathBuilder::new();
    pb.push_circle(12.0, 35.0, 4.0);
    pb.push_circle(50.0, 20.0, 5.0);
    pb.push_circle(88.0, 35.0, 4.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(233, 30, 99, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
}

fn draw_money_bag(pixmap: &mut Pixmap, ts: Transform) {
    // Green sack
    let mut pb = PathBuilder::new();
    pb.move_to(50.0, 30.0);
    pb.cubic_to(38.0, 30.0, 18.0, 45.0, 18.0, 70.0);
    pb.cubic_to(18.0, 90.0, 35.0, 95.0, 50.0, 95.0);
    pb.cubic_to(65.0, 95.0, 82.0, 90.0, 82.0, 70.0);
    pb.cubic_to(82.0, 45.0, 62.0, 30.0, 50.0, 30.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(76, 175, 80, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
    // Tie top
    let mut pb = PathBuilder::new();
    add_rect(&mut pb, 40.0, 22.0, 20.0, 9.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 193, 7, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
    // Dollar sign '$'
    let mut pb = PathBuilder::new();
    pb.move_to(50.0, 46.0);
    pb.line_to(50.0, 78.0);
    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width: 4.0,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint_fill(255, 235, 59, 255), &stroke, ts, None);
    }
}

fn draw_chart(pixmap: &mut Pixmap, ts: Transform) {
    // Axis line
    let mut pb = PathBuilder::new();
    pb.move_to(15.0, 15.0);
    pb.line_to(15.0, 85.0);
    pb.line_to(88.0, 85.0);
    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width: 4.0,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint_fill(120, 144, 156, 255), &stroke, ts, None);
    }
    // Trending green arrow
    let mut pb = PathBuilder::new();
    pb.move_to(20.0, 75.0);
    pb.line_to(42.0, 55.0);
    pb.line_to(58.0, 65.0);
    pb.line_to(82.0, 25.0);
    // Arrow head
    pb.line_to(68.0, 25.0);
    pb.move_to(82.0, 25.0);
    pb.line_to(82.0, 39.0);
    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width: 5.0,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint_fill(0, 200, 83, 255), &stroke, ts, None);
    }
}

fn draw_clapper(pixmap: &mut Pixmap, ts: Transform) {
    // Main slate (Dark Slate)
    let mut pb = PathBuilder::new();
    add_rect(&mut pb, 15.0, 35.0, 70.0, 50.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(38, 50, 56, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
    // Clapper top stick (Slanted black and white stripes)
    let mut pb = PathBuilder::new();
    add_rect(&mut pb, 12.0, 18.0, 76.0, 15.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 255, 255, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
}

fn draw_sparkles(pixmap: &mut Pixmap, ts: Transform) {
    // Main big 4-point sparkle
    let mut pb = PathBuilder::new();
    let cx = 45.0;
    let cy = 45.0;
    pb.move_to(cx, cy - 35.0);
    pb.cubic_to(cx + 3.0, cy - 8.0, cx + 8.0, cy - 3.0, cx + 35.0, cy);
    pb.cubic_to(cx + 8.0, cy + 3.0, cx + 3.0, cy + 8.0, cx, cy + 35.0);
    pb.cubic_to(cx - 3.0, cy + 8.0, cx - 8.0, cy + 3.0, cx - 35.0, cy);
    pb.cubic_to(cx - 8.0, cy - 3.0, cx - 3.0, cy - 8.0, cx, cy - 35.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 214, 0, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
    // Small sparkle
    let mut pb = PathBuilder::new();
    pb.push_circle(78.0, 25.0, 6.0);
    pb.push_circle(22.0, 78.0, 4.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 235, 59, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
}

fn draw_siren(pixmap: &mut Pixmap, ts: Transform) {
    // Red beacon dome
    let mut pb = PathBuilder::new();
    pb.move_to(25.0, 70.0);
    pb.cubic_to(25.0, 30.0, 75.0, 30.0, 75.0, 70.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(244, 67, 54, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
    // Base mount
    let mut pb = PathBuilder::new();
    add_rect(&mut pb, 20.0, 70.0, 60.0, 16.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(97, 97, 97, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
}

fn draw_thumbs_up(pixmap: &mut Pixmap, ts: Transform) {
    let mut pb = PathBuilder::new();
    pb.move_to(25.0, 45.0);
    pb.line_to(45.0, 45.0);
    pb.line_to(48.0, 18.0);
    pb.cubic_to(55.0, 18.0, 58.0, 30.0, 55.0, 45.0);
    pb.line_to(80.0, 45.0);
    pb.cubic_to(85.0, 45.0, 85.0, 55.0, 80.0, 58.0);
    pb.cubic_to(85.0, 58.0, 85.0, 68.0, 80.0, 70.0);
    pb.cubic_to(85.0, 70.0, 85.0, 80.0, 75.0, 82.0);
    pb.line_to(40.0, 82.0);
    pb.line_to(25.0, 65.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 202, 40, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
}

fn draw_diamond(pixmap: &mut Pixmap, ts: Transform) {
    let mut pb = PathBuilder::new();
    pb.move_to(25.0, 30.0);
    pb.line_to(75.0, 30.0);
    pb.line_to(88.0, 45.0);
    pb.line_to(50.0, 88.0);
    pb.line_to(12.0, 45.0);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(41, 182, 246, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }
    // Crown facet line
    let mut pb = PathBuilder::new();
    pb.move_to(12.0, 45.0);
    pb.line_to(88.0, 45.0);
    pb.move_to(38.0, 45.0);
    pb.line_to(50.0, 88.0);
    pb.move_to(62.0, 45.0);
    pb.line_to(50.0, 88.0);
    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width: 2.0,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint_fill(255, 255, 255, 200), &stroke, ts, None);
    }
}

fn draw_fallback_emoji(pixmap: &mut Pixmap, ts: Transform, _raw: &str) {
    // Cheerful yellow expressive emoji face
    let mut pb = PathBuilder::new();
    pb.push_circle(50.0, 50.0, 42.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(255, 214, 0, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Smiling eyes
    let mut pb = PathBuilder::new();
    pb.push_circle(36.0, 42.0, 5.0);
    pb.push_circle(64.0, 42.0, 5.0);
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint_fill(66, 66, 66, 255),
            FillRule::Winding,
            ts,
            None,
        );
    }

    // Smile curve
    let mut pb = PathBuilder::new();
    pb.move_to(32.0, 60.0);
    pb.cubic_to(40.0, 75.0, 60.0, 75.0, 68.0, 60.0);
    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width: 4.5,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint_fill(66, 66, 66, 255), &stroke, ts, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_emoji_char() {
        assert!(is_emoji_char('🔥'));
        assert!(is_emoji_char('🚀'));
        assert!(is_emoji_char('❤'));
        assert!(is_emoji_grapheme("❤️"));
        assert!(is_emoji_char('⚡'));
        assert!(!is_emoji_char('A'));
        assert!(!is_emoji_char('가'));
        assert!(!is_emoji_char('1'));
    }

    #[test]
    fn test_split_text_and_emojis() {
        let text = "오늘 대박 소식! 🔥 100배 성장 🚀 가자";
        let runs = split_text_and_emojis(text);

        assert_eq!(runs.len(), 5);
        assert_eq!(runs[0], TextRun::Text("오늘 대박 소식! ".into()));
        assert_eq!(runs[1], TextRun::Emoji("🔥".into()));
        assert_eq!(runs[2], TextRun::Text(" 100배 성장 ".into()));
        assert_eq!(runs[3], TextRun::Emoji("🚀".into()));
        assert_eq!(runs[4], TextRun::Text(" 가자".into()));
    }

    #[test]
    fn test_render_emoji() {
        let img = render_emoji("🔥", 64).expect("Failed to render fire emoji");
        assert_eq!(img.width(), 64);
        assert_eq!(img.height(), 64);

        // Verify non-zero alpha pixels exist
        let non_transparent = img.pixels().any(|p| p[3] > 0);
        assert!(non_transparent);
    }
}
