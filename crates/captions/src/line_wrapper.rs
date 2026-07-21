//! Line wrapping and token character constraint enforcement.

use crate::types::CaptionToken;

/// Ensures that no single subtitle caption token exceeds `max_chars` by splitting long words if needed.
pub fn ensure_max_characters_per_line(
    tokens: &[CaptionToken],
    max_chars: usize,
) -> Vec<CaptionToken> {
    if max_chars == 0 {
        return tokens.to_vec();
    }

    let mut result = Vec::new();

    for token in tokens {
        if token.text.chars().count() <= max_chars {
            result.push(token.clone());
        } else {
            // Split long token into smaller sub-chunks
            let chars: Vec<char> = token.text.chars().collect();
            let chunks: Vec<String> = chars
                .chunks(max_chars)
                .map(|c| c.iter().collect())
                .collect();

            let total_dur = token.end_ms.saturating_sub(token.start_ms);
            let chunk_dur = (total_dur as f64 / chunks.len() as f64).max(1.0);

            for (idx, chunk) in chunks.into_iter().enumerate() {
                let start = token.start_ms + (idx as f64 * chunk_dur) as u64;
                let end = if idx == chunk.len() - 1 {
                    token.end_ms
                } else {
                    token.start_ms + ((idx + 1) as f64 * chunk_dur) as u64
                };

                result.push(CaptionToken::new(chunk, start, end));
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ensure_max_characters_per_line() {
        let tokens = vec![CaptionToken::new(
            "Supercalifragilisticexpialidocious",
            0,
            1000,
        )];

        let wrapped = ensure_max_characters_per_line(&tokens, 10);
        assert!(wrapped.len() > 1);
        assert!(wrapped.iter().all(|t| t.text.chars().count() <= 10));
    }
}
