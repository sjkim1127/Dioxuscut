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

/// Read an ordered batch of encoded samples while preserving sample order.
/// Callers can choose their own scheduling/concurrency policy around this
/// deterministic primitive; the native implementation intentionally avoids
/// spawning unbounded file readers.
pub fn read_encoded_samples(
    path: impl AsRef<std::path::Path>,
    samples: &[EncodedSample],
) -> Result<Vec<Vec<u8>>, MediaMetadataError> {
    if samples.len() > 1_000_000 {
        return Err(MediaMetadataError::InvalidRange(
            "sample batch exceeds 1,000,000 entries".into(),
        ));
    }
    samples
        .iter()
        .map(|sample| read_encoded_sample(path.as_ref(), sample))
        .collect()
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

    #[test]
    fn reads_ordered_sample_batch() {
        let path = std::env::temp_dir().join(format!("dioxuscut-sample-{}.bin", std::process::id()));
        std::fs::write(&path, [1_u8, 2, 3, 4, 5]).unwrap();
        let samples = [
            EncodedSample { sample_index: 0, offset: 3, size: 2, timestamp: 0.0, duration: 0.0, keyframe: false, composition_offset: 0 },
            EncodedSample { sample_index: 1, offset: 0, size: 2, timestamp: 0.0, duration: 0.0, keyframe: true, composition_offset: 0 },
        ];
        assert_eq!(super::read_encoded_samples(&path, &samples).unwrap(), vec![vec![4, 5], vec![1, 2]]);
        std::fs::remove_file(path).unwrap();
    }
}
