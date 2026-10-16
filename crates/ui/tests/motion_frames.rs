use gpui::{
    Animation, AnimationExt, AppContext as _, Context, HeadlessAppContext, IntoElement, Render,
    Styled, Window, div, px, size,
};
use serde_json::Value;
use std::{cell::Cell, rc::Rc, time::Duration};
use ui::motion::*;

const FIXTURES: &[&str] = &[
    include_str!("../../../fixtures/motion/button-hover.json"),
    include_str!("../../../fixtures/motion/stream-fade.json"),
    include_str!("../../../fixtures/motion/state-transitions.json"),
];

fn sample_time(sample: &Value) -> f64 {
    let rounded = sample[0].as_f64().unwrap();
    let frame_time = (rounded * 60.0 / 1000.0).round() * 1000.0 / 60.0;
    // The recorder seeks exact 60 Hz times, then rounds the stored times to 0.01 ms.
    if (rounded - frame_time).abs() < 0.005 {
        frame_time
    } else {
        rounded
    }
}

fn components(value: &str) -> Vec<f32> {
    if value == "none" {
        return vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    }
    let mut values: Vec<f32> = value
        .split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap())
        .collect();
    if value.starts_with("rgb") {
        for value in &mut values[..3] {
            *value /= 255.0;
        }
        if values.len() == 3 {
            values.push(1.0);
        }
    }
    values
}

fn easing(track: &Value) -> fn(f32) -> f32 {
    match track["case"].as_str().unwrap_or("") {
        "stream-fade-adaptive" | "stream-fade-default" | "stream-fade-delay" => EASE_STREAMING,
        "stream-image-enter" => |t| cubic_bezier(0.0, 0.0, 0.58, 1.0)(t),
        _ => match track["timing"]["easing"].as_str().unwrap() {
            "ease" => EASE_BASIC,
            "linear" => |t| t,
            "cubic-bezier(0.19, 1, 0.22, 1)" => EASE_ENTER,
            "cubic-bezier(0.65, 0, 0.35, 1)" => EASE_MOVE,
            "cubic-bezier(0.2, 0, 0, 1)" => EASE_COMPOSER,
            name => panic!("unhandled recorded easing: {name}"),
        },
    }
}

#[test]
fn easing_matches_original() {
    for fixture in FIXTURES {
        let fixture: Value = serde_json::from_str(fixture).unwrap();
        for track in fixture["tracks"].as_array().unwrap() {
            let samples = track["samples"].as_array().unwrap();
            let duration = track["timing"]["duration"].as_f64().unwrap() as f32;
            let delay = track["timing"]["delay"].as_f64().unwrap() as f32;
            let start = components(samples[0][1].as_str().unwrap());
            let end = components(samples.last().unwrap()[1].as_str().unwrap());
            let tolerance = match track["property"].as_str().unwrap() {
                "color" | "backgroundColor" => 1.0 / 255.0,
                "height" | "maxHeight" | "left" => 0.5,
                "rotate" => 90.0 * 5e-5,
                "opacity" | "transform" => 5e-5,
                property => panic!("unhandled recorded property: {property}"),
            };
            for sample in samples {
                let t = sample_time(sample) as f32;
                let progress = easing(track)(((t - delay) / duration).clamp(0.0, 1.0));
                let actual = components(sample[1].as_str().unwrap());
                assert_eq!(actual.len(), start.len());
                for (index, observed) in actual.iter().enumerate() {
                    let expected = start[index] + (end[index] - start[index]) * progress;
                    assert!(
                        (expected - observed).abs() <= tolerance,
                        "{} {} at {t}ms: {expected} != {observed}",
                        track["case"],
                        track["property"]
                    );
                }
            }
        }
    }
}

#[test]
fn discrete_and_sampled_easing_boundaries() {
    let spin = steps(60);
    for step in 1..60 {
        let boundary = step as f32 / 60.0;
        assert!((spin(boundary - 1e-5) - (step - 1) as f32 / 60.0).abs() < 1e-6);
        assert!((spin(boundary + 1e-5) - step as f32 / 60.0).abs() < 1e-6);
    }
    assert_eq!(spin(-1.0), 0.0);
    assert_eq!(spin(1.0), 1.0);
    assert_eq!(spin(2.0), 1.0);
    let spring = linear_stops(SPRING_SNAPPY_STOPS);
    for (index, pair) in SPRING_SNAPPY_STOPS.windows(2).enumerate() {
        let t = (index as f32 + 0.5) / (SPRING_SNAPPY_STOPS.len() - 1) as f32;
        assert!((spring(t) - (pair[0] + pair[1]) * 0.5).abs() < 1e-6);
    }
    assert_eq!(spring(-1.0), 0.0);
    assert_eq!(spring(2.0), 1.0);
    assert_eq!(linear_stops(&[0.0, 1.5, 1.0])(0.5), 1.5);
    let flat = cubic_bezier(0.0, 0.0, 0.0, 1.0);
    assert!((flat(0.001) - 0.028).abs() < 1e-6);
    assert_eq!(flat(-1.0), 0.0);
    assert_eq!(flat(2.0), 1.0);
    assert!(cubic_bezier(0.2, 2.0, 0.8, 2.0)(0.5) > 1.0);
}

