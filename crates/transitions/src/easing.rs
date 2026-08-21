//! Easing curves, cubic Bézier, and physics-based spring timing for transitions.

pub use dioxuscut_animation::easing::{
    bezier, ease, ease_in, ease_in_cubic, ease_in_out, ease_in_out_cubic, ease_in_out_quad,
    ease_in_out_sine, ease_in_quad, ease_in_sine, ease_out, ease_out_cubic, ease_out_quad,
    ease_out_sine, linear, EasingFn,
};
pub use dioxuscut_animation::spring::{spring, SpringConfig};

/// Timing strategy determining how progress advances over frames.
pub trait TransitionTiming: Send + Sync {
    /// Duration of the transition in frames.
    fn duration_in_frames(&self) -> u32;
    /// Progress value in `0.0..=1.0` at the given local frame index.
    fn progress(&self, frame: u32) -> f32;
}

/// Linear time progression with optional custom easing curve.
#[derive(Debug, Clone, Copy)]
pub struct LinearTiming {
    pub duration_in_frames: u32,
    pub easing: Option<EasingFn>,
}

impl PartialEq for LinearTiming {
    fn eq(&self, other: &Self) -> bool {
        self.duration_in_frames == other.duration_in_frames
            && match (self.easing, other.easing) {
                (None, None) => true,
                (Some(a), Some(b)) => std::ptr::fn_addr_eq(a, b),
                _ => false,
            }
    }
}

impl LinearTiming {
    pub fn new(duration_in_frames: u32) -> Self {
        Self {
            duration_in_frames,
            easing: None,
        }
    }

    pub fn with_easing(mut self, easing: EasingFn) -> Self {
        self.easing = Some(easing);
        self
    }
}

impl TransitionTiming for LinearTiming {
    fn duration_in_frames(&self) -> u32 {
        self.duration_in_frames
    }

    fn progress(&self, frame: u32) -> f32 {
        if self.duration_in_frames == 0 {
            return 1.0;
        }
        let linear_p = (frame as f64 / self.duration_in_frames as f64).clamp(0.0, 1.0);
        let eased = match self.easing {
            Some(f) => f(linear_p),
            None => linear_p,
        };
        eased.clamp(0.0, 1.0) as f32
    }
}

/// Damped harmonic oscillator physics timing.
#[derive(Debug, Clone, PartialEq)]
pub struct SpringTiming {
    pub config: SpringConfig,
    pub fps: f64,
    pub duration_in_frames: u32,
}

impl SpringTiming {
    pub fn new(fps: f64, duration_in_frames: u32) -> Self {
        Self {
            config: SpringConfig::default(),
            fps,
            duration_in_frames,
        }
    }

    pub fn with_config(mut self, config: SpringConfig) -> Self {
        self.config = config;
        self
    }
}

impl TransitionTiming for SpringTiming {
    fn duration_in_frames(&self) -> u32 {
        self.duration_in_frames
    }

    fn progress(&self, frame: u32) -> f32 {
        if self.duration_in_frames == 0 {
            return 1.0;
        }
        let val = spring(frame, self.fps, self.config.clone());
        val.clamp(0.0, 1.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_timing_basic() {
        let timing = LinearTiming::new(10);
        assert_eq!(timing.duration_in_frames(), 10);
        assert!((timing.progress(0) - 0.0).abs() < 1e-5);
        assert!((timing.progress(5) - 0.5).abs() < 1e-5);
        assert!((timing.progress(10) - 1.0).abs() < 1e-5);
        assert!((timing.progress(15) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn linear_timing_with_easing() {
        let timing = LinearTiming::new(10).with_easing(ease_in_quad);
        assert!((timing.progress(5) - 0.25).abs() < 1e-5);
    }

    #[test]
    fn spring_timing_progress() {
        let timing = SpringTiming::new(30.0, 30);
        assert_eq!(timing.duration_in_frames(), 30);
        assert!((timing.progress(0) - 0.0).abs() < 1e-5);
        assert!(timing.progress(15) > 0.5);
    }
}
