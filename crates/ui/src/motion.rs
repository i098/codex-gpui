//! Motion tokens from Codex 26.1007.30912, checked against `fixtures/motion/`.
//! Pass the easing functions to `gpui::Animation::with_easing`.
//! GPUI's `App::set_reduce_motion` switches decorative animations to their static state.
//! Button hover transitions remain active under reduced motion, as in the original.
//!
//! Run `cargo test -p ui --test motion_frames` to check all recorded tracks.
//! Native frame checks use `repeat_synced` for GPUI's test clock.
//! Unsynced GPUI animations use wall time; `HoverFade` accepts caller-supplied time.

use gpui::App;
use std::time::Duration;

pub const BASIC: Duration = Duration::from_millis(150);
pub const MODERATE: Duration = Duration::from_millis(200);
pub const RELAXED: Duration = Duration::from_millis(300);
pub const STREAMING_TEXT: Duration = Duration::from_millis(700);

pub const EASE_BASIC: fn(f32) -> f32 = |t| cubic_bezier(0.25, 0.1, 0.25, 1.0)(t);
pub const EASE_ENTER: fn(f32) -> f32 = |t| cubic_bezier(0.19, 1.0, 0.22, 1.0)(t);
pub const EASE_EXIT: fn(f32) -> f32 = |t| cubic_bezier(0.8, 0.0, 0.4, 1.0)(t);
pub const EASE_EXIT_SNAPPY: fn(f32) -> f32 = |t| cubic_bezier(0.65, 0.0, 0.4, 1.0)(t);
pub const EASE_MOVE: fn(f32) -> f32 = |t| cubic_bezier(0.65, 0.0, 0.35, 1.0)(t);
pub const EASE_ENTER_SNAPPY: fn(f32) -> f32 = |t| cubic_bezier(0.23, 1.0, 0.32, 1.0)(t);
pub const EASE_OUT: fn(f32) -> f32 = |t| cubic_bezier(0.0, 0.0, 0.2, 1.0)(t);
pub const EASE_IN_OUT: fn(f32) -> f32 = |t| cubic_bezier(0.4, 0.0, 0.2, 1.0)(t);
pub const EASE_COMPOSER: fn(f32) -> f32 = |t| cubic_bezier(0.2, 0.0, 0.0, 1.0)(t);
pub const EASE_STREAMING: fn(f32) -> f32 = |t| cubic_bezier(0.37, 0.55, 0.86, 0.88)(t);
pub const SPRING_SNAPPY_STOPS: &[f32] = &[
    0.0, 0.086, 0.2539, 0.428, 0.5786, 0.698, 0.7879, 0.8534, 0.8998, 0.9323, 0.9546, 0.9697, 0.98,
    0.9868, 0.9913, 0.9943, 0.9963, 0.9976, 0.9985, 1.0,
];

/// CSS cubic-bezier easing on normalized time, clamped to 0..=1.
/// Control points must be finite; x coordinates must be in 0..=1.
/// The output can overshoot when the y coordinates do.
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 + Copy {
    assert!((0.0..=1.0).contains(&x1) && (0.0..=1.0).contains(&x2));
    assert!(y1.is_finite() && y2.is_finite());
    let coefficients = |a: f32, b: f32| {
        let (a, b) = (a as f64, b as f64);
        [1.0 - 3.0 * b + 3.0 * a, 3.0 * b - 6.0 * a, 3.0 * a]
    };
    let x = coefficients(x1, x2);
    let y = coefficients(y1, y2);
    move |time| {
        let time = time.clamp(0.0, 1.0) as f64;
        if time == 0.0 || time == 1.0 {
            return time as f32;
        }
        let curve = |c: [f64; 3], t: f64| ((c[0] * t + c[1]) * t + c[2]) * t;
        let (mut low, mut high, mut t) = (0.0, 1.0, time);
        // Solve x(t) first. Near a flat tangent, bisection keeps Newton's step bounded.
        for _ in 0..32 {
            let error = curve(x, t) - time;
            if error.abs() < 1e-9 {
                break;
            }
            if error < 0.0 {
                low = t;
            } else {
                high = t;
            }
            let slope = (3.0 * x[0] * t + 2.0 * x[1]) * t + x[2];
            let next = t - error / slope;
            t = if next > low && next < high {
                next
            } else {
                (low + high) * 0.5
            };
        }
        curve(y, t) as f32
    }
}

/// CSS `steps(count, end)`. The count must be positive.
pub fn steps(count: u32) -> impl Fn(f32) -> f32 + Copy {
    assert!(count > 0);
    move |t| (t.clamp(0.0, 1.0) * count as f32).floor() / count as f32
}

/// CSS `linear()` with evenly spaced output stops, as used by the spring token.
/// Supply at least two finite stops. The function borrows them without allocating.
pub fn linear_stops(stops: &[f32]) -> impl Fn(f32) -> f32 + Copy + '_ {
    assert!(stops.len() >= 2 && stops.iter().all(|v| v.is_finite()));
    move |t| {
        let position = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
        let index = (position as usize).min(stops.len() - 2);
        stops[index] + (stops[index + 1] - stops[index]) * (position - index as f32)
    }
}

/// Reads the same switch that GPUI's animation and spring elements use.
pub fn reduced(cx: &App) -> bool {
    cx.reduce_motion()
}

/// A button's 150 ms `ease` transition, including CSS reversal shortening.
/// Supply monotonic time since a caller-owned epoch, from GPUI's clock in frame tests.
/// Keep one instance per button. Request frames only while `is_animating` is true.
#[derive(Clone, Debug)]
pub struct HoverFade {
    from: f32,
    target: f32,
    started: Duration,
    shortening: f32,
}

impl Default for HoverFade {
    fn default() -> Self {
        Self {
            from: 0.0,
            target: 0.0,
            started: Duration::ZERO,
            shortening: 1.0,
        }
    }
}

impl HoverFade {
    pub fn set_hovered(&mut self, hovered: bool, now: Duration) {
        let target = if hovered { 1.0 } else { 0.0 };
        if target == self.target {
            return;
        }
        let progress = EASE_BASIC(self.progress(now));
        let value = self.from + (self.target - self.from) * progress;
        self.shortening = if self.is_animating(now) {
            progress * self.shortening + 1.0 - self.shortening
        } else {
            1.0
        };
        self.from = value;
        self.target = target;
        self.started = now;
    }

    pub fn value(&self, now: Duration) -> f32 {
        self.from + (self.target - self.from) * EASE_BASIC(self.progress(now))
    }

    pub fn is_animating(&self, now: Duration) -> bool {
        self.from != self.target && self.progress(now) < 1.0
    }

    fn progress(&self, now: Duration) -> f32 {
        let duration = BASIC.as_secs_f32() * self.shortening;
        if duration == 0.0 {
            1.0
        } else {
            (now.saturating_sub(self.started).as_secs_f32() / duration).min(1.0)
        }
    }
}