#[test]
fn hover_follows_recording_and_reverses_without_jumping() {
    let fixture: Value = serde_json::from_str(FIXTURES[0]).unwrap();
    let mut fade = HoverFade::default();
    for (index, track) in fixture["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|track| track["property"] == "opacity")
        .enumerate()
    {
        let start = Duration::from_secs(index as u64);
        fade.set_hovered(index == 0, start);
        for sample in track["samples"].as_array().unwrap() {
            let now = start + Duration::from_secs_f64(sample_time(sample) / 1000.0);
            let expected: f32 = sample[1].as_str().unwrap().parse().unwrap();
            assert!((fade.value(now) - expected).abs() < 5e-5);
        }
        assert!(!fade.is_animating(start + BASIC));
    }
    fade.set_hovered(true, Duration::from_secs(2));
    let reverse = Duration::from_millis(2050);
    let before = fade.value(reverse);
    fade.set_hovered(true, reverse);
    assert_eq!(
        fade.value(reverse),
        before,
        "same target must not restart the fade"
    );
    fade.set_hovered(false, reverse);
    assert_eq!(fade.value(reverse), before);
    let half = reverse + BASIC.mul_f32(before * 0.5);
    assert!((fade.value(half) - before * (1.0 - EASE_BASIC(0.5))).abs() < 1e-5);
    assert_eq!(
        fade.value(reverse + BASIC.mul_f32(before) + Duration::from_micros(1)),
        0.0
    );
    assert!(!fade.is_animating(reverse + BASIC));
}

struct FrameProbe {
    value: Rc<Cell<f32>>,
    animation: Animation,
}

impl Render for FrameProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let value = self.value.clone();
        div().size(px(20.0)).with_animation(
            "stream-fade",
            self.animation.clone(),
            move |element, progress| {
                value.set(progress);
                element.opacity(progress)
            },
        )
    }
}

#[test]
fn native_frames_follow_the_clock_and_reduced_motion_finishes_immediately() {
    let mut cx = HeadlessAppContext::new(textsys::text_system());
    let value = Rc::new(Cell::new(-1.0));
    let window = cx
        .open_window(size(px(30.0), px(30.0)), |_, cx| {
            cx.new(|_| FrameProbe {
                value: value.clone(),
                // GPUI's unsynced animations use wall time, not the test scheduler.
                animation: Animation::new(STREAMING_TEXT)
                    .repeat_synced()
                    .with_easing(EASE_STREAMING),
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert!(value.get().abs() < 1e-3);
    let fixture: Value = serde_json::from_str(FIXTURES[1]).unwrap();
    let mut previous = 0.0;
    for sample in fixture["tracks"][0]["samples"]
        .as_array()
        .unwrap()
        .iter()
        .skip(1)
        .take(12)
    {
        let time = sample_time(sample);
        cx.advance_clock(Duration::from_secs_f64((time - previous) / 1000.0));
        cx.update_window(window.into(), |_, window, cx| {
            window.simulate_next_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        let observed: f32 = sample[1].as_str().unwrap().parse().unwrap();
        let frame = 16.667 / 700.0;
        let t = time as f32 / 700.0;
        assert!(value.get() >= EASE_STREAMING(t - frame) - 1e-3);
        assert!(value.get() <= EASE_STREAMING(t + frame) + 1e-3);
        assert!((value.get() - observed).abs() < 0.04);
        previous = time;
    }
    assert!(value.get() > 0.0 && value.get() < 1.0);
    cx.update(|cx| {
        assert!(!reduced(cx));
        cx.set_reduce_motion(true);
        assert!(reduced(cx));
    });
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.simulate_next_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        value.get(),
        0.0,
        "a reduced repeating animation stays at its start"
    );
    let callbacks = cx
        .update_window(window.into(), |_, window, cx| {
            window.simulate_next_frame(cx)
        })
        .unwrap();
    assert_eq!(
        callbacks, 0,
        "reduced motion must not schedule another frame"
    );
    let oneshot = Rc::new(Cell::new(-1.0));
    let window = cx
        .open_window(size(px(30.0), px(30.0)), |_, cx| {
            cx.new(|_| FrameProbe {
                value: oneshot.clone(),
                animation: Animation::new(STREAMING_TEXT).with_easing(EASE_STREAMING),
            })
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(oneshot.get(), 1.0, "a reduced oneshot starts at its end");
    assert_eq!(
        cx.update_window(window.into(), |_, window, cx| {
            window.simulate_next_frame(cx)
        })
        .unwrap(),
        0
    );
}
