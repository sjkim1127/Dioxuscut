//! CPU rasterizer backend using `tiny-skia`.
//!
//! Renders a [`Scene`] into an RGBA pixel buffer without any GPU or browser dependency.

use crate::backend::{FrameConfig, RasterError, RasterizerBackend};
use crate::font::FontCache;
use crate::scene::{Color, SceneNode, Scene};
use image::RgbaImage;
use tiny_skia::{
    FillRule, Paint, Path, PathBuilder, Pixmap, Rect,
    Stroke, Transform,
};

/// The `tiny-skia` CPU rasterizer.
///
/// Zero dependencies on GPU drivers, Chrome, or any external process.
/// Works in CI, Docker, and serverless environments out of the box.
/// Text is rendered using real TTF glyph data via `ab_glyph`.
pub struct TinySkiaBackend {
    font: FontCache,
}

impl TinySkiaBackend {
    /// Create a new backend, loading a system font automatically.
    pub fn new() -> Self {
        Self { font: FontCache::load() }
    }

    /// Create without loading a font (text will use placeholder blocks).
    pub fn headless() -> Self {
        Self { font: FontCache::headless() }
    }
}

impl Default for TinySkiaBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RasterizerBackend for TinySkiaBackend {
    fn render_frame(&self, scene: &Scene, config: &FrameConfig) -> Result<RgbaImage, RasterError> {
        let mut pixmap = Pixmap::new(config.width, config.height)
            .ok_or_else(|| RasterError::Init("Failed to create Pixmap — invalid dimensions".into()))?;

        // Clear to transparent
        pixmap.fill(tiny_skia::Color::TRANSPARENT);

        render_nodes(&mut pixmap, &scene.nodes, Transform::identity(), 1.0, &self.font);

        // Convert tiny-skia Pixmap (RGBA premultiplied) to image::RgbaImage
        let raw_data = pixmap.data().to_vec();
        RgbaImage::from_raw(config.width, config.height, raw_data)
            .ok_or_else(|| RasterError::ImageEncode("Failed to build RgbaImage from pixel data".into()))
    }
}

fn render_nodes(pixmap: &mut Pixmap, nodes: &[SceneNode], parent_transform: Transform, parent_opacity: f32, font: &FontCache) {
    for node in nodes {
        render_node(pixmap, node, parent_transform, parent_opacity, font);
    }
}

