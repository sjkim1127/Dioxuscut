//! `spring()` — Rust port of Remotion's spring animation.
//!
//! Computes a physically-based spring animation value at a given frame.
//! The algorithm is ported directly from Remotion's `spring-utils.ts`.
//!
//! # Example
//! ```rust
//! use dioxuscut_animation::spring::{spring, SpringConfig};
//!
//! let config = SpringConfig::default(); // damping=10, mass=1, stiffness=100
//! let value = spring(15, 30.0, config);
//! println!("frame 15 spring value: {value:.4}");
//! ```

/// Configuration for the spring physics model.
///
/// Defaults match Remotion's `spring()` defaults exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct SpringConfig {
    /// Resistance force — higher = less oscillation. Default: `10`.
    pub damping: f64,
    /// Object mass — higher = slower. Default: `1`.
    pub mass: f64,
    /// Spring tension — higher = faster. Default: `100`.
    pub stiffness: f64,
    /// If `true`, clamps overshoot to `[from, to]`. Default: `false`.
    pub overshoot_clamping: bool,
}

impl Default for SpringConfig {
    fn default() -> Self {
        Self {
            damping: 10.0,
            mass: 1.0,
            stiffness: 100.0,
            overshoot_clamping: false,
        }
    }
}

#[derive(Debug, Clone)]
struct AnimationNode {
    last_timestamp: f64,
    to_value: f64,
    current: f64,
    velocity: f64,
    prev_position: f64,
}

/// Advance the spring simulation by one timestep.
///
/// Direct port of Remotion's `advance()` in `spring-utils.ts`.
fn advance(node: &AnimationNode, now: f64, config: &SpringConfig) -> AnimationNode {
    let AnimationNode {
        to_value,
        last_timestamp,
        current,
        velocity,
        ..
    } = *node;

    let delta_time = (now - last_timestamp).min(64.0); // cap at 64 ms

    let c = config.damping;
    let m = config.mass;
    let k = config.stiffness;

    let v0 = -velocity;
    let x0 = to_value - current;

    let zeta = c / (2.0 * (k * m).sqrt()); // damping ratio
    let omega0 = (k / m).sqrt(); // undamped angular frequency (rad/ms)
    let omega1 = omega0 * (1.0 - zeta.powi(2)).sqrt(); // exponential decay frequency

    let t = delta_time / 1000.0;

    let sin1 = (omega1 * t).sin();
    let cos1 = (omega1 * t).cos();

    // ── Under-damped (zeta < 1) ───────────────────────────────────────────────
    let under_damped_envelope = (-zeta * omega0 * t).exp();
    let under_damped_frag1 =
        under_damped_envelope * (sin1 * ((v0 + zeta * omega0 * x0) / omega1) + x0 * cos1);

    let under_damped_position = to_value - under_damped_frag1;
    let under_damped_velocity = zeta * omega0 * under_damped_frag1
        - under_damped_envelope * (cos1 * (v0 + zeta * omega0 * x0) - omega1 * x0 * sin1);

    // ── Critically damped (zeta >= 1) ─────────────────────────────────────────
    let critically_damped_envelope = (-omega0 * t).exp();
    let critically_damped_position =
        to_value - critically_damped_envelope * (x0 + (v0 + omega0 * x0) * t);
    let critically_damped_velocity =
        critically_damped_envelope * (v0 * (t * omega0 - 1.0) + t * x0 * omega0 * omega0);

    let (new_current, new_velocity) = if zeta < 1.0 {
        (under_damped_position, under_damped_velocity)
    } else {
        (critically_damped_position, critically_damped_velocity)
    };

    AnimationNode {
        to_value,
        prev_position: current,
        last_timestamp: now,
        current: new_current,
        velocity: new_velocity,
    }
}

/// Compute the spring value at `frame` (0-indexed).
///
/// # Arguments
/// * `frame` — current frame number
/// * `fps`   — frames per second of the composition
/// * `config` — spring physics configuration
///
/// Returns a value interpolated between `0.0` (start) and `1.0` (settled).
/// Use [`crate::interpolate::interpolate`] to map this to any output range.
///
/// # Notes
/// This is a direct port of Remotion's `springCalculation()` from `spring-utils.ts`.
/// It produces identical results to the JavaScript version.
pub fn spring(frame: u32, fps: f64, config: SpringConfig) -> f64 {
    let from = 0.0_f64;
    let to = 1.0_f64;

    let mut animation = AnimationNode {
        last_timestamp: 0.0,
        current: from,
        to_value: to,
        velocity: 0.0,
        prev_position: 0.0,
    };

    let frame_clamped = frame.max(0) as f64;
    let frame_floor = frame_clamped.floor() as u32;
    let uneven_rest = frame_clamped.fract();

    for f in 0..=frame_floor {
        let f_f64 = if f == frame_floor {
            f as f64 + uneven_rest
        } else {
            f as f64
        };
        let time = (f_f64 / fps) * 1000.0;
        animation = advance(&animation, time, &config);
    }

    let result = animation.current;

    // Apply overshoot clamping
    if config.overshoot_clamping {
        let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
        result.clamp(lo, hi)
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the spring settles to 1.0 and starts at 0.0.
    #[test]
    fn settles_to_one() {
        let config = SpringConfig::default();
        // By frame 60 (2s at 30fps) a default spring should be settled within 0.001 of 1.0
        let v = spring(60, 30.0, config);
        assert!(
            (v - 1.0).abs() < 0.001,
            "frame 60 should be ~1.0, got {v:.6}"
        );
    }

    #[test]
    fn increases_monotonically_early() {
        let config = SpringConfig::default();
        // The spring should increase for the first few frames
        let v0 = spring(0, 30.0, config.clone());
        let v1 = spring(1, 30.0, config.clone());
        let v3 = spring(3, 30.0, config.clone());
        assert!(v1 > v0, "v1({v1:.4}) should be > v0({v0:.4})");
        assert!(v3 > v1, "v3({v3:.4}) should be > v1({v1:.4})");
    }

    #[test]
    fn overshoot_clamping() {
        let config = SpringConfig {
            overshoot_clamping: true,
            ..Default::default()
        };
        // Frame 10 normally overshoots slightly above 1.0
        let v = spring(10, 30.0, config);
        assert!(v <= 1.0, "value {v} should be clamped to <= 1.0");
    }

    #[test]
    fn frame_zero_is_zero() {
        let v = spring(0, 30.0, SpringConfig::default());
        assert!((v - 0.0).abs() < 1e-6, "got {v}");
    }
}
