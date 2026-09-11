//! Kinetic typography and text animation utilities.
//!
//! Provides Remotion-compatible text motion effects:
//! - Typewriter character-by-character and word-by-word reveal
//! - Cyberpunk / matrix text scrambling and progressive decoding

const SCRAMBLE_GLYPHS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789#@$%&*!?/\\+=~<>";

/// Renders a typewriter effect revealing characters based on `progress` in `[0.0, 1.0]`.
///
/// If `show_cursor` is true and `progress < 1.0`, `cursor_char` is appended to the visible slice.
pub fn typewriter_text(text: &str, progress: f64, show_cursor: bool, cursor_char: char) -> String {
    let progress = progress.clamp(0.0, 1.0);
    let chars: Vec<char> = text.chars().collect();
    let total_chars = chars.len();

    if total_chars == 0 {
        return String::new();
    }

    let visible_count = ((total_chars as f64) * progress).round() as usize;
    let visible_count = visible_count.min(total_chars);

    let mut result: String = chars[..visible_count].iter().collect();

    if show_cursor && progress < 1.0 {
        result.push(cursor_char);
    }

    result
}

/// Renders a word-by-word typewriter reveal effect.
pub fn typewriter_words(text: &str, progress: f64) -> String {
    let progress = progress.clamp(0.0, 1.0);
    let words: Vec<&str> = text.split_whitespace().collect();
    let total_words = words.len();

    if total_words == 0 {
        return String::new();
    }

    let visible_count = ((total_words as f64) * progress).round() as usize;
    let visible_count = visible_count.min(total_words);

    words[..visible_count].join(" ")
}

/// Renders a cyberpunk/matrix scrambled text decoding effect.
///
/// Characters ahead of the reveal progress are replaced by pseudorandom glyphs
/// generated from `seed`, while whitespace characters are preserved.
pub fn scramble_text(target_text: &str, progress: f64, seed: u64) -> String {
    let progress = progress.clamp(0.0, 1.0);
    if progress >= 1.0 {
        return target_text.to_string();
    }

    let chars: Vec<char> = target_text.chars().collect();
    let total_chars = chars.len();
    if total_chars == 0 {
        return String::new();
    }

    let decoded_count = ((total_chars as f64) * progress).floor() as usize;

    let mut output = String::with_capacity(target_text.len());

    for (i, &ch) in chars.iter().enumerate() {
        if i < decoded_count || ch.is_whitespace() {
            output.push(ch);
        } else {
            // Pseudorandom glyph selection based on seed, char position, and time step
            let step = (progress * 50.0) as u64;
            let hash = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add((i as u64).wrapping_mul(1442695040888963407))
                .wrapping_add(step.wrapping_mul(1013904223));
            let glyph_idx = (hash as usize) % SCRAMBLE_GLYPHS.len();
            output.push(SCRAMBLE_GLYPHS[glyph_idx] as char);
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typewriter_progress() {
        let text = "Hello";
        assert_eq!(typewriter_text(text, 0.0, false, '|'), "");
        assert_eq!(typewriter_text(text, 0.0, true, '|'), "|");
        assert_eq!(typewriter_text(text, 0.4, false, '|'), "He");
        assert_eq!(typewriter_text(text, 1.0, true, '|'), "Hello");
    }

    #[test]
    fn test_typewriter_words() {
        let text = "The quick brown fox";
        assert_eq!(typewriter_words(text, 0.0), "");
        assert_eq!(typewriter_words(text, 0.25), "The");
        assert_eq!(typewriter_words(text, 0.5), "The quick");
        assert_eq!(typewriter_words(text, 0.75), "The quick brown");
        assert_eq!(typewriter_words(text, 1.0), "The quick brown fox");
    }

    #[test]
    fn test_scramble_text() {
        let text = "HELLO WORLD";
        let half = scramble_text(text, 0.5, 42);
        assert_eq!(half.chars().count(), text.chars().count());
        // Whitespace preserved
        assert_eq!(half.chars().nth(5).unwrap(), ' ');
        // First characters decoded
        assert!(half.starts_with("HELLO"));

        let full = scramble_text(text, 1.0, 42);
        assert_eq!(full, text);
    }
}
