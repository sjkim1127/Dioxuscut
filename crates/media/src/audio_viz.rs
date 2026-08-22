//! Audio waveform visualization — Remotion `visualizeAudio` / `@remotion/media-utils`.
//!
//! Provides utilities to decode audio (WAV), compute waveform slices, calculate frequency
//! spectrum magnitude bins via Discrete Fourier Transform (DFT), and generate SVG visualization paths.

use std::f64::consts::PI;
use std::path::Path;

/// Decoded audio PCM data.
#[derive(Debug, Clone)]
pub struct AudioData {
    /// Per-channel waveform samples (normalized `f32` in `-1.0..=1.0`).
    pub channel_waveforms: Vec<Vec<f32>>,
    pub sample_rate: u32,
    pub duration_secs: f32,
}

/// Target profile for audio spectrum visualization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisualizeFor {
    #[default]
    Music,
    Voice,
}

/// Errors occurring during audio visualization or decoding.
#[derive(Debug)]
pub enum AudioVizError {
    Io(std::io::Error),
    Decode(String),
}

impl std::fmt::Display for AudioVizError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Decode(s) => write!(f, "Audio decode error: {s}"),
        }
    }
}

impl std::error::Error for AudioVizError {}

impl From<std::io::Error> for AudioVizError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Loads and decodes a WAV audio file into [`AudioData`].
pub fn load_audio_data(path: &Path) -> Result<AudioData, AudioVizError> {
    let reader = hound::WavReader::open(path)
        .map_err(|e| AudioVizError::Decode(format!("failed to open WAV: {e}")))?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let sample_rate = spec.sample_rate;

    if channels == 0 {
        return Err(AudioVizError::Decode("audio file has 0 channels".into()));
    }

    let mut channel_waveforms: Vec<Vec<f32>> = vec![Vec::new(); channels];

    match spec.sample_format {
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample;
            if bits <= 16 {
                let max_val = (1i32 << (bits - 1)) as f32;
                for (i, sample) in reader.into_samples::<i16>().enumerate() {
                    let s = sample.map_err(|e| AudioVizError::Decode(e.to_string()))?;
                    let ch = i % channels;
                    channel_waveforms[ch].push((s as f32 / max_val).clamp(-1.0, 1.0));
                }
            } else {
                let max_val = (1i64 << (bits - 1)) as f32;
                for (i, sample) in reader.into_samples::<i32>().enumerate() {
                    let s = sample.map_err(|e| AudioVizError::Decode(e.to_string()))?;
                    let ch = i % channels;
                    channel_waveforms[ch].push((s as f32 / max_val).clamp(-1.0, 1.0));
                }
            }
        }
        hound::SampleFormat::Float => {
            for (i, sample) in reader.into_samples::<f32>().enumerate() {
                let s = sample.map_err(|e| AudioVizError::Decode(e.to_string()))?;
                let ch = i % channels;
                channel_waveforms[ch].push(s.clamp(-1.0, 1.0));
            }
        }
    }

    let total_samples = channel_waveforms[0].len();
    let duration_secs = if sample_rate > 0 {
        total_samples as f32 / sample_rate as f32
    } else {
        0.0
    };

    Ok(AudioData {
        channel_waveforms,
        sample_rate,
        duration_secs,
    })
}

/// Computes normalized frequency band magnitudes (`0.0..=1.0`) for a specific composition frame.
///
/// Uses windowed Discrete Fourier Transform (DFT) sampled around the frame timestamp.
pub fn visualize_audio(
    data: &AudioData,
    frame: u32,
    fps: f64,
    n_samples: usize,
    _optimize_for: VisualizeFor,
) -> Vec<f32> {
    if n_samples == 0 {
        return Vec::new();
    }
    if data.channel_waveforms.is_empty() || data.sample_rate == 0 {
        return vec![0.0; n_samples];
    }

    let channels = data.channel_waveforms.len();
    let total_len = data.channel_waveforms[0].len();
    if total_len == 0 {
        return vec![0.0; n_samples];
    }

    let center_sample = ((frame as f64 / fps.max(1.0)) * data.sample_rate as f64).round() as isize;

    // Window size: around 1024 to 2048 samples (approx 20-40ms at 48kHz)
    let window_size = 1024.min(total_len);
    let half_win = (window_size / 2) as isize;

    let mut window = Vec::with_capacity(window_size);
    for i in 0..window_size {
        let sample_idx = center_sample - half_win + i as isize;
        if sample_idx >= 0 && (sample_idx as usize) < total_len {
            let idx = sample_idx as usize;
            let mut mono = 0.0f32;
            for ch in 0..channels {
                mono += data.channel_waveforms[ch][idx];
            }
            mono /= channels as f32;

            // Apply Hann window
            let multiplier =
                0.5 * (1.0 - (2.0 * PI * i as f64 / (window_size - 1).max(1) as f64).cos());
            window.push(mono as f64 * multiplier);
        } else {
            window.push(0.0);
        }
    }

    // Subsample window if needed to keep DFT fast
    let dft_points = 256.min(window_size);
    let step = (window_size as f64 / dft_points as f64).max(1.0);
    let sampled_input: Vec<f64> = (0..dft_points)
        .map(|i| {
            let idx = ((i as f64 * step) as usize).min(window_size - 1);
            window[idx]
        })
        .collect();

    let mut magnitudes = vec![0.0f32; n_samples];
    let n_in = sampled_input.len();
    if n_in == 0 {
        return magnitudes;
    }

    for (k, mag_out) in magnitudes.iter_mut().enumerate() {
        // Map bin k to frequency index in sampled_input spectrum (using power/logarithmic distribution)
        let freq_ratio = (k + 1) as f64 / n_samples as f64;
        let bin_idx = (freq_ratio.powf(1.8) * (n_in / 2) as f64).clamp(1.0, (n_in / 2) as f64);

        let mut re = 0.0f64;
        let mut im = 0.0f64;
        for (n, &s) in sampled_input.iter().enumerate() {
            let angle = -2.0 * PI * bin_idx * n as f64 / n_in as f64;
            re += s * angle.cos();
            im += s * angle.sin();
        }

        let mag = (re * re + im * im).sqrt() / (n_in as f64 * 0.25);
        *mag_out = (mag as f32).clamp(0.0, 1.0);
    }

    magnitudes
}

