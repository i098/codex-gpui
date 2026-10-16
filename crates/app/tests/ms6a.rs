//! MS6a replays the original's switch toggle and `Worked for` disclosure at 60 Hz
//! (`refs/interactions/ms6a.json`). Run on macOS: `cargo test -p app --test ms6a -- --nocapture`.
//! Switch: the thumb centre follows the `translate` track and the uncovered track end follows
//! the `background-color` track (150 ms `ease-out` both ways, also under reduced motion).
//! Disclosure: the chevron's principal axis follows the `rotate` track; the answer below moves
//! down by the content height, which follows the content's `opacity` track; the content is
//! gone at the end of the close track. Under reduced motion the content is instant.
//! Tolerance: one 8-bit level, 0.5 CSS px, 2.5 deg; one 60 Hz frame for the content height only.
//! Keyboard toggling (Enter, Space) is recorded but not replayed: the port's controls are not
//! focusable yet.
#![cfg(target_os = "macos")]
mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, GREETING};
use image::RgbaImage;
use serde_json::Value;
use std::time::Duration;

fn recording() -> Result<Value> {
    Ok(serde_json::from_str(include_str!(
        "../../../refs/interactions/ms6a.json"
    ))?)
}

/// Device pixel (the window is 2x) as `[r, g, b]`.
fn px3(image: &RgbaImage, x: u32, y: u32) -> [f64; 3] {
    let p = image.get_pixel(x, y).0;
    [p[0] as f64, p[1] as f64, p[2] as f64]
}

fn ms(sample: &Value) -> Duration {
    Duration::from_secs_f64(sample[0].as_f64().unwrap() / 1000.)
}

fn value(sample: &Value) -> f64 {
    sample[1].as_f64().unwrap()
}

/// Steps the clock through `samples` (`[ms, ...]`) from `elapsed` and calls `check` at each.
fn replay(
    fx: &mut Fx,
    samples: &[Value],
    mut elapsed: Duration,
    mut check: impl FnMut(&mut Fx, usize, &Value) -> Result<()>,
) -> Result<Duration> {
    for (index, sample) in samples.iter().enumerate() {
        let time = ms(sample);
        if time < elapsed {
            continue;
        }
        fx.advance(time - elapsed)?;
        elapsed = time;
        check(fx, index, sample)?;
    }
    Ok(elapsed)
}

/// The recorded values `frames` before and after `index`: the allowed range. The samples are
/// exact (paused, seeked) and the port clock is exact, so CSS tracks get no time slack; the
/// live content height lags its track by up to one frame.
fn bracket(
    samples: &[Value],
    index: usize,
    frames: usize,
    f: impl Fn(&Value) -> f64,
) -> (f64, f64) {
    let before = f(&samples[index.saturating_sub(frames)]);
    let after = f(&samples[(index + frames).min(samples.len() - 1)]);
    (before.min(after), before.max(after))
}

fn within(observed: f64, (low, high): (f64, f64), eps: f64) -> bool {
    (low - eps..=high + eps).contains(&observed)
}

// --- Switch ---

/// `Show ChatGPT in the menu bar` on Settings > General at 1280x832 (refs/settings spec
/// `[1123.25, 709.13, 32, 20]`), in device px; the track starts half a device px in.
const TRACK: [u32; 4] = [2246, 1418, 64, 40];
const TRACK_LEFT: f64 = 2246.5;

/// Thumb inset from the track's left edge in CSS px, from the white pixels' centre.
fn thumb_inset(image: &RgbaImage) -> f64 {
    let (mut sum, mut n) = (0., 0.);
    for y in TRACK[1] + 6..TRACK[1] + TRACK[3] - 6 {
        for x in TRACK[0]..TRACK[0] + TRACK[2] {
            let [r, g, b] = px3(image, x, y);
            if r + g + b > 3. * 200. {
                sum += x as f64 + 0.5;
                n += 1.;
            }
        }
    }
    (sum / n - TRACK_LEFT) / 2. - 8.
}