fn render_node(pixmap: &mut Pixmap, node: &SceneNode, transform: Transform, opacity: f32, font: &FontCache) {
    match node {
        SceneNode::Rect { x, y, w, h, fill, stroke, stroke_width, corner_radius } => {
            let rect = match Rect::from_xywh(*x, *y, *w, *h) {
                Some(r) => r,
                None => return,
            };

            let path = if *corner_radius > 0.0 {
                build_rounded_rect(*x, *y, *w, *h, *corner_radius)
            } else {
                PathBuilder::from_rect(rect)
            };

            // Fill
            let mut paint = Paint::default();
            paint.set_color(apply_opacity(*fill, opacity));
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, transform, None);

            // Stroke
            if let (Some(stroke_color), sw) = (stroke, stroke_width) {
                if *sw > 0.0 {
                    let mut stroke_paint = Paint::default();
                    stroke_paint.set_color(apply_opacity(*stroke_color, opacity));
                    stroke_paint.anti_alias = true;
                    let stroke = Stroke { width: *sw, ..Default::default() };
                    pixmap.stroke_path(&path, &stroke_paint, &stroke, transform, None);
                }
            }
        }

        SceneNode::Circle { cx, cy, r, fill, stroke, stroke_width } => {
            let path = build_circle(*cx, *cy, *r);

            let mut paint = Paint::default();
            paint.set_color(apply_opacity(*fill, opacity));
            paint.anti_alias = true;
            pixmap.fill_path(&path, &paint, FillRule::Winding, transform, None);

            if let (Some(stroke_color), sw) = (stroke, stroke_width) {
                if *sw > 0.0 {
                    let mut stroke_paint = Paint::default();
                    stroke_paint.set_color(apply_opacity(*stroke_color, opacity));
                    stroke_paint.anti_alias = true;
                    let stroke = Stroke { width: *sw, ..Default::default() };
                    pixmap.stroke_path(&path, &stroke_paint, &stroke, transform, None);
                }
            }
        }

        SceneNode::Path { d, fill, stroke, stroke_width, opacity: node_opacity } => {
            let combined_opacity = opacity * node_opacity;

            if let Some(path) = svgpath_to_tiny_skia(d) {
                if let Some(fill_color) = fill {
                    let mut paint = Paint::default();
                    paint.set_color(apply_opacity(*fill_color, combined_opacity));
                    paint.anti_alias = true;
                    pixmap.fill_path(&path, &paint, FillRule::Winding, transform, None);
                }

                if let (Some(stroke_color), sw) = (stroke, stroke_width) {
                    if *sw > 0.0 {
                        let mut stroke_paint = Paint::default();
                        stroke_paint.set_color(apply_opacity(*stroke_color, combined_opacity));
                        stroke_paint.anti_alias = true;
                        let stroke = Stroke { width: *sw, ..Default::default() };
                        pixmap.stroke_path(&path, &stroke_paint, &stroke, transform, None);
                    }
                }
            }
        }

        SceneNode::LinearGradient { x, y, w, h, angle_deg, stops } => {
            if stops.is_empty() { return; }

            let rect = match Rect::from_xywh(*x, *y, *w, *h) {
                Some(r) => r,
                None => return,
            };
            let path = PathBuilder::from_rect(rect);

            // Compute gradient endpoints from angle
            let angle_rad = angle_deg.to_radians();
            let cx = x + w / 2.0;
            let cy = y + h / 2.0;
            let half_diag = (w * w + h * h).sqrt() / 2.0;

            let x1 = cx - angle_rad.sin() * half_diag;
            let y1 = cy - angle_rad.cos() * half_diag;
            let x2 = cx + angle_rad.sin() * half_diag;
            let y2 = cy + angle_rad.cos() * half_diag;

            let sk_stops: Vec<tiny_skia::GradientStop> = stops.iter().map(|s| {
                tiny_skia::GradientStop::new(s.position, apply_opacity(s.color, opacity))
            }).collect();

            if let Some(shader) = tiny_skia::LinearGradient::new(
                tiny_skia::Point::from_xy(x1, y1),
                tiny_skia::Point::from_xy(x2, y2),
                sk_stops,
                tiny_skia::SpreadMode::Pad,
                Transform::identity(),
            ) {
                let mut paint = Paint::default();
                paint.shader = shader;
                paint.anti_alias = true;
                pixmap.fill_path(&path, &paint, FillRule::Winding, transform, None);
            }
        }

        SceneNode::RadialGradient { cx, cy, r, stops } => {
            if stops.is_empty() { return; }

            let path = build_circle(*cx, *cy, *r);

            let sk_stops: Vec<tiny_skia::GradientStop> = stops.iter().map(|s| {
                tiny_skia::GradientStop::new(s.position, apply_opacity(s.color, opacity))
            }).collect();

            if let Some(shader) = tiny_skia::RadialGradient::new(
                tiny_skia::Point::from_xy(*cx, *cy),
                tiny_skia::Point::from_xy(*cx, *cy),
                *r,
                sk_stops,
                tiny_skia::SpreadMode::Pad,
                Transform::identity(),
            ) {
                let mut paint = Paint::default();
                paint.shader = shader;
                paint.anti_alias = true;
                pixmap.fill_path(&path, &paint, FillRule::Winding, transform, None);
            }
        }

        SceneNode::Group { transform: group_transform, opacity: group_opacity, children } => {
            let new_transform = transform.post_concat(group_transform.to_tiny_skia());
            let new_opacity = opacity * group_opacity;
            render_nodes(pixmap, children, new_transform, new_opacity, font);
        }

        SceneNode::Text { x, y, content, font_size, color, .. } => {
            let text_color = apply_opacity(*color, opacity);

            if let Some(rendered) = font.rasterize(content, *font_size) {
                // Blit the glyph coverage map onto the pixmap at (x, y - baseline)
                let origin_x = x.floor() as i32;
                let origin_y = (*y - rendered.baseline as f32).floor() as i32;

                let pw = pixmap.width() as i32;
                let ph = pixmap.height() as i32;
                let pixmap_width = pixmap.width(); // cache before mutable borrow

                let pixels_rgba = pixmap.pixels_mut();

                for gy in 0..rendered.height {
                    for gx in 0..rendered.width {
                        let coverage = rendered.pixels[(gy * rendered.width + gx) as usize];
                        if coverage == 0 { continue; }

                        let px = origin_x + gx as i32;
                        let py = origin_y + gy as i32;
                        if px < 0 || py < 0 || px >= pw || py >= ph { continue; }

                        let idx = (py as u32 * pixmap_width + px as u32) as usize;
                        // Alpha-composite glyph pixel over existing pixel
                        let src_a = (coverage as f32 / 255.0)
                            * (text_color.alpha() as f32 / 255.0);
                        let dst = pixels_rgba[idx];
                        let dst_a = dst.alpha() as f32 / 255.0;
                        let out_a = src_a + dst_a * (1.0 - src_a);
                        if out_a > 0.0 {
                            let blend = |src_c: f32, dst_c: f32| -> u8 {
                                ((src_c * src_a + dst_c * dst_a * (1.0 - src_a)) / out_a * 255.0)
                                    .clamp(0.0, 255.0) as u8
                            };
                            let r = blend(text_color.red() as f32,   dst.red() as f32);
                            let g = blend(text_color.green() as f32, dst.green() as f32);
                            let b = blend(text_color.blue() as f32,  dst.blue() as f32);
                            let a = (out_a * 255.0).clamp(0.0, 255.0) as u8;
                            pixels_rgba[idx] = tiny_skia::PremultipliedColorU8::from_rgba(r, g, b, a)
                                .unwrap_or(pixels_rgba[idx]);
                        }
                    }
                }
            } else {
                // Fallback: draw a solid colour block per character (no font loaded)
                let char_w = *font_size * 0.6;
                let mut cx = *x;
                for _ch in content.chars() {
                    let rect = match Rect::from_xywh(cx, *y - font_size, char_w.max(1.0), font_size.max(1.0)) {
                        Some(r) => r,
                        None => { cx += char_w; continue; },
                    };
                    let path = PathBuilder::from_rect(rect);
                    let mut paint = Paint::default();
                    paint.set_color(text_color);
                    paint.anti_alias = true;
                    pixmap.fill_path(&path, &paint, FillRule::Winding, transform, None);
                    cx += char_w;
                }
            }
        }
    }
}

