use std::collections::HashMap;
use std::f64::consts::PI;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::backend::RasterError;

/// Decoded PCM audio channel samples.
#[derive(Debug, Clone)]
pub struct AudioData {
    /// Per-channel normalized samples in `-1.0..=1.0`.
    pub channel_waveforms: Vec<Vec<f32>>,
    pub sample_rate: u32,
    pub duration_secs: f64,
}

impl AudioData {
    /// Slices mono average waveform amplitudes for a specific time window.
    pub fn get_waveform_slice(
        &self,
        time_secs: f64,
        window_secs: f64,
        n_points: usize,
    ) -> Vec<f32> {
        if n_points == 0 || self.channel_waveforms.is_empty() || self.sample_rate == 0 {
            return vec![0.0; n_points];
        }

        let channels = self.channel_waveforms.len();
        let total_samples = self.channel_waveforms[0].len();
        if total_samples == 0 {
            return vec![0.0; n_points];
        }

        let half_win = window_secs * 0.5;
        let start_sec = (time_secs - half_win).max(0.0);
        let end_sec = (time_secs + half_win).min(self.duration_secs);

        let start_idx = ((start_sec * self.sample_rate as f64).round() as usize).min(total_samples);
        let end_idx = ((end_sec * self.sample_rate as f64).round() as usize).min(total_samples);

        if start_idx >= end_idx {
            return vec![0.0; n_points];
        }

        let range_len = end_idx - start_idx;
        let step = (range_len as f64 / n_points as f64).max(1.0);

        let mut points = Vec::with_capacity(n_points);
        for i in 0..n_points {
            let sample_idx = (start_idx + (i as f64 * step) as usize).min(total_samples - 1);
            let mut mono = 0.0f32;
            for ch in 0..channels {
                mono += self.channel_waveforms[ch][sample_idx];
            }
            points.push((mono / channels as f32).clamp(-1.0, 1.0));
        }

        points
    }

    /// Computes normalized frequency band magnitudes (`0.0..=1.0`) around `time_secs`.
    pub fn get_spectrum(&self, time_secs: f64, n_bins: usize) -> Vec<f32> {
        if n_bins == 0 || self.channel_waveforms.is_empty() || self.sample_rate == 0 {
            return vec![0.0; n_bins];
        }

        let channels = self.channel_waveforms.len();
        let total_len = self.channel_waveforms[0].len();
        if total_len == 0 {
            return vec![0.0; n_bins];
        }

        let center_sample = (time_secs * self.sample_rate as f64).round() as isize;
        let window_size = 1024.min(total_len);
        let half_win = (window_size / 2) as isize;

        let mut window = Vec::with_capacity(window_size);
        for i in 0..window_size {
            let sample_idx = center_sample - half_win + i as isize;
            if sample_idx >= 0 && (sample_idx as usize) < total_len {
                let idx = sample_idx as usize;
                let mut mono = 0.0f32;
                for ch in 0..channels {
                    mono += self.channel_waveforms[ch][idx];
                }
                mono /= channels as f32;

                // Hann window weighting
                let weight =
                    0.5 * (1.0 - (2.0 * PI * i as f64 / (window_size - 1).max(1) as f64).cos());
                window.push(mono as f64 * weight);
            } else {
                window.push(0.0);
            }
        }

        // Subsample for fast discrete frequency bin evaluation
        let dft_points = 256.min(window_size);
        let step = (window_size as f64 / dft_points as f64).max(1.0);
        let sampled_input: Vec<f64> = (0..dft_points)
            .map(|i| {
                let idx = ((i as f64 * step) as usize).min(window_size - 1);
                window[idx]
            })
            .collect();

        let n_in = sampled_input.len();
        if n_in == 0 {
            return vec![0.0; n_bins];
        }

        let mut magnitudes = vec![0.0f32; n_bins];
        for (k, mag_out) in magnitudes.iter_mut().enumerate() {
            let freq_ratio = (k + 1) as f64 / n_bins as f64;
            // Power curve for natural audio band distribution (boost bass/mid)
            let bin_idx = (freq_ratio.powf(1.8) * (n_in / 2) as f64).clamp(1.0, (n_in / 2) as f64);

            let mut re = 0.0f64;
            let mut im = 0.0f64;
            for (n, &s) in sampled_input.iter().enumerate() {
                let angle = -2.0 * PI * bin_idx * n as f64 / n_in as f64;
                re += s * angle.cos();
                im += s * angle.sin();
            }

            let mag = (re * re + im * im).sqrt() / (n_in as f64 * 0.22);
            *mag_out = (mag as f32).clamp(0.0, 1.0);
        }

        magnitudes
    }
}