/// `color` (`[r, g, b, a]` 0-255, alpha 0-1) over `card`.
fn over(color: &Value, card: [f64; 3]) -> [f64; 3] {
    let c = color.as_array().unwrap();
    let a = c[3].as_f64().unwrap();
    [0, 1, 2].map(|i| card[i] * (1. - a) + c[i].as_f64().unwrap() * a)
}

/// Premultiplied sRGB mix of `from` to `to` at `p`, over `card`.
fn mix(from: &Value, to: &Value, p: f64, card: [f64; 3]) -> [f64; 3] {
    let (f, t) = (from.as_array().unwrap(), to.as_array().unwrap());
    let (fa, ta) = (f[3].as_f64().unwrap(), t[3].as_f64().unwrap());
    let a = fa + (ta - fa) * p;
    [0, 1, 2].map(|i| {
        let c = (f[i].as_f64().unwrap() * fa * (1. - p) + t[i].as_f64().unwrap() * ta * p) / a;
        card[i] * (1. - a) + c * a
    })
}

#[test]
fn switch_follows_recorded_tracks() -> Result<()> {
    let recording = recording()?;
    let switch = &recording["switch"];
    let steps = switch["steps"].as_array().unwrap();
    let off = &steps.iter().find(|s| s["step"] == "click-off").unwrap()["tracks"];
    let tracks = off.as_array().unwrap();
    let translate = &tracks
        .iter()
        .find(|t| t["property"] == "translate")
        .unwrap()["samples"];
    let color = &tracks
        .iter()
        .find(|t| t["property"] == "background-color")
        .unwrap()["samples"];
    let (translate, color) = (translate.as_array().unwrap(), color.as_array().unwrap());
    let (blue, grey) = (&color[0][1], &color[color.len() - 1][1]);
    for reduced in [false, true] {
        let mut fx = Fx::new()?;
        fx.cx.update(|cx| cx.set_reduce_motion(reduced));
        fx.keys("cmd-,")?;
        fx.hover((640., 22.))?;
        let rest = fx.shot()?;
        // The card right of the track.
        let card = px3(&rest, TRACK[0] + TRACK[2] + 8, TRACK[1] + 20);
        let at = fx.at("settings", switch["label"].as_str().unwrap())?;
        let inset = thumb_inset(&rest);
        ensure!((inset - 14.).abs() <= 0.5, "rest thumb inset {inset}");
        // The on direction uses the same declaration: the off track mirrored (the recorded
        // `stub-revert-on` step shows the same easing and duration from 3.48 px to 14 px).
        for mirror in [false, true] {
            let name = if mirror { "click-on" } else { "click-off" };
            fx.click(at)?;
            replay(&mut fx, translate, Duration::ZERO, |fx, index, sample| {
                let image = fx.shot()?;
                let expected_inset = |s: &Value| if mirror { 16. - value(s) } else { value(s) };
                let observed = thumb_inset(&image);
                ensure!(
                    within(observed, bracket(translate, index, 0, expected_inset), 0.5),
                    "{name} reduced={reduced} at {:?}: thumb inset {observed:.2} px, expected {:?}",
                    ms(sample),
                    bracket(translate, index, 0, expected_inset)
                );
                // The track end the thumb (16 px plus its shadow) does not cover.
                if value(sample) <= 10. {
                    let x = if mirror {
                        TRACK[0] + 6
                    } else {
                        TRACK[0] + TRACK[2] - 6
                    };
                    let observed = px3(&image, x, TRACK[1] + 20);
                    let expected = |s: &Value| {
                        if mirror {
                            mix(grey, blue, (14. - value(s)) / 12., card)
                        } else {
                            over(
                                &color[translate.iter().position(|t| t == s).unwrap()][1],
                                card,
                            )
                        }
                    };
                    for channel in 0..3 {
                        let range = bracket(translate, index, 0, |s| expected(s)[channel]);
                        ensure!(
                            within(observed[channel], range, 1.5),
                            "{name} reduced={reduced} at {:?}: track channel {channel} is {}, expected {range:?}",
                            ms(sample),
                            observed[channel]
                        );
                    }
                }
                Ok(())
            })?;
            fx.advance(Duration::from_millis(100))?;
        }
    }
    Ok(())
}

// --- Disclosure ---

