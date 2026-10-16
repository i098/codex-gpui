//! Motion tokens from Codex 26.1007.30912, checked against `fixtures/motion/`.
//! Pass the easing functions to `gpui::Animation::with_easing`.
//! GPUI's `App::set_reduce_motion` switches decorative animations to their static state.
//! Button hover transitions remain active under reduced motion, as in the original.
//!
//! Run `cargo test -p ui --test motion_frames` to check all recorded tracks.
//! Native frame checks use `repeat_synced` for GPUI's test clock.
//! Unsynced GPUI animations use wall time; `HoverFade` accepts caller-supplied time.

use gpui::{
    App, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, RenderOnce,
    Rgba, StatefulInteractiveElement, Styled, Window, div,
};
use std::time::Duration;
use theme::{ActiveTheme as _, Appearance};

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

/// Chromium's `background-color` transition value at `p`: premultiplied interpolation in
/// oklab (CSS Color 4), so a blue-to-grey track stays within one 8-bit level of the original.
pub fn mix_oklab(from: Rgba, to: Rgba, p: f32) -> Rgba {
    fn linear(c: f32) -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    fn gamma(c: f32) -> f32 {
        let c = c.clamp(0., 1.);
        if c <= 0.003_130_8 {
            12.92 * c
        } else {
            1.055 * c.powf(1. / 2.4) - 0.055
        }
    }
    fn oklab(c: Rgba) -> [f32; 3] {
        let (r, g, b) = (linear(c.r), linear(c.g), linear(c.b));
        let l = (0.412_221_47 * r + 0.536_332_54 * g + 0.051_445_995 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        [
            0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        ]
    }
    let (a0, a1) = (from.a, to.a);
    let a = a0 + (a1 - a0) * p;
    if a == 0. {
        return Rgba {
            r: 0.,
            g: 0.,
            b: 0.,
            a,
        };
    }
    let (f, t) = (oklab(from), oklab(to));
    let [lab_l, lab_a, lab_b] = [0, 1, 2].map(|i| (f[i] * a0 * (1. - p) + t[i] * a1 * p) / a);
    let l = (lab_l + 0.396_337_78 * lab_a + 0.215_803_76 * lab_b).powi(3);
    let m = (lab_l - 0.105_561_346 * lab_a - 0.063_854_17 * lab_b).powi(3);
    let s = (lab_l - 0.089_484_18 * lab_a - 1.291_485_5 * lab_b).powi(3);
    Rgba {
        r: gamma(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_93 * s),
        g: gamma(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s),
        b: gamma(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
        a,
    }
}

/// A two-state CSS transition between 0 and 1, including CSS reversal shortening.
/// `set_hovered` is a button's 150 ms `ease` track; `transition` takes any duration and easing.
/// Supply monotonic time since a caller-owned epoch, from GPUI's clock in frame tests.
/// Keep one instance per control. Request frames only while `is_animating` is true.
#[derive(Clone, Debug)]
pub struct HoverFade {
    from: f32,
    target: f32,
    started: Duration,
    shortening: f32,
    duration: Duration,
    ease: fn(f32) -> f32,
}

impl Default for HoverFade {
    fn default() -> Self {
        Self::settled(0.0)
    }
}

impl HoverFade {
    /// At rest at `value`, so the first frame does not animate.
    pub fn settled(value: f32) -> Self {
        Self {
            from: value,
            target: value,
            started: Duration::ZERO,
            shortening: 1.0,
            duration: BASIC,
            ease: EASE_BASIC,
        }
    }

    pub fn set_hovered(&mut self, hovered: bool, now: Duration) {
        self.transition(if hovered { 1.0 } else { 0.0 }, now, BASIC, EASE_BASIC);
    }

    /// Starts a transition to `target` with the given track; the current value is the start.
    pub fn transition(
        &mut self,
        target: f32,
        now: Duration,
        duration: Duration,
        ease: fn(f32) -> f32,
    ) {
        if target == self.target {
            return;
        }
        let progress = (self.ease)(self.progress(now));
        let value = self.from + (self.target - self.from) * progress;
        self.shortening = if self.is_animating(now) {
            progress * self.shortening + 1.0 - self.shortening
        } else {
            1.0
        };
        self.from = value;
        self.target = target;
        self.started = now;
        self.duration = duration;
        self.ease = ease;
    }

    pub fn target(&self) -> f32 {
        self.target
    }

    pub fn value(&self, now: Duration) -> f32 {
        self.from + (self.target - self.from) * (self.ease)(self.progress(now))
    }

    pub fn is_animating(&self, now: Duration) -> bool {
        self.from != self.target && self.progress(now) < 1.0
    }

    fn progress(&self, now: Duration) -> f32 {
        let duration = self.duration.as_secs_f32() * self.shortening;
        if duration == 0.0 {
            1.0
        } else {
            (now.saturating_sub(self.started).as_secs_f32() / duration).min(1.0)
        }
    }
}

/// The visual child of an icon button. The outer button keeps its actions and tooltip.
/// Hover and press use separate CSS tracks, including reversal shortening.
/// These tracks stay active under reduced motion, as recorded in `refs/interactions/ms1.json`.
#[derive(IntoElement)]
pub(crate) struct ButtonVisual<E: Styled + IntoElement + 'static> {
    pub glyph: E,
    pub radius: Pixels,
    pub hover: Hsla,
    pub foreground: Hsla,
    pub selected: bool,
}

impl<E: Styled + IntoElement + 'static> RenderOnce for ButtonVisual<E> {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state("button-fade", cx, |_, cx| {
            (
                cx.background_executor().now(),
                HoverFade::default(),
                HoverFade::default(),
            )
        });
        let (epoch, hover, press) = state.read(cx);
        let now = cx.background_executor().now() - *epoch;
        let progress = hover.value(now);
        let pressed = press.value(now);
        if hover.is_animating(now) || press.is_animating(now) {
            window.request_animation_frame();
        }
        if progress > 0.0 && !self.selected {
            let from = Rgba::from(self.glyph.style().text.color.unwrap_or(self.foreground));
            let to = Rgba::from(self.foreground);
            self.glyph = self.glyph.text_color(Rgba {
                r: from.r + (to.r - from.r) * progress,
                g: from.g + (to.g - from.g) * progress,
                b: from.b + (to.b - from.b) * progress,
                a: from.a + (to.a - from.a) * progress,
            });
        }
        let active_alpha = match cx.theme().appearance {
            Appearance::Dark => 0.16,
            Appearance::Light => 0.12,
        };
        let mut fill = self.hover;
        fill.a = if self.selected {
            (active_alpha - fill.a) / (1.0 - fill.a) * pressed
        } else {
            (fill.a + (active_alpha - fill.a) * pressed) * progress
        };
        div()
            .id("button-visual")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .rounded(self.radius)
            .bg(fill)
            .on_hover({
                let state = state.clone();
                move |hovered, _, cx| {
                    state.update(cx, |(epoch, hover, press), cx| {
                        let now = cx.background_executor().now() - *epoch;
                        hover.set_hovered(*hovered, now);
                        if !hovered {
                            press.set_hovered(false, now);
                        }
                        cx.notify();
                    })
                }
            })
            .on_mouse_down(MouseButton::Left, {
                let state = state.clone();
                move |_, _, cx| {
                    state.update(cx, |(epoch, _, press), cx| {
                        press.set_hovered(true, cx.background_executor().now() - *epoch);
                        cx.notify();
                    })
                }
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                state.update(cx, |(epoch, _, press), cx| {
                    press.set_hovered(false, cx.background_executor().now() - *epoch);
                    cx.notify();
                })
            })
            .child(self.glyph)
    }
}
