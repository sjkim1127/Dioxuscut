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

/// Cache telemetry distinguishing hits, misses, and hit rate.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct CacheTelemetry {
    pub hits: u64,
    pub misses: u64,
    pub hit_rate: Option<f64>,
}

impl CacheTelemetry {
    pub fn new(hits: u64, misses: u64) -> Self {
        let total = hits.saturating_add(misses);
        let hit_rate = if total > 0 {
            Some((hits as f64 / total as f64) * 100.0)
        } else {
            None
        };
        Self {
            hits,
            misses,
            hit_rate,
        }
    }
}

/// Granular per-frame telemetry sample capturing the elapsed time and data volumes
/// across every stage of the GPU rendering pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FrameProfileSample {
    /// Zero-based frame index within the composition or render sequence.
    pub frame_idx: u32,
    /// Time spent evaluating the scene AST (e.g. Rhai script, Remotion composition evaluation).
    pub scene_eval_ns: u64,
    /// Time spent decoding assets (video frames, GIF frames, image files, Lottie, text rasterization).
    pub decode_ns: u64,
    /// Time spent preparing and uploading textures to the GPU queue (images, video, text atlas).
    pub upload_ns: u64,
    /// Total bytes uploaded to GPU textures for this frame.
    pub upload_bytes: u64,
    /// Time spent compiling the scene and encoding WGPU render passes/commands.
    pub compile_encode_ns: u64,
    /// CPU residual wait time waiting for GPU hardware fence (`poll(Maintain::wait_for(submission))`).
    pub submission_wait_ns: u64,
    /// Exact GPU hardware execution time measured via WGPU timestamp queries (if supported).
    pub gpu_exec_ns: Option<u64>,
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
    pub session_wall_ms: f64,
    pub throughput_fps: f64,
    pub scene_eval: StageSummary,
    pub decode: StageSummary,
    pub upload: StageSummary,
    pub compile_encode: StageSummary,
    pub submission_wait: StageSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_exec: Option<StageSummary>,
    pub readback: StageSummary,
    pub video_encode: StageSummary,
    pub total_frame: StageSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steady_state: Option<StageSummary>,
    pub image_cache: CacheTelemetry,
    pub text_atlas_cache: CacheTelemetry,
}

impl ProfilingSummary {
    /// Compute a statistical profiling summary from a sequence of frame samples and cache counters.
    pub fn from_samples(
        samples: &[FrameProfileSample],
        session_wall_ns: u64,
        image_cache: CacheTelemetry,
        text_atlas_cache: CacheTelemetry,
    ) -> Self {
        if samples.is_empty() {
            return Self {
                image_cache,
                text_atlas_cache,
                ..Default::default()
            };
        }

        let total_frames = samples.len() as u32;
        let mut gpu_frames = 0u32;
        let mut cpu_fallback_frames = 0u32;

        let mut scene_eval_ns = Vec::with_capacity(samples.len());
        let mut decode_ns = Vec::with_capacity(samples.len());
        let mut upload_ns = Vec::with_capacity(samples.len());
        let mut upload_bytes = Vec::with_capacity(samples.len());
        let mut compile_encode_ns = Vec::with_capacity(samples.len());
        let mut submission_wait_ns = Vec::with_capacity(samples.len());
        let mut gpu_exec_ns = Vec::new();
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
            scene_eval_ns.push(sample.scene_eval_ns);
            decode_ns.push(sample.decode_ns);
            upload_ns.push(sample.upload_ns);
            upload_bytes.push(sample.upload_bytes);
            compile_encode_ns.push(sample.compile_encode_ns);
            submission_wait_ns.push(sample.submission_wait_ns);
            if let Some(exec) = sample.gpu_exec_ns {
                gpu_exec_ns.push(exec);
            }
            readback_ns.push(sample.readback_ns);
            readback_bytes.push(sample.readback_bytes);
            video_encode_ns.push(sample.video_encode_ns);
            total_frame_ns.push(sample.total_frame_ns);
        }

