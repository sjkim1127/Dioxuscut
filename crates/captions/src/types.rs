//! Caption data structures and types.

use serde::{Deserialize, Serialize};

/// Represents an individual timed caption token or word.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionToken {
    /// Text content of the word/phrase.
    pub text: String,
    /// Start timestamp in milliseconds.
    pub start_ms: u64,
    /// End timestamp in milliseconds.
    pub end_ms: u64,
}

impl CaptionToken {
    pub fn new(text: impl Into<String>, start_ms: u64, end_ms: u64) -> Self {
        Self {
            text: text.into(),
            start_ms,
            end_ms,
        }
    }
}

/// A page or group of caption tokens displayed together on screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionPage {
    /// Tokens included in this page view.
    pub tokens: Vec<CaptionToken>,
    /// Page start timestamp in milliseconds.
    pub start_ms: u64,
    /// Page end timestamp in milliseconds.
    pub end_ms: u64,
}
