//! Replay the recorded model-picker keyboard open and `Speed` submenu steps
//! (`refs/interactions/f2c.json`: HC-104, HC-105 for the model trigger).

use anyhow::{Result, ensure};
use common::Fx;
use serde_json::Value;
use std::time::Duration;

mod common;

#[test]
fn proxy() {
    common::proxy_child();
}

/// Centre of a recorded `target.bbox`.
fn centre(step: &Value) -> (f32, f32) {
    let b = &step["target"]["bbox"];
    let n = |k: &str| b[k].as_f64().unwrap() as f32;
    (n("x") + n("w") / 2., n("y") + n("h") / 2.)
}

#[test]
fn model_picker_matches_original_focus() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/f2c.json"))?;
    let mut fx = Fx::captured("dark", (1280., 832.), common::script(None, |_| {})?)?;
    let before = fx.sent()?.len();
    // The pixel of the `Fast` row while `Standard` is the highlighted one (step 7).
    let mut fast_unhighlighted = None;
    for (i, step) in recording["steps"].as_array().unwrap().iter().enumerate() {
        let key = step["key"].as_str();
        match step["type"].as_str().unwrap() {
            "click" => fx.click(centre(step))?,
            "hover" => fx.hover(centre(step))?,
            "key" => fx.keys(match key.unwrap() {
                "Tab" => "tab",
                "Enter" => "enter",
                "Space" => "space",
                "Escape" => "escape",
                "ArrowDown" => "down",
                "ArrowLeft" => "left",
                "ArrowRight" => "right",
                other => anyhow::bail!("unsupported key {other}"),
            })?,
            other => anyhow::bail!("unsupported recorded step {other}"),
        }
        if let Some(ms) = step["waitMs"].as_u64() {
            fx.advance(Duration::from_millis(ms))?;
        }
        let expected = &recording["result"]["dom"][i];
        let (menu, speed, trigger) = fx.cx.update_window(fx.window, |_, window, cx| {
            let composer = fx.root.read(cx).composer(cx);
            let composer = composer.read(cx);
            (
                composer.menu_state(window),
                composer.speed_menu_state(window),
                composer.model_trigger_focused(window),
            )
        })?;
        ensure!(
            menu.0 == expected["open"].as_bool().unwrap(),
            "step {i}: open {menu:?}"
        );
        ensure!(
            menu.1.map_or(-1, |n| n as i64) == expected["focusedItem"].as_i64().unwrap(),
            "step {i}: menu focus {menu:?}"
        );
        ensure!(
            speed.0 == expected["speedOpen"].as_bool().unwrap(),
            "step {i}: speed submenu open {speed:?}"
        );
        ensure!(
            speed.1.map_or(-1, |n| n as i64) == expected["speedItem"].as_i64().unwrap(),
            "step {i}: speed focus {speed:?}"
        );
        // Escape's focus target (HC-103) is not an F2c row: the original returns to the
        // editor after a keyboard close, and to the trigger after a hover-open.
        if key != Some("Escape") {
            let original_trigger = expected["active"]["tag"] == "button"
                && expected["active"]["text"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("GPT-6 Astra"));
            ensure!(
                trigger == original_trigger,
                "step {i}: trigger focus {trigger}"
            );
        }
        // The highlighted submenu item: `rgba(255, 255, 255, 0.08)` over the `Fast` row, at
        // the recorded position (1100, 740): right of its text, left of the check column.
        if speed.1 == Some(0) {
            fast_unhighlighted = Some(fx.shot()?);
        } else if speed.1 == Some(1) {
            let rgba = expected["active"]["computed"]["backgroundColor"]
                .as_str()
                .unwrap();
            let channels: Vec<f32> = rgba
                .strip_prefix("rgba(")
                .unwrap()
                .trim_end_matches(')')
                .split(',')
                .map(|s| s.trim().parse())
                .collect::<Result<_, _>>()?;
            let image = fx.shot()?;
            let scale = image.width() as f32 / 1280.;
            let (x, y) = ((1100. * scale) as u32, (740. * scale) as u32);
            let base = fast_unhighlighted.as_ref().unwrap().get_pixel(x, y);
            let painted = image.get_pixel(x, y);
            for channel in 0..3 {
                let blended =
                    channels[channel] * channels[3] + base[channel] as f32 * (1. - channels[3]);
                ensure!(
                    (painted[channel] as f32 - blended).abs() <= 2.,
                    "step {i}: highlight {painted:?}, expected {blended}"
                );
            }
        }
    }
    ensure!(
        fx.sent()?[before..] == *recording["result"]["requests"].as_array().unwrap(),
        "menu navigation sent an app-server request"
    );
    Ok(())
}
