//! Whisper JSON and WebVTT subtitle transcription parsers.

use crate::types::CaptionToken;
use serde::Deserialize;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum SubtitleError {
    #[error("Failed to parse JSON: {0}")]
    JsonParse(String),
    #[error("Invalid timestamp: {0}")]
    InvalidTimestamp(String),
    #[error("Empty transcription data")]
    Empty,
}

#[derive(Deserialize)]
struct WhisperWord {
    word: Option<String>,
    text: Option<String>,
    start: Option<f64>,
    end: Option<f64>,
}

#[derive(Deserialize)]
struct WhisperSegment {
    words: Option<Vec<WhisperWord>>,
    text: Option<String>,
    start: Option<f64>,
    end: Option<f64>,
}

#[derive(Deserialize)]
struct WhisperVerboseResponse {
    segments: Option<Vec<WhisperSegment>>,
}

/// Parses OpenAI / Faster-Whisper transcription JSON into a list of word-level [`CaptionToken`]s.
///
/// Supports:
/// 1. Verbose response with `segments` and nested `words`.
/// 2. Direct list of word objects `[{"word": "Hello", "start": 0.0, "end": 0.5}, ...]`.
/// 3. Segments with text and segment-level timestamps if words are not present.
pub fn parse_whisper_json(json_str: &str) -> Result<Vec<CaptionToken>, SubtitleError> {
    let trimmed = json_str.trim();
    if trimmed.is_empty() {
        return Err(SubtitleError::Empty);
    }

    // Try parsing as direct word array
    if let Ok(words) = serde_json::from_str::<Vec<WhisperWord>>(trimmed) {
        let tokens = extract_tokens_from_words(&words);
        if !tokens.is_empty() {
            return Ok(tokens);
        }
    }

    // Try parsing as verbose object { "segments": [...] }
    if let Ok(verbose) = serde_json::from_str::<WhisperVerboseResponse>(trimmed) {
        if let Some(segments) = verbose.segments {
            let mut all_tokens = Vec::new();
            for seg in segments {
                if let Some(words) = seg.words {
                    all_tokens.extend(extract_tokens_from_words(&words));
                } else if let (Some(text), Some(start), Some(end)) = (seg.text, seg.start, seg.end)
                {
                    all_tokens.extend(split_segment_text_evenly(&text, start, end));
                }
            }
            if !all_tokens.is_empty() {
                return Ok(all_tokens);
            }
        }
    }

    // Try parsing as array of segments `[{"text": "...", "start": 0.0, "end": 2.0}]`
    if let Ok(segments) = serde_json::from_str::<Vec<WhisperSegment>>(trimmed) {
        let mut all_tokens = Vec::new();
        for seg in segments {
            if let Some(words) = seg.words {
                all_tokens.extend(extract_tokens_from_words(&words));
            } else if let (Some(text), Some(start), Some(end)) = (seg.text, seg.start, seg.end) {
                all_tokens.extend(split_segment_text_evenly(&text, start, end));
            }
        }
        if !all_tokens.is_empty() {
            return Ok(all_tokens);
        }
    }

    Err(SubtitleError::JsonParse(
        "Unrecognized Whisper JSON schema. Expected verbose segments or list of word objects."
            .into(),
    ))
}

fn extract_tokens_from_words(words: &[WhisperWord]) -> Vec<CaptionToken> {
    let mut tokens = Vec::new();
    for w in words {
        let word_text = w.word.as_deref().or(w.text.as_deref()).unwrap_or("").trim();
        if word_text.is_empty() {
            continue;
        }
        let start_sec = w.start.unwrap_or(0.0).max(0.0);
        let end_sec = w.end.unwrap_or(start_sec + 0.3).max(start_sec);

        let start_ms = (start_sec * 1000.0).round() as u64;
        let end_ms = (end_sec * 1000.0).round() as u64;

        tokens.push(CaptionToken::new(word_text, start_ms, end_ms));
    }
    tokens
}

fn split_segment_text_evenly(text: &str, start_sec: f64, end_sec: f64) -> Vec<CaptionToken> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let start_ms = (start_sec.max(0.0) * 1000.0).round() as u64;
    let end_ms = (end_sec.max(start_sec) * 1000.0).round() as u64;
    let dur = end_ms.saturating_sub(start_ms);
    let word_dur = (dur as f64 / words.len() as f64).max(1.0);

    let mut tokens = Vec::new();
    for (idx, word) in words.iter().enumerate() {
        let w_start = start_ms + (idx as f64 * word_dur) as u64;
        let w_end = if idx == words.len() - 1 {
            end_ms
        } else {
            start_ms + ((idx + 1) as f64 * word_dur) as u64
        };
        tokens.push(CaptionToken::new(*word, w_start, w_end));
    }
    tokens
}