        let scene_eval = summarize_stage(&scene_eval_ns, &[]);
        let decode = summarize_stage(&decode_ns, &[]);
        let upload = summarize_stage(&upload_ns, &upload_bytes);
        let compile_encode = summarize_stage(&compile_encode_ns, &[]);
        let submission_wait = summarize_stage(&submission_wait_ns, &[]);
        let gpu_exec = if !gpu_exec_ns.is_empty() {
            Some(summarize_stage(&gpu_exec_ns, &[]))
        } else {
            None
        };
        let readback = summarize_stage(&readback_ns, &readback_bytes);
        let video_encode = summarize_stage(&video_encode_ns, &[]);
        let total_frame = summarize_stage(&total_frame_ns, &[]);

        let session_wall_ms = if session_wall_ns > 0 {
            session_wall_ns as f64 / 1_000_000.0
        } else {
            total_frame.total_ms
        };

        let throughput_fps = if session_wall_ms > 0.0 {
            (total_frames as f64 * 1000.0) / session_wall_ms
        } else {
            0.0
        };

        let steady_state = if total_frames > 1 {
            Some(summarize_stage(&total_frame_ns[1..], &[]))
        } else {
            None
        };

        Self {
            total_frames,
            gpu_frames,
            cpu_fallback_frames,
            session_wall_ms,
            throughput_fps,
            scene_eval,
            decode,
            upload,
            compile_encode,
            submission_wait,
            gpu_exec,
            readback,
            video_encode,
            total_frame,
            steady_state,
            image_cache,
            text_atlas_cache,
        }
    }

    /// Render a formatted ASCII breakdown table for display in logs and CLI outputs.
    pub fn render_table(&self) -> String {
        let mut out = String::new();
        out.push_str("┌───────────────────────────┬─────────┬─────────┬─────────┬─────────┬──────────┬──────────────┐\n");
        out.push_str("│ Pipeline Stage            │     p50 │     p95 │     p99 │    Mean │    Total │ Bandwidth    │\n");
        out.push_str("├───────────────────────────┼─────────┼─────────┼─────────┼─────────┼──────────┼──────────────┤\n");

        if self.scene_eval.total_ms > 0.001 {
            append_stage_row(&mut out, "0. Scene Evaluation", &self.scene_eval, false);
        }
        append_stage_row(&mut out, "1. Decode & Asset Prep", &self.decode, false);
        append_stage_row(&mut out, "2. Texture Upload", &self.upload, true);
        append_stage_row(
            &mut out,
            "3. Compile & Command Encode",
            &self.compile_encode,
            false,
        );
        append_stage_row(
            &mut out,
            "4. GPU Fence Wait (CPU)",
            &self.submission_wait,
            false,
        );
        if let Some(ref exec) = self.gpu_exec {
            append_stage_row(&mut out, "5. GPU Hardware Exec (ts)", exec, false);
        } else {
            out.push_str("│ 5. GPU Hardware Exec      │       - │       - │       - │       - │        - │ (N/A: no ts) │\n");
        }
        append_stage_row(&mut out, "6. Readback & Buffer Unmap", &self.readback, true);
        append_stage_row(
            &mut out,
            "7. Video Encode / Pipe Write",
            &self.video_encode,
            false,
        );

        out.push_str("├───────────────────────────┼─────────┼─────────┼─────────┼─────────┼──────────┼──────────────┤\n");
        out.push_str(&format!(
            "│ Total Frame Latency       │ {:>6.2}ms │ {:>6.2}ms │ {:>6.2}ms │ {:>6.2}ms │ {:>7.2}ms │            - │\n",
            self.total_frame.p50_ms,
            self.total_frame.p95_ms,
            self.total_frame.p99_ms,
            self.total_frame.mean_ms,
            self.total_frame.total_ms,
        ));
        out.push_str("└───────────────────────────┴─────────┴─────────┴─────────┴─────────┴──────────┴──────────────┘\n");

        out.push_str(&format!(
            "Throughput: {:.1} FPS (Session Wall: {:.2}ms, {} frames)\n",
            self.throughput_fps, self.session_wall_ms, self.total_frames,
        ));

        if let Some(ref steady) = self.steady_state {
            out.push_str(&format!(
                "Steady-State Latency (excl. frame 0 warm-up): p50 = {:.2}ms | p95 = {:.2}ms | p99 = {:.2}ms | Mean = {:.2}ms\n",
                steady.p50_ms, steady.p95_ms, steady.p99_ms, steady.mean_ms
            ));
        }

        let img_rate_str = match self.image_cache.hit_rate {
            Some(rate) => format!("{:.1}%", rate),
            None => "N/A".to_string(),
        };
        let atlas_rate_str = match self.text_atlas_cache.hit_rate {
            Some(rate) => format!("{:.1}%", rate),
            None => "N/A".to_string(),
        };

        let cache_str = format!(
            "Image Cache: {} ({} hits, {} misses) | Text Atlas: {} ({} hits, {} misses) | Frames: {} (GPU: {}, Fallback: {})\n",
            img_rate_str,
            self.image_cache.hits,
            self.image_cache.misses,
            atlas_rate_str,
            self.text_atlas_cache.hits,
            self.text_atlas_cache.misses,
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
                scene_eval_ns: 500_000,
                decode_ns: 2_000_000,
                upload_ns: 1_000_000,
                upload_bytes: 1024,
                compile_encode_ns: 3_000_000,
                submission_wait_ns: 5_000_000,
                gpu_exec_ns: Some(3_500_000),
                readback_ns: 2_000_000,
                readback_bytes: 4096,
                video_encode_ns: 1_000_000,
                total_frame_ns: 14_500_000,
                cpu_fallback: false,
            },
            FrameProfileSample {
                frame_idx: 1,
                scene_eval_ns: 200_000,
                decode_ns: 1_000_000,
                upload_ns: 500_000,
                upload_bytes: 0,
                compile_encode_ns: 2_000_000,
                submission_wait_ns: 4_000_000,
                gpu_exec_ns: Some(2_500_000),
                readback_ns: 1_500_000,
                readback_bytes: 4096,
                video_encode_ns: 1_000_000,
                total_frame_ns: 10_200_000,
                cpu_fallback: false,
            },
        ];

        let img_cache = CacheTelemetry::new(10, 2);
        let atlas_cache = CacheTelemetry::new(0, 0);
        let session_wall_ns = 20_000_000;
        let summary =
            ProfilingSummary::from_samples(&samples, session_wall_ns, img_cache, atlas_cache);
        assert_eq!(summary.total_frames, 2);
        assert_eq!(summary.gpu_frames, 2);
        assert_eq!(summary.cpu_fallback_frames, 0);
        assert_eq!(summary.session_wall_ms, 20.0);
        assert!((summary.throughput_fps - 100.0).abs() < 1e-4);
        assert!(summary.decode.p50_ms > 0.0);
        assert_eq!(summary.upload.total_bytes, 1024);
        assert_eq!(summary.readback.total_bytes, 8192);
        assert_eq!(summary.readback.bytes_per_frame, 4096.0);
        assert!(summary.gpu_exec.is_some());
        assert!(summary.steady_state.is_some());
        assert_eq!(summary.image_cache.hit_rate, Some(10.0 / 12.0 * 100.0));
        assert_eq!(summary.text_atlas_cache.hit_rate, None);

        let table = summary.render_table();
        assert!(table.contains("Pipeline Stage"));
        assert!(table.contains("GPU Fence Wait (CPU)"));
        assert!(table.contains("GPU Hardware Exec (ts)"));
        assert!(table.contains("Throughput: 100.0 FPS"));
        assert!(table.contains("Steady-State Latency"));
        assert!(table.contains("Text Atlas: N/A"));
    }
}
