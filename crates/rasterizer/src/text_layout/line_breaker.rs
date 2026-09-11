//! UAX #14 Line breaking and word wrapping algorithm.

use crate::text_layout::shaper::{ShapedGlyph, ShapedRun};
use unicode_linebreak::{linebreaks, BreakOpportunity};

/// A single formatted line containing shaped glyphs.
#[derive(Debug, Clone, PartialEq)]
pub struct LineGlyphs {
    pub glyphs: Vec<ShapedGlyph>,
    pub width: f32,
    pub is_rtl: bool,
}

/// Breaks shaped runs into lines based on `max_width` and UAX #14 rules.
pub fn break_lines(
    text: &str,
    shaped_runs: &[ShapedRun],
    max_width: Option<f32>,
    max_lines: Option<usize>,
) -> Vec<LineGlyphs> {
    if text.is_empty() || shaped_runs.is_empty() {
        return Vec::new();
    }

    // Collect all break opportunities from UAX #14
    let breaks: Vec<(usize, BreakOpportunity)> = linebreaks(text).collect();

    // Flatten glyphs while retaining run metadata
    let mut all_glyphs: Vec<ShapedGlyph> = Vec::new();
    for run in shaped_runs {
        all_glyphs.extend(run.glyphs.clone());
    }

    let mut lines = Vec::new();
    let mut current_line_glyphs = Vec::new();
    let mut current_line_width = 0.0_f32;

    let mut last_break_idx = 0_usize;
    let mut glyph_idx = 0_usize;

    while glyph_idx < all_glyphs.len() {
        let glyph = &all_glyphs[glyph_idx];
        let cluster = glyph.cluster as usize;

        // Check if we crossed a mandatory line break
        let has_mandatory_break = breaks.iter().any(|&(b_idx, opp)| {
            opp == BreakOpportunity::Mandatory && b_idx > last_break_idx && b_idx <= cluster
        });

        if has_mandatory_break && !current_line_glyphs.is_empty() {
            lines.push(LineGlyphs {
                width: current_line_width,
                is_rtl: false,
                glyphs: std::mem::take(&mut current_line_glyphs),
            });
            current_line_width = 0.0;
            last_break_idx = cluster;

            if let Some(max_l) = max_lines {
                if lines.len() >= max_l {
                    break;
                }
            }
        }

        // Check soft wrapping against max_width
        if let Some(max_w) = max_width {
            let next_width = current_line_width + glyph.x_advance;
            if next_width > max_w && !current_line_glyphs.is_empty() {
                let break_candidate = breaks
                    .iter()
                    .rfind(|&&(b_idx, _)| b_idx > last_break_idx && b_idx <= cluster);

                if let Some(&(break_cluster, _)) = break_candidate {
                    // Split current_line_glyphs at break_cluster
                    let split_pos = current_line_glyphs
                        .iter()
                        .position(|g| g.cluster as usize >= break_cluster);

                    if let Some(pos) = split_pos {
                        if pos > 0 {
                            let remaining = current_line_glyphs.split_off(pos);
                            let line_w = current_line_glyphs.iter().map(|g| g.x_advance).sum();

                            lines.push(LineGlyphs {
                                width: line_w,
                                is_rtl: false,
                                glyphs: std::mem::take(&mut current_line_glyphs),
                            });

                            current_line_glyphs = remaining;
                            current_line_width =
                                current_line_glyphs.iter().map(|g| g.x_advance).sum();
                            last_break_idx = break_cluster;

                            if let Some(max_l) = max_lines {
                                if lines.len() >= max_l {
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    // Emergency break: current word itself exceeds max_w
                    lines.push(LineGlyphs {
                        width: current_line_width,
                        is_rtl: false,
                        glyphs: std::mem::take(&mut current_line_glyphs),
                    });
                    current_line_width = 0.0;
                    last_break_idx = cluster;

                    if let Some(max_l) = max_lines {
                        if lines.len() >= max_l {
                            break;
                        }
                    }
                }
            }
        }

        current_line_width += glyph.x_advance;
        current_line_glyphs.push(glyph.clone());
        glyph_idx += 1;
    }

    if !current_line_glyphs.is_empty() {
        lines.push(LineGlyphs {
            width: current_line_width,
            is_rtl: false,
            glyphs: current_line_glyphs,
        });
    }

    lines
}