/// Decodes audio file into PCM samples using Symphonia (pure Rust).
#[allow(clippy::needless_range_loop)]
pub fn decode_audio_file(path: &Path) -> Result<AudioData, RasterError> {
    let file = File::open(path).map_err(|e| RasterError::ImageAsset {
        path: path.display().to_string(),
        reason: format!("failed to open audio file: {e}"),
    })?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .map_err(|e| RasterError::ImageAsset {
            path: path.display().to_string(),
            reason: format!("unsupported audio format or probe failure: {e}"),
        })?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| RasterError::ImageAsset {
            path: path.display().to_string(),
            reason: "no supported audio track found in file".into(),
        })?;

    let dec_opts: DecoderOptions = Default::default();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &dec_opts)
        .map_err(|e| RasterError::ImageAsset {
            path: path.display().to_string(),
            reason: format!("failed to initialize audio decoder: {e}"),
        })?;

    let track_id = track.id;
    let mut sample_rate = 0;
    let mut channel_waveforms: Vec<Vec<f32>> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => {
                tracing::debug!("Audio packet decode finish: {e}");
                break;
            }
        };

        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                if sample_rate == 0 {
                    sample_rate = spec.rate;
                }
                let channels = spec.channels.count();
                if channel_waveforms.is_empty() {
                    channel_waveforms = vec![Vec::new(); channels];
                }

                match decoded {
                    AudioBufferRef::F32(buf) => {
                        for ch in 0..channels {
                            channel_waveforms[ch].extend_from_slice(buf.chan(ch));
                        }
                    }
                    AudioBufferRef::U8(buf) => {
                        for ch in 0..channels {
                            for &s in buf.chan(ch) {
                                channel_waveforms[ch].push((s as f32 - 128.0) / 128.0);
                            }
                        }
                    }
                    AudioBufferRef::U16(buf) => {
                        for ch in 0..channels {
                            for &s in buf.chan(ch) {
                                channel_waveforms[ch].push((s as f32 - 32768.0) / 32768.0);
                            }
                        }
                    }
                    AudioBufferRef::U24(buf) => {
                        for ch in 0..channels {
                            for &s in buf.chan(ch) {
                                channel_waveforms[ch].push((s.0 as f32 - 8388608.0) / 8388608.0);
                            }
                        }
                    }
                    AudioBufferRef::U32(buf) => {
                        for ch in 0..channels {
                            for &s in buf.chan(ch) {
                                channel_waveforms[ch]
                                    .push((s as f32 - 2147483648.0) / 2147483648.0);
                            }
                        }
                    }
                    AudioBufferRef::S8(buf) => {
                        for ch in 0..channels {
                            for &s in buf.chan(ch) {
                                channel_waveforms[ch].push(s as f32 / 128.0);
                            }
                        }
                    }
                    AudioBufferRef::S16(buf) => {
                        for ch in 0..channels {
                            for &s in buf.chan(ch) {
                                channel_waveforms[ch].push(s as f32 / 32768.0);
                            }
                        }
                    }
                    AudioBufferRef::S24(buf) => {
                        for ch in 0..channels {
                            for &s in buf.chan(ch) {
                                channel_waveforms[ch].push(s.0 as f32 / 8388608.0);
                            }
                        }
                    }
                    AudioBufferRef::S32(buf) => {
                        for ch in 0..channels {
                            for &s in buf.chan(ch) {
                                channel_waveforms[ch].push(s as f32 / 2147483648.0);
                            }
                        }
                    }
                    AudioBufferRef::F64(buf) => {
                        for ch in 0..channels {
                            for &s in buf.chan(ch) {
                                channel_waveforms[ch].push(s as f32);
                            }
                        }
                    }
                }
            }
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            Err(SymphoniaError::DecodeError(e)) => {
                tracing::debug!("Audio frame decode error: {e}");
                continue;
            }
            Err(e) => {
                tracing::debug!("Audio decode terminating: {e}");
                break;
            }
        }
    }

    if channel_waveforms.is_empty() || channel_waveforms[0].is_empty() {
        return Err(RasterError::ImageAsset {
            path: path.display().to_string(),
            reason: "no PCM audio samples decoded from file".into(),
        });
    }

    let total_samples = channel_waveforms[0].len();
    let duration_secs = if sample_rate > 0 {
        total_samples as f64 / sample_rate as f64
    } else {
        0.0
    };

    Ok(AudioData {
        channel_waveforms,
        sample_rate,
        duration_secs,
    })
}

/// Cache for loaded audio assets and their spectrum / waveform calculations.
#[derive(Default)]
pub(crate) struct AudioCache {
    audio_files: Mutex<HashMap<PathBuf, Arc<AudioData>>>,
}

impl AudioCache {
    pub(crate) fn get_or_load(&self, src: &str) -> Result<Arc<AudioData>, RasterError> {
        let path = local_path(src)?;
        let canonical = path.canonicalize().unwrap_or(path);

        let mut cache = self.audio_files.lock().expect("audio cache lock poisoned");
        if let Some(entry) = cache.get(&canonical) {
            return Ok(Arc::clone(entry));
        }

        let audio_data = decode_audio_file(&canonical)?;
        let arc_data = Arc::new(audio_data);
        cache.insert(canonical, Arc::clone(&arc_data));
        Ok(arc_data)
    }

    pub(crate) fn get_spectrum(
        &self,
        src: &str,
        time_secs: f64,
        n_bins: usize,
    ) -> Result<Vec<f32>, RasterError> {
        let data = self.get_or_load(src)?;
        Ok(data.get_spectrum(time_secs, n_bins))
    }

    pub(crate) fn get_waveform_slice(
        &self,
        src: &str,
        time_secs: f64,
        window_secs: f64,
        n_points: usize,
    ) -> Result<Vec<f32>, RasterError> {
        let data = self.get_or_load(src)?;
        Ok(data.get_waveform_slice(time_secs, window_secs, n_points))
    }
}

fn local_path(src: &str) -> Result<PathBuf, RasterError> {
    let src = src.trim();
    if src.is_empty() {
        return Err(RasterError::ImageAsset {
            path: src.into(),
            reason: "source path is empty".into(),
        });
    }

    let path = if let Some(path) = src.strip_prefix("file://") {
        path
    } else if src.contains("://") || src.starts_with("data:") {
        return Err(RasterError::ImageAsset {
            path: src.into(),
            reason: "remote URLs and data URIs are not yet supported for audio".into(),
        });
    } else {
        src
    };

    Ok(PathBuf::from(path))
}