/// Slices mono waveform amplitudes across the given time range (`start_sec..start_sec + duration_sec`).
pub fn get_waveform_portion(data: &AudioData, start_sec: f32, duration_sec: f32) -> Vec<f32> {
    if data.channel_waveforms.is_empty() || data.sample_rate == 0 || duration_sec <= 0.0 {
        return Vec::new();
    }

    let channels = data.channel_waveforms.len();
    let total_len = data.channel_waveforms[0].len();

    let start_idx =
        ((start_sec.max(0.0) * data.sample_rate as f32).round() as usize).min(total_len);
    let end_idx = (((start_sec.max(0.0) + duration_sec) * data.sample_rate as f32).round()
        as usize)
        .min(total_len);

    if start_idx >= end_idx {
        return Vec::new();
    }

    let count = end_idx - start_idx;
    let mut portion = Vec::with_capacity(count);

    for idx in start_idx..end_idx {
        let mut mono = 0.0f32;
        for ch in 0..channels {
            mono += data.channel_waveforms[ch][idx];
        }
        portion.push(mono / channels as f32);
    }

    portion
}

/// Generates a smoothed SVG path (`d` attribute string) from a slice of amplitude values.
pub fn create_smooth_svg_path(points: &[f32], width: f32, height: f32) -> String {
    if points.is_empty() {
        return String::new();
    }
    if points.len() == 1 {
        let y = height * (1.0 - points[0].clamp(0.0, 1.0));
        return format!("M 0,{y:.2} L {width:.2},{y:.2}");
    }

    let n = points.len();
    let dx = width / (n - 1) as f32;

    let coords: Vec<(f32, f32)> = points
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            let x = i as f32 * dx;
            let y = height * (1.0 - p.clamp(0.0, 1.0));
            (x, y)
        })
        .collect();

    let mut path = format!("M {:.2},{:.2}", coords[0].0, coords[0].1);

    for i in 0..n - 1 {
        let p0 = if i > 0 { coords[i - 1] } else { coords[i] };
        let p1 = coords[i];
        let p2 = coords[i + 1];
        let p3 = if i + 2 < n { coords[i + 2] } else { p2 };

        let cp1x = p1.0 + (p2.0 - p0.0) / 6.0;
        let cp1y = p1.1 + (p2.1 - p0.1) / 6.0;
        let cp2x = p2.0 - (p3.0 - p1.0) / 6.0;
        let cp2y = p2.1 - (p3.1 - p1.1) / 6.0;

        path.push_str(&format!(
            " C {:.2},{:.2} {:.2},{:.2} {:.2},{:.2}",
            cp1x, cp1y, cp2x, cp2y, p2.0, p2.1
        ));
    }

    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visualize_audio_returns_correct_length() {
        let data = AudioData {
            channel_waveforms: vec![vec![0.5f32; 44100]],
            sample_rate: 44100,
            duration_secs: 1.0,
        };
        let bars = visualize_audio(&data, 0, 30.0, 32, VisualizeFor::Music);
        assert_eq!(bars.len(), 32);
        for b in &bars {
            assert!(*b >= 0.0 && *b <= 1.0);
        }
    }

    #[test]
    fn test_waveform_portion_length() {
        let data = AudioData {
            channel_waveforms: vec![vec![0.3f32; 44100]],
            sample_rate: 44100,
            duration_secs: 1.0,
        };
        let portion = get_waveform_portion(&data, 0.0, 0.5);
        assert!(portion.len() >= 22000 && portion.len() <= 22100);
    }

    #[test]
    fn test_create_smooth_svg_path_not_empty() {
        let points = vec![0.0, 0.5, 1.0, 0.5, 0.0];
        let path = create_smooth_svg_path(&points, 200.0, 100.0);
        assert!(path.starts_with('M'));
        assert!(path.contains('C'));
    }

    #[test]
    fn test_visualize_audio_empty_data() {
        let data = AudioData {
            channel_waveforms: vec![],
            sample_rate: 44100,
            duration_secs: 0.0,
        };
        let bars = visualize_audio(&data, 0, 30.0, 16, VisualizeFor::Music);
        assert_eq!(bars.len(), 16);
        for b in &bars {
            assert_eq!(*b, 0.0);
        }
    }
}
