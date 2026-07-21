//! Easing functions — Rust port of Remotion's `Easing` module.
//!
//! All functions map an input in `[0.0, 1.0]` to an output in `[0.0, 1.0]`
//! (though some may overshoot slightly).

/// An easing function: maps progress `t ∈ [0, 1]` to an eased value.
pub type EasingFn = fn(f64) -> f64;

// ─── Basic easings ────────────────────────────────────────────────────────────

/// Linear — no easing.
#[inline]
pub fn linear(t: f64) -> f64 {
    t
}

/// Quad ease-in.
#[inline]
pub fn ease_in_quad(t: f64) -> f64 {
    t * t
}

/// Quad ease-out.
#[inline]
pub fn ease_out_quad(t: f64) -> f64 {
    t * (2.0 - t)
}

/// Quad ease-in-out.
#[inline]
pub fn ease_in_out_quad(t: f64) -> f64 {
    if t < 0.5 {
        2.0 * t * t
    } else {
        -1.0 + (4.0 - 2.0 * t) * t
    }
}

/// Cubic ease-in.
#[inline]
pub fn ease_in_cubic(t: f64) -> f64 {
    t * t * t
}

/// Cubic ease-out.
#[inline]
pub fn ease_out_cubic(t: f64) -> f64 {
    let t1 = t - 1.0;
    t1 * t1 * t1 + 1.0
}

/// Cubic ease-in-out.
#[inline]
pub fn ease_in_out_cubic(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        (t - 1.0) * (2.0 * t - 2.0) * (2.0 * t - 2.0) + 1.0
    }
}

/// Sine ease-in.
#[inline]
pub fn ease_in_sine(t: f64) -> f64 {
    1.0 - (t * std::f64::consts::FRAC_PI_2).cos()
}

/// Sine ease-out.
#[inline]
pub fn ease_out_sine(t: f64) -> f64 {
    (t * std::f64::consts::FRAC_PI_2).sin()
}

/// Sine ease-in-out.
#[inline]
pub fn ease_in_out_sine(t: f64) -> f64 {
    -(((std::f64::consts::PI * t).cos() - 1.0) / 2.0)
}

// ─── Cubic Bézier (Remotion `Easing.bezier`) ─────────────────────────────────

/// Generates a cubic Bézier easing function from two control points.
///
/// Equivalent to CSS `cubic-bezier(x1, y1, x2, y2)` and Remotion's
/// `Easing.bezier(x1, y1, x2, y2)`.
///
/// # Arguments
/// * `x1`, `y1` — first control point
/// * `x2`, `y2` — second control point
///
/// Returns a closure that maps `t ∈ [0, 1]` → eased value.
pub fn bezier(x1: f64, y1: f64, x2: f64, y2: f64) -> impl Fn(f64) -> f64 {
    move |t: f64| {
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }

        // Newton's method to find the parameter `u` such that Bx(u) = t
        let cx = 3.0 * x1;
        let bx = 3.0 * (x2 - x1) - cx;
        let ax = 1.0 - cx - bx;

        let cy = 3.0 * y1;
        let by = 3.0 * (y2 - y1) - cy;
        let ay = 1.0 - cy - by;

        let poly_x = |u: f64| ((ax * u + bx) * u + cx) * u;
        let poly_x_deriv = |u: f64| (3.0 * ax * u + 2.0 * bx) * u + cx;
        let poly_y = |u: f64| ((ay * u + by) * u + cy) * u;

        // Initial guess
        let mut u = t;
        for _ in 0..8 {
            let x_err = poly_x(u) - t;
            let d = poly_x_deriv(u);
            if d.abs() < 1e-12 {
                break;
            }
            u -= x_err / d;
        }
        poly_y(u)
    }
}

// ─── Remotion-style preset easings ───────────────────────────────────────────

/// CSS `ease` preset — `cubic-bezier(0.25, 0.1, 0.25, 1.0)`.
pub fn ease() -> impl Fn(f64) -> f64 {
    bezier(0.25, 0.1, 0.25, 1.0)
}

/// CSS `ease-in` preset — `cubic-bezier(0.42, 0, 1.0, 1.0)`.
pub fn ease_in() -> impl Fn(f64) -> f64 {
    bezier(0.42, 0.0, 1.0, 1.0)
}

/// CSS `ease-out` preset — `cubic-bezier(0, 0, 0.58, 1.0)`.
pub fn ease_out() -> impl Fn(f64) -> f64 {
    bezier(0.0, 0.0, 0.58, 1.0)
}

/// CSS `ease-in-out` preset — `cubic-bezier(0.42, 0, 0.58, 1.0)`.
pub fn ease_in_out() -> impl Fn(f64) -> f64 {
    bezier(0.42, 0.0, 0.58, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_identity() {
        assert!((linear(0.5) - 0.5).abs() < 1e-10);
    }

    #[test]
    fn bezier_endpoints() {
        let f = bezier(0.25, 0.1, 0.25, 1.0);
        assert!((f(0.0)).abs() < 1e-10);
        assert!((f(1.0) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn bezier_midpoint_reasonable() {
        let f = ease_out();
        // ease-out should be > 0.5 at t=0.5 (fast start, slow end)
        assert!(f(0.5) > 0.5);
    }
}
