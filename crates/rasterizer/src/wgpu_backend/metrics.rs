//! Instrumentation and timing metrics for WgpuBackend.

use serde::{Deserialize, Serialize};

/// Runtime counters for deciding whether a WGPU render is actually using the
/// GPU path. They are intentionally monotonic and lock-free so instrumentation
/// does not perturb frame scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WgpuRenderStats {
    pub gpu_frames: u64,
    pub cpu_fallback_frames: u64,
    pub texture_cache_hits: u64,
    pub texture_cache_misses: u64,
}

/// Monotonic timing accumulators for the native video texture path.
///
/// * `video_decode_ns` covers decoded-frame cache lookup and FFmpeg decode.
/// * `texture_upload_ns` covers creation and upload of cache-miss textures to the GPU.
/// * `gpu_submit_readback_ns` covers GPU fence synchronization wait (`Maintain::wait_for`)
///   plus staging buffer mapping and copy into CPU memory.
/// * `gpu_submit_no_readback_ns` covers GPU fence synchronization wait (`Maintain::wait_for`)
///   for GPU-native submissions where no CPU readback occurs.
///
/// These are cumulative monotonic nanosecond counters, not per-frame averages,
/// and therefore remain meaningful across streaming renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WgpuVideoTimingStats {
    pub video_decode_ns: u64,
    pub texture_upload_ns: u64,
    pub gpu_submit_readback_ns: u64,
    pub gpu_submit_no_readback_ns: u64,
}

impl WgpuRenderStats {
    /// Fraction of observed frames that used the CPU fallback.
    pub fn cpu_fallback_ratio(self) -> f64 {
        let total = self.gpu_frames.saturating_add(self.cpu_fallback_frames);
        if total == 0 {
            0.0
        } else {
            self.cpu_fallback_frames as f64 / total as f64
        }
    }
}

/// Granular per-frame telemetry sample capturing the elapsed time and data volumes
/// across every stage of the GPU rendering pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FrameProfileSample {
    /// Zero-based frame index within the composition or render sequence.
    pub frame_idx: u32,
    /// Time spent decoding assets (video frames, GIF frames, image files, Lottie).
    pub decode_ns: u64,
    /// Time spent preparing and uploading textures to the GPU queue.
    pub upload_ns: u64,
    /// Total bytes uploaded to GPU textures for this frame.
    pub upload_bytes: u64,
    /// Time spent compiling the scene and encoding WGPU render passes/commands.
    pub compile_encode_ns: u64,
    /// Time spent waiting for GPU hardware execution (`poll(Maintain::wait_for(submission))`).
    pub gpu_fence_ns: u64,
    /// Time spent mapping the staging buffer, unpadding rows, and copying to host memory.
    pub readback_ns: u64,
    /// Bytes transferred from GPU staging buffer to CPU memory (e.g. `width * height * 4`).
    pub readback_bytes: u64,
    /// Time spent in the frame sink (scaling and writing to the video encoder pipe).
    pub video_encode_ns: u64,
    /// Total end-to-end elapsed time for this frame.
    pub total_frame_ns: u64,
    /// Whether this frame fell back to CPU rasterization.
    pub cpu_fallback: bool,
}

/// Statistical summary for a single pipeline stage across all sampled frames.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StageSummary {
    pub min_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub total_ms: f64,
    pub total_bytes: u64,
    pub bytes_per_frame: f64,
}

/// Comprehensive profiling summary aggregating all stages, percentiles, throughput,
/// and cache telemetry across the render session.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ProfilingSummary {
    pub total_frames: u32,
    pub gpu_frames: u32,
    pub cpu_fallback_frames: u32,
    pub total_duration_ms: f64,
    pub effective_fps: f64,
    pub decode: StageSummary,
    pub upload: StageSummary,
    pub compile_encode: StageSummary,
    pub gpu_fence: StageSummary,
    pub readback: StageSummary,
    pub video_encode: StageSummary,
    pub total_frame: StageSummary,
    pub texture_cache_hits: u64,
    pub texture_cache_misses: u64,
    pub texture_cache_hit_rate: f64,
}