// --- Helpers ---

fn apply_opacity(color: Color, opacity: f32) -> tiny_skia::Color {
    let a = (color.a as f32 * opacity.clamp(0.0, 1.0)) as u8;
    tiny_skia::Color::from_rgba8(color.r, color.g, color.b, a)
}

fn build_circle(cx: f32, cy: f32, r: f32) -> Path {
    let mut pb = PathBuilder::new();
    // Approximate circle with 4 cubic bezier curves (standard approximation)
    let k = 0.5522847498 * r;
    pb.move_to(cx, cy - r);
    pb.cubic_to(cx + k, cy - r, cx + r, cy - k, cx + r, cy);
    pb.cubic_to(cx + r, cy + k, cx + k, cy + r, cx, cy + r);
    pb.cubic_to(cx - k, cy + r, cx - r, cy + k, cx - r, cy);
    pb.cubic_to(cx - r, cy - k, cx - k, cy - r, cx, cy - r);
    pb.close();
    pb.finish().unwrap_or_else(|| PathBuilder::new().finish().unwrap())
}

fn build_rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let r = r.min(w / 2.0).min(h / 2.0);
    let k = 0.5522847498 * r;
    let mut pb = PathBuilder::new();

    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();

    pb.finish().unwrap_or_else(|| PathBuilder::from_rect(Rect::from_xywh(x, y, w, h).unwrap()).into())
}

/// Minimal SVG `d` attribute parser → tiny-skia PathBuilder.
/// Supports M, L, H, V, C, Q, Z commands (absolute only).
fn svgpath_to_tiny_skia(d: &str) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let tokens = tokenize_path(d);
    let mut pos = 0usize;

    let mut cx = 0.0f32;
    let mut cy = 0.0f32;

    while pos < tokens.len() {
        match tokens[pos].as_str() {
            "M" => {
                pos += 1;
                let x = parse_f32(&tokens, &mut pos)?;
                let y = parse_f32(&tokens, &mut pos)?;
                pb.move_to(x, y);
                cx = x; cy = y;
            }
            "L" => {
                pos += 1;
                let x = parse_f32(&tokens, &mut pos)?;
                let y = parse_f32(&tokens, &mut pos)?;
                pb.line_to(x, y);
                cx = x; cy = y;
            }
            "H" => {
                pos += 1;
                let x = parse_f32(&tokens, &mut pos)?;
                pb.line_to(x, cy);
                cx = x;
            }
            "V" => {
                pos += 1;
                let y = parse_f32(&tokens, &mut pos)?;
                pb.line_to(cx, y);
                cy = y;
            }
            "C" => {
                pos += 1;
                let x1 = parse_f32(&tokens, &mut pos)?;
                let y1 = parse_f32(&tokens, &mut pos)?;
                let x2 = parse_f32(&tokens, &mut pos)?;
                let y2 = parse_f32(&tokens, &mut pos)?;
                let x  = parse_f32(&tokens, &mut pos)?;
                let y  = parse_f32(&tokens, &mut pos)?;
                pb.cubic_to(x1, y1, x2, y2, x, y);
                cx = x; cy = y;
            }
            "Q" => {
                pos += 1;
                let x1 = parse_f32(&tokens, &mut pos)?;
                let y1 = parse_f32(&tokens, &mut pos)?;
                let x  = parse_f32(&tokens, &mut pos)?;
                let y  = parse_f32(&tokens, &mut pos)?;
                pb.quad_to(x1, y1, x, y);
                cx = x; cy = y;
            }
            "Z" | "z" => {
                pb.close();
                pos += 1;
            }
            _ => { pos += 1; }
        }
    }

    pb.finish()
}

