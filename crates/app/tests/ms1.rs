//! MS1 replays the original Plugins hover, press, release, and leave at 60 Hz.
//! Run on macOS: `cargo test -p app --test ms1 -- --nocapture`.
//! The original keeps these button transitions active with reduced motion enabled.
//! The test checks dark and light themes within one 60 Hz frame and one 8-bit color level.
//! Composer hover stays instant. Custom diff controls remain outside this six-file slice.
#![cfg(target_os = "macos")]
mod common;

use anyhow::{Result, ensure};
use common::Fx;
use gpui::{Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, point, px};
use serde_json::Value;
use std::time::Duration;

fn pixel(image: &image::RgbaImage, x: u32, y: u32) -> u8 {
    image
        .get_pixel(x * image.width() / 1280, y * image.height() / 832)
        .0[0]
}

fn alpha(color: &str) -> f64 {
    if let Some((_, value)) = color.rsplit_once(" / ").or_else(|| {
        color
            .starts_with("rgba")
            .then(|| color.rsplit_once(", ").unwrap())
    }) {
        value.trim_end_matches(')').parse().unwrap()
    } else {
        1.0
    }
}

#[test]
fn recorded_button_frames() -> Result<()> {
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/ms1.json"))?;
    for (theme, reduced) in [
        ("dark", false),
        ("dark", true),
        ("light", false),
        ("light", true),
    ] {
        let run = recording["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|run| run["viewport"]["theme"] == theme)
            .unwrap();
        let foreground: f64 = run["rest"]["color"]
            .as_str()
            .unwrap()
            .split_once('(')
            .unwrap()
            .1
            .split(',')
            .next()
            .unwrap()
            .parse()?;
        let rest_alpha = alpha(run["rest"]["color"].as_str().unwrap());
        let mut fx = Fx::with(theme, (1280., 832.), None, |_| {})?;
        fx.cx.update(|cx| cx.set_reduce_motion(reduced));
        fx.hover((640., 22.))?;
        let rest_image = fx.shot()?;
        let rest = pixel(&rest_image, 12, 202) as f64;
        let glyph = (16..36)
            .flat_map(|x| (192..212).map(move |y| (x, y)))
            .max_by_key(|&(x, y)| (pixel(&rest_image, x, y) as i32 - rest as i32).abs())
            .unwrap();
        let coverage = (pixel(&rest_image, glyph.0, glyph.1) as f64 - rest)
            / ((foreground - rest) * rest_alpha);
        for track in run["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["reduced"] == reduced)
        {
            let action = track["action"].as_str().unwrap();
            match action {
                "hover" => fx.hover((26., 202.))?,
                "leave" => fx.hover((640., 22.))?,
                "press" => fx.input(PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position: point(px(26.), px(202.)),
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }))?,
                "release" => fx.input(PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position: point(px(26.), px(202.)),
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }))?,
                _ => panic!("unknown recorded action: {action}"),
            }
            let expected = |sample: &Value| {
                let fill = alpha(sample["background"].as_str().unwrap())
                    * sample["opacity"].as_f64().unwrap();
                let background = rest + (foreground - rest) * fill;
                let glyph = background
                    + (foreground - background)
                        * alpha(sample["color"].as_str().unwrap())
                        * coverage;
                [background, glyph]
            };
            let samples = track["samples"].as_array().unwrap();
            let mut elapsed = Duration::ZERO;
            for (index, sample) in samples.iter().enumerate() {
                let time = Duration::from_secs_f64(sample["ms"].as_f64().unwrap() / 1000.);
                fx.advance(time - elapsed)?;
                elapsed = time;
                let image = fx.shot()?;
                let observed = [
                    pixel(&image, 12, 202) as f64,
                    pixel(&image, glyph.0, glyph.1) as f64,
                ];
                // The inventory allows one 60 Hz frame and one 8-bit color level.
                let before = expected(&samples[index.saturating_sub(1)]);
                let after = expected(&samples[(index + 1).min(samples.len() - 1)]);
                for channel in 0..2 {
                    let low = before[channel].min(after[channel]) - 1.0;
                    let high = before[channel].max(after[channel]) + 1.0;
                    ensure!(
                        (low..=high).contains(&observed[channel]),
                        "{theme} {action} reduced={reduced} at {time:?}: channel {channel} is {}, expected {low}..={high}",
                        observed[channel]
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn sidebar_row_hover_stays_instant() -> Result<()> {
    let mut fx = Fx::new()?;
    let at = fx.at("home", "Change the greeting")?;
    fx.hover((640., 22.))?;
    let rest = fx.shot()?;
    fx.hover((100., at.1))?;
    let first = fx.shot()?;
    fx.advance(Duration::from_millis(150))?;
    let settled = fx.shot()?;
    let y = at.1 as u32;
    ensure!(pixel(&first, 70, y) != pixel(&rest, 70, y));
    ensure!(pixel(&first, 70, y) == pixel(&settled, 70, y));
    Ok(())
}
