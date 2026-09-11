//! HarfBuzz text shaping using `rustybuzz`.

use crate::backend::RasterError;
use crate::text_layout::bidi::BidiRun;
use crate::text_layout::font_resolver::FontResolver;
use crate::text_layout::style::TextStyle;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// A single shaped glyph resulting from OpenType shaping.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedGlyph {
    /// Index of the font in the `FontResolver` chain.
    pub font_index: usize,
    /// OpenType glyph ID within the font.
    pub glyph_id: u16,
    /// Horizontal pen advance in canvas pixels.
    pub x_advance: f32,
    /// Vertical pen advance in canvas pixels.
    pub y_advance: f32,
    /// Horizontal subpixel positioning offset from pen position.
    pub x_offset: f32,
    /// Vertical subpixel positioning offset from baseline (positive = down).
    pub y_offset: f32,
    /// Source text byte offset of the cluster that produced this glyph.
    pub cluster: u32,
}

/// A shaped run of glyphs using a single font with uniform directionality.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedRun {
    pub font_index: usize,
    pub is_rtl: bool,
    pub glyphs: Vec<ShapedGlyph>,
    pub width: f32,
    pub source_range: Range<usize>,
}

/// Shapes a segmented list of bidirectional runs using the font chain.
pub fn shape_bidi_runs(
    text: &str,
    bidi_runs: &[BidiRun],
    font_resolver: &FontResolver,
    style: &TextStyle,
) -> Result<Vec<ShapedRun>, RasterError> {
    let fonts = font_resolver.fonts();
    if fonts.is_empty() {
        return Err(RasterError::Scene("no fonts available for shaping".into()));
    }

    let mut shaped_runs = Vec::new();

    for bidi_run in bidi_runs {
        let run_text = &text[bidi_run.range.clone()];
        if run_text.is_empty() {
            continue;
        }

        // Subdivide bidi run into font-consistent segments
        let mut font_segments: Vec<(usize, usize, usize)> = Vec::new();
        for (grapheme_start, grapheme) in run_text.grapheme_indices(true) {
            let font_index = font_resolver.resolve_font_for_grapheme(grapheme);
            let grapheme_end = grapheme_start + grapheme.len();

            if let Some((last_font, _, last_end)) = font_segments.last_mut() {
                if *last_font == font_index && *last_end == grapheme_start {
                    *last_end = grapheme_end;
                    continue;
                }
            }
            font_segments.push((font_index, grapheme_start, grapheme_end));
        }

        for (font_index, seg_start, seg_end) in font_segments {
            let seg_text = &run_text[seg_start..seg_end];
            let font = &fonts[font_index];

            let face = rustybuzz::Face::from_slice(font.data.as_slice(), 0).ok_or_else(|| {
                RasterError::Scene(format!(
                    "font index {font_index} could not be loaded into shaping engine"
                ))
            })?;

            let units_per_em = (face.units_per_em() as f32).max(1.0);
            let unit_scale = style.font_size / units_per_em;

            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(seg_text);

            if bidi_run.is_rtl {
                buffer.set_direction(rustybuzz::Direction::RightToLeft);
            } else {
                buffer.set_direction(rustybuzz::Direction::LeftToRight);
            }
            buffer.guess_segment_properties();

            let glyph_buffer = rustybuzz::shape(&face, &[], buffer);
            let mut glyphs = Vec::new();
            let mut run_width = 0.0_f32;

            for (info, pos) in glyph_buffer
                .glyph_infos()
                .iter()
                .zip(glyph_buffer.glyph_positions())
            {
                let glyph_id = info.glyph_id as u16;
                let x_advance = (pos.x_advance as f32 * unit_scale) + style.letter_spacing;
                let y_advance = pos.y_advance as f32 * unit_scale;
                let x_offset = pos.x_offset as f32 * unit_scale;
                let y_offset = -(pos.y_offset as f32 * unit_scale);
                let cluster = (bidi_run.range.start + seg_start) as u32 + info.cluster;

                run_width += x_advance;
                glyphs.push(ShapedGlyph {
                    font_index,
                    glyph_id,
                    x_advance,
                    y_advance,
                    x_offset,
                    y_offset,
                    cluster,
                });
            }

            let abs_range = (bidi_run.range.start + seg_start)..(bidi_run.range.start + seg_end);
            shaped_runs.push(ShapedRun {
                font_index,
                is_rtl: bidi_run.is_rtl,
                glyphs,
                width: run_width,
                source_range: abs_range,
            });
        }
    }

    Ok(shaped_runs)
}