/// Parses WebVTT content into [`CaptionToken`]s.
pub fn parse_vtt(vtt_content: &str) -> Result<Vec<CaptionToken>, SubtitleError> {
    let normalized = vtt_content.replace("\r\n", "\n");
    let mut tokens = Vec::new();

    let blocks = normalized.split("\n\n");
    for block in blocks {
        let trimmed = block.trim();
        if trimmed.is_empty() || trimmed.starts_with("WEBVTT") || trimmed.starts_with("NOTE") {
            continue;
        }

        let lines = trimmed.lines().collect::<Vec<_>>();
        let time_line = lines.iter().find(|l| l.contains("-->"));
        let Some(time_line) = time_line else {
            continue;
        };

        let parts = time_line.split("-->").map(str::trim).collect::<Vec<_>>();
        if parts.len() != 2 {
            continue;
        }

        let start_ms = parse_vtt_timestamp(parts[0].split_whitespace().next().unwrap_or(parts[0]))?;
        let end_ms = parse_vtt_timestamp(parts[1].split_whitespace().next().unwrap_or(parts[1]))?;

        let text_lines: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|l| !l.contains("-->") && !l.parse::<usize>().is_ok())
            .collect();
        let full_text = text_lines.join(" ");

        tokens.extend(split_segment_text_evenly(
            &full_text,
            start_ms as f64 / 1000.0,
            end_ms as f64 / 1000.0,
        ));
    }

    Ok(tokens)
}

fn parse_vtt_timestamp(ts: &str) -> Result<u64, SubtitleError> {
    // WebVTT format: "00:01:23.456" or "01:23.456"
    let clean = ts.trim();
    let parts: Vec<&str> = clean.split(':').collect();

    match parts.len() {
        2 => {
            // mm:ss.mmm
            let mins: u64 = parts[0]
                .parse()
                .map_err(|_| SubtitleError::InvalidTimestamp(ts.into()))?;
            let sec_parts: Vec<&str> = parts[1].split('.').collect();
            if sec_parts.len() != 2 {
                return Err(SubtitleError::InvalidTimestamp(ts.into()));
            }
            let secs: u64 = sec_parts[0]
                .parse()
                .map_err(|_| SubtitleError::InvalidTimestamp(ts.into()))?;
            let ms: u64 = sec_parts[1]
                .parse()
                .map_err(|_| SubtitleError::InvalidTimestamp(ts.into()))?;
            Ok(mins * 60_000 + secs * 1_000 + ms)
        }
        3 => {
            // hh:mm:ss.mmm
            let hours: u64 = parts[0]
                .parse()
                .map_err(|_| SubtitleError::InvalidTimestamp(ts.into()))?;
            let mins: u64 = parts[1]
                .parse()
                .map_err(|_| SubtitleError::InvalidTimestamp(ts.into()))?;
            let sec_parts: Vec<&str> = parts[2].split('.').collect();
            if sec_parts.len() != 2 {
                return Err(SubtitleError::InvalidTimestamp(ts.into()));
            }
            let secs: u64 = sec_parts[0]
                .parse()
                .map_err(|_| SubtitleError::InvalidTimestamp(ts.into()))?;
            let ms: u64 = sec_parts[1]
                .parse()
                .map_err(|_| SubtitleError::InvalidTimestamp(ts.into()))?;
            Ok(hours * 3_600_000 + mins * 60_000 + secs * 1_000 + ms)
        }
        _ => Err(SubtitleError::InvalidTimestamp(ts.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_whisper_direct_words() {
        let json = r#"[
            {"word": "Hello", "start": 0.1, "end": 0.5},
            {"word": "world", "start": 0.5, "end": 0.9}
        ]"#;

        let tokens = parse_whisper_json(json).expect("parse failed");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].text, "Hello");
        assert_eq!(tokens[0].start_ms, 100);
        assert_eq!(tokens[0].end_ms, 500);
        assert_eq!(tokens[1].text, "world");
        assert_eq!(tokens[1].start_ms, 500);
        assert_eq!(tokens[1].end_ms, 900);
    }

    #[test]
    fn test_parse_whisper_verbose_segments() {
        let json = r#"{
            "segments": [
                {
                    "start": 1.0,
                    "end": 2.0,
                    "text": "Viral shorts",
                    "words": [
                        {"word": "Viral", "start": 1.0, "end": 1.4},
                        {"word": "shorts", "start": 1.4, "end": 2.0}
                    ]
                }
            ]
        }"#;

        let tokens = parse_whisper_json(json).expect("parse failed");
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].text, "Viral");
        assert_eq!(tokens[0].start_ms, 1000);
        assert_eq!(tokens[1].text, "shorts");
        assert_eq!(tokens[1].start_ms, 1400);
    }

    #[test]
    fn test_parse_vtt() {
        let vtt = r#"WEBVTT

00:00:01.500 --> 00:00:03.000
Quick brown fox
"#;

        let tokens = parse_vtt(vtt).expect("parse failed");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].text, "Quick");
        assert_eq!(tokens[0].start_ms, 1500);
        assert_eq!(tokens[2].text, "fox");
        assert_eq!(tokens[2].end_ms, 3000);
    }
}