impl ProfilingSummary {
    /// Compute a statistical profiling summary from a sequence of frame samples and cache counters.
    pub fn from_samples(
        samples: &[FrameProfileSample],
        cache_hits: u64,
        cache_misses: u64,
    ) -> Self {
        if samples.is_empty() {
            return Self::default();
        }

        let total_frames = samples.len() as u32;
        let mut gpu_frames = 0u32;
        let mut cpu_fallback_frames = 0u32;

        let mut decode_ns = Vec::with_capacity(samples.len());
        let mut upload_ns = Vec::with_capacity(samples.len());
        let mut upload_bytes = Vec::with_capacity(samples.len());
        let mut compile_encode_ns = Vec::with_capacity(samples.len());
        let mut gpu_fence_ns = Vec::with_capacity(samples.len());
        let mut readback_ns = Vec::with_capacity(samples.len());
        let mut readback_bytes = Vec::with_capacity(samples.len());
        let mut video_encode_ns = Vec::with_capacity(samples.len());
        let mut total_frame_ns = Vec::with_capacity(samples.len());

        for sample in samples {
            if sample.cpu_fallback {
                cpu_fallback_frames = cpu_fallback_frames.saturating_add(1);
            } else {
                gpu_frames = gpu_frames.saturating_add(1);
            }
            decode_ns.push(sample.decode_ns);
            upload_ns.push(sample.upload_ns);
            upload_bytes.push(sample.upload_bytes);
            compile_encode_ns.push(sample.compile_encode_ns);
            gpu_fence_ns.push(sample.gpu_fence_ns);
            readback_ns.push(sample.readback_ns);
            readback_bytes.push(sample.readback_bytes);
            video_encode_ns.push(sample.video_encode_ns);
            total_frame_ns.push(sample.total_frame_ns);
        }

        let decode = summarize_stage(&decode_ns, &[]);
        let upload = summarize_stage(&upload_ns, &upload_bytes);
        let compile_encode = summarize_stage(&compile_encode_ns, &[]);
        let gpu_fence = summarize_stage(&gpu_fence_ns, &[]);
        let readback = summarize_stage(&readback_ns, &readback_bytes);
        let video_encode = summarize_stage(&video_encode_ns, &[]);
        let total_frame = summarize_stage(&total_frame_ns, &[]);

        let total_duration_ms = total_frame.total_ms;
        let effective_fps = if total_duration_ms > 0.0 {
            (total_frames as f64 * 1000.0) / total_duration_ms
        } else {
            0.0
        };

        let total_cache_lookups = cache_hits.saturating_add(cache_misses);
        let texture_cache_hit_rate = if total_cache_lookups > 0 {
            (cache_hits as f64 / total_cache_lookups as f64) * 100.0
        } else {
            0.0
        };

        Self {
            total_frames,
            gpu_frames,
            cpu_fallback_frames,
            total_duration_ms,
            effective_fps,
            decode,
            upload,
            compile_encode,
            gpu_fence,
            readback,
            video_encode,
            total_frame,
            texture_cache_hits: cache_hits,
            texture_cache_misses: cache_misses,
            texture_cache_hit_rate,
        }
    }

    /// Render a formatted ASCII breakdown table for display in logs and CLI outputs.
    pub fn render_table(&self) -> String {
        let mut out = String::new();
        out.push_str("┌───────────────────────────┬─────────┬─────────┬─────────┬─────────┬──────────┬──────────────┐\n");
        out.push_str("│ Pipeline Stage            │     p50 │     p95 │     p99 │    Mean │    Total │ Bandwidth    │\n");
        out.push_str("├───────────────────────────┼─────────┼─────────┼─────────┼─────────┼──────────┼──────────────┤\n");

        append_stage_row(&mut out, "1. Decode (Media/Assets)", &self.decode, false);
        append_stage_row(&mut out, "2. Texture Upload", &self.upload, true);
        append_stage_row(
            &mut out,
            "3. Compile & Command Encode",
            &self.compile_encode,
            false,
        );
        append_stage_row(&mut out, "4. GPU Hardware Fence", &self.gpu_fence, false);
        append_stage_row(&mut out, "5. Readback & Buffer Unmap", &self.readback, true);
        append_stage_row(
            &mut out,
            "6. Video Encode / Pipe Write",
            &self.video_encode,
            false,
        );

        out.push_str("├───────────────────────────┼─────────┼─────────┼─────────┼─────────┼──────────┼──────────────┤\n");
        let fps_label = format!("{:.1} FPS", self.effective_fps);
        out.push_str(&format!(
            "│ Total Frame Latency       │ {:>6.2}ms │ {:>6.2}ms │ {:>6.2}ms │ {:>6.2}ms │ {:>7.2}ms │ {:>12} │\n",
            self.total_frame.p50_ms,
            self.total_frame.p95_ms,
            self.total_frame.p99_ms,
            self.total_frame.mean_ms,
            self.total_frame.total_ms,
            fps_label,
        ));
        out.push_str("└───────────────────────────┴─────────┴─────────┴─────────┴─────────┴──────────┴──────────────┘\n");

        let cache_str = format!(
            "Texture Cache: {:.1}% hit rate ({} hits, {} misses) | Frames: {} (GPU: {}, Fallback: {})\n",
            self.texture_cache_hit_rate,
            self.texture_cache_hits,
            self.texture_cache_misses,
            self.total_frames,
            self.gpu_frames,
            self.cpu_fallback_frames,
        );
        out.push_str(&cache_str);

        out
    }
}

