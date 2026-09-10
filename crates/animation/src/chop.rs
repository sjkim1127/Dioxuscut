//! TouchDesigner-inspired CHOP (Channel Operator) primitives for programmatic motion.
//!
//! Provides:
//! - [`LfoChop`]: Low-frequency oscillator waveforms (Sine, Square, Triangle, Ramp, Pulse).
//! - [`LagChop`]: Inertial lag smoothing filter for organic motion response.
//! - [`MathChop`]: Channel combinations and remapping.
//! - [`AudioEnvelopeChop`]: Audio-reactive envelope tracking with attack and release dynamics.

use std::f64::consts::PI;

/// Waveform shape for [`LfoChop`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LfoWave {
    #[default]
    Sine,
    Triangle,
    Square,
    Ramp,
    Pulse,
}

/// Low-Frequency Oscillator (LFO) Channel Operator.
///
/// Generates continuous periodic modulation curves synchronized with composition timeline FPS.
#[derive(Debug, Clone, PartialEq)]
pub struct LfoChop {
    pub wave: LfoWave,
    pub freq_hz: f64,
    pub amplitude: f64,
    pub offset: f64,
    pub phase: f64,
    pub pulse_width: f64,
}

impl Default for LfoChop {
    fn default() -> Self {
        Self {
            wave: LfoWave::Sine,
            freq_hz: 1.0,
            amplitude: 1.0,
            offset: 0.0,
            phase: 0.0,
            pulse_width: 0.5,
        }
    }
}

impl LfoChop {
    pub fn new(wave: LfoWave, freq_hz: f64) -> Self {
        Self {
            wave,
            freq_hz,
            ..Default::default()
        }
    }

    pub fn with_amplitude(mut self, amp: f64) -> Self {
        self.amplitude = amp;
        self
    }

    pub fn with_offset(mut self, offset: f64) -> Self {
        self.offset = offset;
        self
    }

    pub fn with_phase(mut self, phase: f64) -> Self {
        self.phase = phase;
        self
    }

    /// Sample the oscillator value at the given composition `frame` and `fps`.
    pub fn sample(&self, frame: f64, fps: f64) -> f64 {
        let t = frame / fps.max(1.0);
        let cycle = (t * self.freq_hz + self.phase).rem_euclid(1.0);

        let raw = match self.wave {
            LfoWave::Sine => (cycle * 2.0 * PI).sin(),
            LfoWave::Triangle => {
                if cycle < 0.5 {
                    -1.0 + 4.0 * cycle
                } else {
                    3.0 - 4.0 * cycle
                }
            }
            LfoWave::Square => {
                if cycle < self.pulse_width {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoWave::Ramp => -1.0 + 2.0 * cycle,
            LfoWave::Pulse => {
                if cycle < self.pulse_width {
                    1.0
                } else {
                    0.0
                }
            }
        };

        self.offset + self.amplitude * raw
    }
}

/// Inertial lag smoothing filter matching TouchDesigner's `Lag CHOP`.
///
/// Smooths jagged or abrupt value steps with asymmetric rise/fall inertia.
#[derive(Debug, Clone, PartialEq)]
pub struct LagChop {
    pub lag_up: f64,
    pub lag_down: f64,
}

impl Default for LagChop {
    fn default() -> Self {
        Self {
            lag_up: 0.2,
            lag_down: 0.2,
        }
    }
}

impl LagChop {
    pub fn new(lag_up: f64, lag_down: f64) -> Self {
        Self { lag_up, lag_down }
    }

    /// Update smoothed state towards `target` given time step `dt` seconds.
    pub fn filter(&self, current: f64, target: f64, dt: f64) -> f64 {
        let lag = if target >= current {
            self.lag_up.max(0.0001)
        } else {
            self.lag_down.max(0.0001)
        };

        let alpha = 1.0 - (-dt.max(0.0) / lag).exp();
        current + (target - current) * alpha.clamp(0.0, 1.0)
    }
}

/// Audio envelope follower for reactive visual parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioEnvelopeChop {
    pub attack: f64,
    pub release: f64,
}

impl Default for AudioEnvelopeChop {
    fn default() -> Self {
        Self {
            attack: 0.01,  // 10ms fast rise
            release: 0.15, // 150ms smooth decay
        }
    }
}

impl AudioEnvelopeChop {
    pub fn new(attack: f64, release: f64) -> Self {
        Self { attack, release }
    }

    /// Track audio envelope from instantaneous audio amplitude sample.
    pub fn track(&self, current_env: f64, sample_amp: f64, dt: f64) -> f64 {
        let target = sample_amp.abs();
        let rate = if target > current_env {
            self.attack.max(0.0001)
        } else {
            self.release.max(0.0001)
        };

        let alpha = 1.0 - (-dt.max(0.0) / rate).exp();
        current_env + (target - current_env) * alpha.clamp(0.0, 1.0)
    }
}

/// Operation for combining multi-channel signals in [`MathChop`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathOp {
    Add,
    Multiply,
    Average,
    Minimum,
    Maximum,
}

/// Math Channel Operator for signal combining and remapping.
pub struct MathChop;

impl MathChop {
    /// Remap a value from input range to output range, with optional clamping.
    pub fn map_range(val: f64, in_range: (f64, f64), out_range: (f64, f64), clamp: bool) -> f64 {
        let (in_min, in_max) = in_range;
        let (out_min, out_max) = out_range;
        let span = in_max - in_min;
        if span.abs() < 1e-9 {
            return out_min;
        }

        let normalized = (val - in_min) / span;
        let t = if clamp {
            normalized.clamp(0.0, 1.0)
        } else {
            normalized
        };

        out_min + (out_max - out_min) * t
    }

    /// Combine multiple channels into a single channel output.
    pub fn combine(channels: &[f64], op: MathOp) -> f64 {
        if channels.is_empty() {
            return 0.0;
        }

        match op {
            MathOp::Add => channels.iter().sum(),
            MathOp::Multiply => channels.iter().product(),
            MathOp::Average => channels.iter().sum::<f64>() / channels.len() as f64,
            MathOp::Minimum => channels.iter().copied().fold(f64::INFINITY, f64::min),
            MathOp::Maximum => channels.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lfo_sine_sampling() {
        let lfo = LfoChop::new(LfoWave::Sine, 1.0); // 1 Hz
        let s0 = lfo.sample(0.0, 30.0);
        let s15 = lfo.sample(7.5, 30.0); // 1/4 period -> peak 1.0
        let s30 = lfo.sample(15.0, 30.0); // 1/2 period -> 0.0

        assert!(s0.abs() < 1e-4);
        assert!((s15 - 1.0).abs() < 1e-4);
        assert!(s30.abs() < 1e-4);
    }

    #[test]
    fn test_lag_filter_convergence() {
        let lag = LagChop::new(0.1, 0.1);
        let mut val = 0.0;
        // Step to 1.0 across 10 frames at 30 fps (dt = 0.0333s)
        for _ in 0..10 {
            val = lag.filter(val, 1.0, 1.0 / 30.0);
        }
        assert!(val > 0.9, "Lag filter should converge towards 1.0: {val}");
    }

    #[test]
    fn test_math_chop_range_remap() {
        let mapped = MathChop::map_range(5.0, (0.0, 10.0), (100.0, 200.0), true);
        assert_eq!(mapped, 150.0);
    }
}
