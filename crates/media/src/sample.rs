//! Decoder-neutral encoded media sample contracts shared by native and browser hosts.

use serde::{Deserialize, Serialize};

use crate::metadata::{read_media_range, MediaMetadataError};

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

/// Read one encoded sample using the same bounded range contract as the
/// browser `readIsoBmffSample()` adapter.
pub fn read_encoded_sample(
    path: impl AsRef<std::path::Path>,
    sample: &EncodedSample,
) -> Result<Vec<u8>, MediaMetadataError> {
    let end = sample
        .offset
        .checked_add(u64::from(sample.size))
        .ok_or_else(|| MediaMetadataError::InvalidRange("sample range overflow".into()))?;
    read_media_range(path, sample.offset, end)
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