fn append_stage_row(out: &mut String, label: &str, stage: &StageSummary, has_bytes: bool) {
    let bytes_label = if has_bytes && stage.bytes_per_frame > 0.0 {
        format!("{:.2} MB/f", stage.bytes_per_frame / (1024.0 * 1024.0))
    } else {
        "-".to_string()
    };
    out.push_str(&format!(
        "│ {:<25} │ {:>6.2}ms │ {:>6.2}ms │ {:>6.2}ms │ {:>6.2}ms │ {:>7.2}ms │ {:>12} │\n",
        label, stage.p50_ms, stage.p95_ms, stage.p99_ms, stage.mean_ms, stage.total_ms, bytes_label,
    ));
}

fn summarize_stage(times_ns: &[u64], bytes: &[u64]) -> StageSummary {
    if times_ns.is_empty() {
        return StageSummary::default();
    }

    let mut sorted_ms: Vec<f64> = times_ns.iter().map(|ns| *ns as f64 / 1_000_000.0).collect();
    sorted_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let min_ms = *sorted_ms.first().unwrap_or(&0.0);
    let max_ms = *sorted_ms.last().unwrap_or(&0.0);
    let total_ms: f64 = sorted_ms.iter().sum();
    let mean_ms = total_ms / sorted_ms.len() as f64;

    let p50_ms = compute_percentile(&sorted_ms, 0.50);
    let p95_ms = compute_percentile(&sorted_ms, 0.95);
    let p99_ms = compute_percentile(&sorted_ms, 0.99);

    let total_bytes: u64 = bytes.iter().sum();
    let bytes_per_frame = if !bytes.is_empty() {
        total_bytes as f64 / bytes.len() as f64
    } else {
        0.0
    };

    StageSummary {
        min_ms,
        max_ms,
        mean_ms,
        p50_ms,
        p95_ms,
        p99_ms,
        total_ms,
        total_bytes,
        bytes_per_frame,
    }
}

fn compute_percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = pct * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        let weight = rank - lower as f64;
        sorted[lower] * (1.0 - weight) + sorted[upper] * weight
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentile_computation() {
        let mut values = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50 = compute_percentile(&values, 0.50);
        assert!((p50 - 5.5).abs() < 1e-6);
        let p95 = compute_percentile(&values, 0.95);
        assert!((p95 - 9.55).abs() < 1e-6);
        let p99 = compute_percentile(&values, 0.99);
        assert!((p99 - 9.91).abs() < 1e-6);
    }

    #[test]
    fn test_profiling_summary_aggregation() {
        let samples = vec![
            FrameProfileSample {
                frame_idx: 0,
                decode_ns: 2_000_000,
                upload_ns: 1_000_000,
                upload_bytes: 1024,
                compile_encode_ns: 3_000_000,
                gpu_fence_ns: 5_000_000,
                readback_ns: 2_000_000,
                readback_bytes: 4096,
                video_encode_ns: 1_000_000,
                total_frame_ns: 14_000_000,
                cpu_fallback: false,
            },
            FrameProfileSample {
                frame_idx: 1,
                decode_ns: 1_000_000,
                upload_ns: 500_000,
                upload_bytes: 0,
                compile_encode_ns: 2_000_000,
                gpu_fence_ns: 4_000_000,
                readback_ns: 1_500_000,
                readback_bytes: 4096,
                video_encode_ns: 1_000_000,
                total_frame_ns: 10_000_000,
                cpu_fallback: false,
            },
        ];

        let summary = ProfilingSummary::from_samples(&samples, 10, 2);
        assert_eq!(summary.total_frames, 2);
        assert_eq!(summary.gpu_frames, 2);
        assert_eq!(summary.cpu_fallback_frames, 0);
        assert!(summary.decode.p50_ms > 0.0);
        assert_eq!(summary.upload.total_bytes, 1024);
        assert_eq!(summary.readback.total_bytes, 8192);
        assert_eq!(summary.readback.bytes_per_frame, 4096.0);
        assert!((summary.texture_cache_hit_rate - (10.0 / 12.0 * 100.0)).abs() < 1e-4);

        let table = summary.render_table();
        assert!(table.contains("Pipeline Stage"));
        assert!(table.contains("Texture Cache:"));
    }
}
