use serde::{Deserialize, Serialize};

/// Speech time interval in seconds `[start, end]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpeechInterval {
    pub start: f64,
    pub end: f64,
}

impl SpeechInterval {
    pub fn new(start: f64, end: f64) -> Self {
        Self {
            start: start.min(end),
            end: start.max(end),
        }
    }
}

/// Options configuring background music auto-ducking around speech intervals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DuckingOptions {
    /// Normal BGM volume level in `0.0..=1.0` (default 0.8).
    pub base_volume: f64,
    /// Ducked BGM volume during speech in `0.0..=1.0` (default 0.15).
    pub duck_volume: f64,
    /// Duration in seconds to ramp down volume before speech starts (default 0.25s).
    pub attack_sec: f64,
    /// Duration in seconds to ramp up volume back to base level after speech ends (default 0.40s).
    pub release_sec: f64,
    /// Minimum silence duration between speech intervals to allow ramping back up (default 0.35s).
    /// If silence is shorter than this, volume remains ducked.
    pub hold_threshold_sec: f64,
}

impl Default for DuckingOptions {
    fn default() -> Self {
        Self {
            base_volume: 0.8,
            duck_volume: 0.15,
            attack_sec: 0.25,
            release_sec: 0.40,
            hold_threshold_sec: 0.35,
        }
    }
}

/// Merges overlapping speech intervals or intervals separated by less than `hold_threshold_sec`.
pub fn merge_speech_intervals(
    intervals: &[SpeechInterval],
    hold_threshold_sec: f64,
) -> Vec<SpeechInterval> {
    if intervals.is_empty() {
        return Vec::new();
    }

    let mut sorted: Vec<SpeechInterval> = intervals
        .iter()
        .filter(|iv| iv.end > iv.start && iv.start >= 0.0)
        .copied()
        .collect();

    sorted.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if sorted.is_empty() {
        return Vec::new();
    }

    let mut merged = Vec::with_capacity(sorted.len());
    let mut current = sorted[0];

    for next in sorted.into_iter().skip(1) {
        if next.start <= current.end + hold_threshold_sec {
            current.end = current.end.max(next.end);
        } else {
            merged.push(current);
            current = next;
        }
    }
    merged.push(current);

    merged
}

/// Calculates a volume keyframe envelope `[(timeline_sec, volume_gain)]` for background music ducking.
pub fn calculate_ducking_envelope(
    intervals: &[SpeechInterval],
    total_duration: f64,
    options: &DuckingOptions,
) -> Vec<(f64, f64)> {
    if total_duration <= 0.0 {
        return Vec::new();
    }

    let merged = merge_speech_intervals(intervals, options.hold_threshold_sec);
    if merged.is_empty() {
        return vec![
            (0.0, options.base_volume),
            (total_duration, options.base_volume),
        ];
    }

    let mut keyframes: Vec<(f64, f64)> = Vec::new();

    // Start of timeline
    keyframes.push((0.0, options.base_volume));

    for iv in merged {
        let t_attack_start = (iv.start - options.attack_sec).max(0.0);
        let t_attack_end = iv.start.min(total_duration);
        let t_release_start = iv.end.min(total_duration);
        let t_release_end = (iv.end + options.release_sec).min(total_duration);

        // Before attack ramp
        if t_attack_start > 0.0 {
            keyframes.push((t_attack_start, options.base_volume));
        }

        // Ramp down to duck volume
        keyframes.push((t_attack_end, options.duck_volume));

        // Hold at duck volume during speech
        if t_release_start > t_attack_end {
            keyframes.push((t_release_start, options.duck_volume));
        }

        // Ramp up to base volume
        if t_release_end > t_release_start {
            keyframes.push((t_release_end, options.base_volume));
        }
    }

    // End of timeline
    if let Some(&last) = keyframes.last() {
        if last.0 < total_duration {
            keyframes.push((total_duration, options.base_volume));
        }
    }

    // Sort and deduplicate keyframes by time
    keyframes.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut deduped: Vec<(f64, f64)> = Vec::with_capacity(keyframes.len());
    for (t, v) in keyframes {
        if let Some(last) = deduped.last_mut() {
            if (last.0 - t).abs() < 0.001 {
                // Keep the lower volume if at nearly identical time
                last.1 = last.1.min(v);
                continue;
            }
        }
        deduped.push((t, v));
    }

    deduped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_speech_intervals() {
        let raw = vec![
            SpeechInterval::new(1.0, 2.0),
            SpeechInterval::new(2.2, 3.0), // gap = 0.2 < hold_threshold 0.35 -> merged
            SpeechInterval::new(5.0, 6.0),
        ];

        let merged = merge_speech_intervals(&raw, 0.35);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].start, 1.0);
        assert_eq!(merged[0].end, 3.0);
        assert_eq!(merged[1].start, 5.0);
        assert_eq!(merged[1].end, 6.0);
    }

    #[test]
    fn test_calculate_ducking_envelope() {
        let raw = vec![SpeechInterval::new(2.0, 4.0)];
        let options = DuckingOptions {
            base_volume: 0.8,
            duck_volume: 0.2,
            attack_sec: 0.5,
            release_sec: 0.5,
            hold_threshold_sec: 0.3,
        };

        let env = calculate_ducking_envelope(&raw, 10.0, &options);
        assert!(!env.is_empty());
        assert_eq!(env[0], (0.0, 0.8));
        // Attack start at 2.0 - 0.5 = 1.5
        assert!(env
            .iter()
            .any(|&(t, v)| (t - 1.5).abs() < 0.01 && (v - 0.8).abs() < 0.01));
        // Ducked at 2.0
        assert!(env
            .iter()
            .any(|&(t, v)| (t - 2.0).abs() < 0.01 && (v - 0.2).abs() < 0.01));
        // Held ducked at 4.0
        assert!(env
            .iter()
            .any(|&(t, v)| (t - 4.0).abs() < 0.01 && (v - 0.2).abs() < 0.01));
        // Release finish at 4.5
        assert!(env
            .iter()
            .any(|&(t, v)| (t - 4.5).abs() < 0.01 && (v - 0.8).abs() < 0.01));
        // End of timeline at 10.0
        assert_eq!(env.last().unwrap().0, 10.0);
    }
}
