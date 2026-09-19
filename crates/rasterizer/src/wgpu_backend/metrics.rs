//! Instrumentation and timing metrics for WgpuBackend.

/// Runtime counters for deciding whether a WGPU render is actually using the
/// GPU path. They are intentionally monotonic and lock-free so instrumentation
/// does not perturb frame scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