/// Orientation of the glyph's principal axis in `window` (device px), in degrees, from the
/// second moments of the pixels that differ from `bg`. Rotating the glyph rotates the axis.
fn axis_angle(image: &RgbaImage, window: [u32; 4], bg: [f64; 3]) -> f64 {
    let weight = |x, y| {
        let p = px3(image, x, y);
        let d = (0..3).map(|i| (p[i] - bg[i]).abs()).sum::<f64>() / 3.;
        if d > 24. { d } else { 0. }
    };
    let pixels = || {
        (window[1]..window[1] + window[3])
            .flat_map(|y| (window[0]..window[0] + window[2]).map(move |x| (x, y)))
    };
    let (mut m00, mut m10, mut m01) = (0., 0., 0.);
    for (x, y) in pixels() {
        let w = weight(x, y);
        m00 += w;
        m10 += w * x as f64;
        m01 += w * y as f64;
    }
    let (cx, cy) = (m10 / m00, m01 / m00);
    let (mut u20, mut u02, mut u11) = (0., 0., 0.);
    for (x, y) in pixels() {
        let w = weight(x, y);
        let (dx, dy) = (x as f64 - cx, y as f64 - cy);
        u20 += w * dx * dx;
        u02 += w * dy * dy;
        u11 += w * dx * dy;
    }
    (0.5 * (2. * u11).atan2(u20 - u02)).to_degrees()
}

/// Angle difference on the 180 degree axis circle.
fn axis_delta(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(180.);
    d.min(180. - d)
}

/// Vertical offset (device px, at most `max`) at which `strip` of `reference` best matches
/// `image`.
fn strip_offset(reference: &RgbaImage, image: &RgbaImage, strip: [u32; 4], max: u32) -> u32 {
    (0..=max)
        .min_by_key(|&dy| {
            let mut error = 0u64;
            for y in strip[1]..strip[1] + strip[3] {
                for x in strip[0]..strip[0] + strip[2] {
                    let a = px3(reference, x, y);
                    let b = px3(image, x, y + dy);
                    error += (0..3).map(|i| (a[i] - b[i]).abs() as u64).sum::<u64>();
                }
            }
            error
        })
        .unwrap()
}

/// The collapsed disclosure in the greeting thread, in device px: `(label_left, hairline_y,
/// chevron_center)`. The hairline is the first full-width row of one non-background colour;
/// the chevron is the last glyph cluster of the row above it (the path starts 5.08 px into
/// its 14 px box, whose centre is 14.5 px above the hairline).
fn locate(rest: &RgbaImage) -> Result<(u32, u32, (f64, f64))> {
    let bg = px3(rest, 1000, 150);
    let differs = |a: [f64; 3], b: [f64; 3]| (0..3).map(|i| (a[i] - b[i]).abs()).sum::<f64>() > 12.;
    let hairline = (300..1000)
        .find(|&y| {
            let c = px3(rest, 1100, y);
            differs(c, bg) && (1100..2300).step_by(4).all(|x| px3(rest, x, y) == c)
        })
        .context("no hairline")?;
    let occupied: Vec<bool> = (0..2560)
        .map(|x| (hairline - 50..hairline - 8).any(|y| differs(px3(rest, x, y), bg)))
        .collect();
    let mut clusters = Vec::new();
    let mut x = 860;
    while x < 1400 {
        if occupied[x] {
            let start = x;
            while x < 1400 && (x..x + 12).any(|i| occupied[i]) {
                x += 1;
            }
            clusters.push((start, x));
        }
        x += 1;
    }
    let label_left = clusters.first().context("no label")?.0 as u32;
    let chevron_left = clusters.last().context("no chevron")?.0 as f64;
    Ok((
        label_left,
        hairline,
        (chevron_left + 2. * 1.92, hairline as f64 - 2. * 14.5),
    ))
}