fn tokenize_path(d: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for c in d.chars() {
        if c.is_alphabetic() {
            if !current.trim().is_empty() {
                tokens.push(current.trim().to_string());
                current = String::new();
            }
            tokens.push(c.to_string());
        } else if c == ',' || c.is_whitespace() {
            if !current.trim().is_empty() {
                tokens.push(current.trim().to_string());
                current = String::new();
            }
        } else {
            current.push(c);
        }
    }
    if !current.trim().is_empty() {
        tokens.push(current.trim().to_string());
    }
    tokens
}

fn parse_f32(tokens: &[String], pos: &mut usize) -> Option<f32> {
    if *pos >= tokens.len() { return None; }
    let v = tokens[*pos].parse::<f32>().ok()?;
    *pos += 1;
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::FrameConfig;
    use crate::scene::{Color, Scene, SceneNode};

    fn render(scene: &Scene, w: u32, h: u32) -> RgbaImage {
        let backend = TinySkiaBackend::new();
        let config = FrameConfig::new(w, h, 0, 30.0);
        backend.render_frame(scene, &config).expect("render failed")
    }

    #[test]
    fn test_solid_red_rect() {
        let mut scene = Scene::new();
        scene.push(SceneNode::Rect {
            x: 0.0, y: 0.0, w: 100.0, h: 100.0,
            fill: Color::rgb(255, 0, 0),
            stroke: None, stroke_width: 0.0, corner_radius: 0.0,
        });

        let img = render(&scene, 100, 100);
        let px = img.get_pixel(50, 50);
        assert_eq!(px[0], 255, "Red channel should be 255");
        assert_eq!(px[1], 0,   "Green channel should be 0");
        assert_eq!(px[2], 0,   "Blue channel should be 0");
    }

    #[test]
    fn test_circle_center_pixel() {
        let mut scene = Scene::new();
        scene.push(SceneNode::Circle {
            cx: 100.0, cy: 100.0, r: 80.0,
            fill: Color::rgb(0, 0, 255),
            stroke: None, stroke_width: 0.0,
        });

        let img = render(&scene, 200, 200);
        let px = img.get_pixel(100, 100);
        assert_eq!(px[2], 255, "Blue channel at circle center should be 255");
    }

    #[test]
    fn test_transparent_background() {
        let scene = Scene::new(); // empty
        let img = render(&scene, 64, 64);
        let px = img.get_pixel(32, 32);
        assert_eq!(px[3], 0, "Empty scene should be fully transparent");
    }

    #[test]
    fn test_linear_gradient_renders() {
        use crate::scene::GradientStop;
        let mut scene = Scene::new();
        scene.push(SceneNode::LinearGradient {
            x: 0.0, y: 0.0, w: 200.0, h: 100.0,
            angle_deg: 90.0,
            stops: vec![
                GradientStop { position: 0.0, color: Color::rgb(255, 0, 0) },
                GradientStop { position: 1.0, color: Color::rgb(0, 0, 255) },
            ],
        });

        let img = render(&scene, 200, 100);
        // Left pixel should be more red than blue
        let left = img.get_pixel(5, 50);
        let right = img.get_pixel(195, 50);
        assert!(left[0] > left[2], "Left side of gradient should be redder");
        assert!(right[2] > right[0], "Right side of gradient should be bluer");
    }

    #[test]
    fn test_group_with_opacity() {
        let mut scene = Scene::new();
        scene.push(SceneNode::Group {
            transform: Default::default(),
            opacity: 0.5,
            children: vec![
                SceneNode::Rect {
                    x: 10.0, y: 10.0, w: 80.0, h: 80.0,
                    fill: Color::rgb(255, 255, 255),
                    stroke: None, stroke_width: 0.0, corner_radius: 0.0,
                },
            ],
        });

        let img = render(&scene, 100, 100);
        let px = img.get_pixel(50, 50);
        // At 50% opacity over transparent, alpha should be ~127
        assert!(px[3] > 50 && px[3] < 200, "Group opacity should reduce alpha");
    }
}
