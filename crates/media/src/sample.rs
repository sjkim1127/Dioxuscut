//! Decoder-neutral encoded media sample contracts shared by native and browser hosts.

use serde::{Deserialize, Serialize};

/// A single encoded sample location and timeline position.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EncodedSample {
    pub sample_index: u32,
    pub offset: u64,
    pub size: u32,
    pub timestamp: f64,
    pub duration: f64,
    pub keyframe: bool,
    pub composition_offset: i64,
}

impl EncodedSample {
    /// Presentation timestamp after applying a composition offset in the
    /// track's timescale. This matches the browser sample contract.
    pub fn presentation_timestamp(&self, timescale: u32) -> Option<f64> {
        (timescale > 0).then(|| self.timestamp + self.composition_offset as f64 / timescale as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::EncodedSample;

    #[test]
    fn presentation_timestamp_applies_composition_offset() {
        let sample = EncodedSample {
            sample_index: 3,
            offset: 128,
            size: 64,
            timestamp: 1.0,
            duration: 0.04,
            keyframe: false,
            composition_offset: 480,
        };
        let timestamp = sample.presentation_timestamp(48_000).unwrap();
        assert!((timestamp - 1.01).abs() < f64::EPSILON);
        assert_eq!(sample.presentation_timestamp(0), None);
    }
}