#[test]
fn disclosure_follows_recorded_tracks() -> Result<()> {
    let recording = recording()?;
    let steps = recording["disclosure"]["steps"].as_array().unwrap();
    let mut located = None;
    for reduced in [false, true] {
        let mut fx = Fx::new()?;
        fx.cx.update(|cx| cx.set_reduce_motion(reduced));
        fx.click(fx.at("home", "Change the greeting")?)?;
        fx.wait_thread(GREETING)?;
        fx.hover((640., 22.))?;
        let rest = fx.shot()?;
        let (label_left, hairline, center) = *located.get_or_insert(locate(&rest)?);
        let window = [center.0 as u32 - 11, center.1 as u32 - 11, 22, 22];
        let at = ((label_left as f32 + 40.) / 2., center.1 as f32 / 2.);
        let bg = px3(&rest, window[0] + 1, window[1] + 1);
        let rest_angle = axis_angle(&rest, window, bg);
        ensure!(
            axis_delta(rest_angle, 90.) < 3.,
            "rest chevron axis {rest_angle:.1}"
        );
        // The answer's first text line under the hairline: the strip the content pushes down.
        let first_text = (hairline + 4..hairline + 120)
            .find(|&y| {
                (label_left..label_left + 600)
                    .any(|x| px3(&rest, x, y).iter().sum::<f64>() > bg.iter().sum::<f64>() + 90.)
            })
            .context("no answer text")?;
        let strip = [label_left, first_text - 2, 600, 36];
        // The content's natural height: fully open, then closed again.
        fx.click(at)?;
        fx.advance(Duration::from_millis(400))?;
        let natural = strip_offset(&rest, &fx.shot()?, strip, 400);
        ensure!(natural > 40, "open content height {natural} device px");
        fx.click(at)?;
        fx.advance(Duration::from_millis(400))?;
        ensure!(
            strip_offset(&rest, &fx.shot()?, strip, 400) == 0,
            "content left after close"
        );
        let max = natural + 4;
        let natural = natural as f64;
        for step in steps
            .iter()
            .filter(|s| s["step"].as_str().unwrap().starts_with("click-"))
        {
            let name = step["step"].as_str().unwrap();
            let open = name.contains("open");
            let tracks = step["tracks"].as_array().unwrap();
            let angles = &tracks.iter().find(|t| t["property"] == "rotate").unwrap()["samples"];
            let content = &tracks.iter().find(|t| t["property"] == "opacity").unwrap()["samples"];
            let (angles, content) = (angles.as_array().unwrap(), content.as_array().unwrap());
            fx.click(at)?;
            let height = |fx: &mut Fx| -> Result<f64> {
                Ok(strip_offset(&rest, &fx.shot()?, strip, max) as f64)
            };
            if reduced {
                let h = height(&mut fx)?;
                ensure!(
                    h == if open { natural } else { 0. },
                    "{name} reduced: content {h} of {natural} device px"
                );
            }
            // Chevron: 150 ms, checked at its own samples; the content alongside.
            let elapsed = replay(&mut fx, angles, Duration::ZERO, |fx, index, sample| {
                let image = fx.shot()?;
                let observed = axis_delta(axis_angle(&image, window, bg), rest_angle);
                let range = bracket(angles, index, 0, value);
                ensure!(
                    within(observed, range, 2.5),
                    "{name} reduced={reduced} at {:?}: chevron at {observed:.1} deg, expected {range:?}",
                    ms(sample)
                );
                if !reduced {
                    let h = strip_offset(&rest, &image, strip, max) as f64;
                    let range = bracket(content, index, 1, |s| value(s) * natural);
                    ensure!(
                        within(h, range, 1.5),
                        "{name} at {:?}: content {h} device px, expected {range:?}",
                        ms(sample)
                    );
                }
                Ok(())
            })?;
            // The content's remaining 300 ms open track.
            replay(&mut fx, content, elapsed, |fx, index, sample| {
                if reduced {
                    return Ok(());
                }
                let h = height(fx)?;
                let range = bracket(content, index, 1, |s| value(s) * natural);
                ensure!(
                    within(h, range, 1.5),
                    "{name} at {:?}: content {h} device px, expected {range:?}",
                    ms(sample)
                );
                Ok(())
            })?;
            let h = height(&mut fx)?;
            ensure!(
                h == if open { natural } else { 0. },
                "{name} settled: content {h} of {natural} device px"
            );
        }
    }
    Ok(())
}
