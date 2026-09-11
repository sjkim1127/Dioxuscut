//! UAX #9 Bidirectional text segmentation and analysis.

use crate::text_layout::style::TextDirection;
use std::ops::Range;
use unicode_bidi::{BidiInfo, Level};

/// A visually ordered contiguous run of text with uniform directionality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiRun {
    /// Byte range within the source text.
    pub range: Range<usize>,
    /// Resolved embedding level (even = LTR, odd = RTL).
    pub level: u8,
    /// Whether this run should be shaped and rendered Right-to-Left.
    pub is_rtl: bool,
}

/// Checks if a string contains any Right-to-Left characters (Arabic, Hebrew, etc.).
pub fn contains_rtl(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{0590}'..='\u{05FF}' // Hebrew
            | '\u{0600}'..='\u{06FF}' // Arabic
            | '\u{0700}'..='\u{074F}' // Syriac
            | '\u{0750}'..='\u{077F}' // Arabic Supplement
            | '\u{0780}'..='\u{07BF}' // Thaana
            | '\u{08A0}'..='\u{08FF}' // Arabic Extended-A
            | '\u{FB1D}'..='\u{FB4F}' // Hebrew Presentation Forms
            | '\u{FB50}'..='\u{FDFF}' // Arabic Presentation Forms-A
            | '\u{FE70}'..='\u{FEFF}' // Arabic Presentation Forms-B
        )
    })
}

/// Segments text into visual bidirectional runs conforming to Unicode Annex #9.
pub fn segment_bidi_runs(text: &str, direction: TextDirection) -> Vec<BidiRun> {
    if text.is_empty() {
        return Vec::new();
    }

    // Fast-path for pure Left-to-Right text with non-RTL explicit direction
    if direction != TextDirection::Rtl && !contains_rtl(text) {
        return vec![BidiRun {
            range: 0..text.len(),
            level: 0,
            is_rtl: false,
        }];
    }

    let default_level = match direction {
        TextDirection::Ltr => Some(Level::ltr()),
        TextDirection::Rtl => Some(Level::rtl()),
        TextDirection::Auto => None,
    };

    let bidi_info = BidiInfo::new(text, default_level);
    let mut runs = Vec::new();

    for para in &bidi_info.paragraphs {
        let (_levels, visual_runs) = bidi_info.visual_runs(para, para.range.clone());
        for run_range in visual_runs {
            let level = bidi_info.levels[run_range.start];
            runs.push(BidiRun {
                is_rtl: level.is_rtl(),
                level: level.number(),
                range: run_range,
            });
        }
    }

    runs
}

/// Returns the Unicode mirrored character for paired punctuation in RTL text.
pub fn mirror_char(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        '‹' => '›',
        '›' => '‹',
        '“' => '”',
        '”' => '“',
        '‘' => '’',
        '’' => '‘',
        _ => c,
    }
}
