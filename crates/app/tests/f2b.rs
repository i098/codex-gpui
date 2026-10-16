//! Replay the recorded permission-menu focus, search, and Tab behavior.

use anyhow::{Result, ensure};
use common::Fx;
use serde_json::Value;
use std::time::Duration;

mod common;

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn permission_menu_matches_original_focus() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/f2b.json"))?;
    let mut fx = Fx::captured("dark", (1280., 832.), common::script(None, |_| {})?)?;
    let before_requests = fx.sent()?;
    let mut unhighlighted = None;
    for (i, step) in recording["steps"].as_array().unwrap().iter().enumerate() {
        if let Some(ms) = step["beforeMs"].as_u64() {
            std::thread::sleep(Duration::from_millis(ms));
        }
        match step["type"].as_str().unwrap() {
            "click" => {
                let b = &step["target"]["bbox"];
                fx.click((
                    (b["x"].as_f64().unwrap() + b["w"].as_f64().unwrap() / 2.) as f32,
                    (b["y"].as_f64().unwrap() + b["h"].as_f64().unwrap() / 2.) as f32,
                ))?;
            }
            // The existing port menu is 320 px wide. Use the same item and heading, not the
            // original's coordinates within its 417 px menu.
            "hover" if step["target"]["text"] == "Full access" => fx.hover((790., 684.))?,
            "hover" => fx.hover((700., 548.))?,
            "key" => {
                let key = match step["key"].as_str().unwrap() {
                    "End" => "end",
                    "Home" => "home",
                    "ArrowDown" => "down",
                    "Enter" => "enter",
                    "Space" => "space",
                    "Escape" => "escape",
                    "Tab" if step["modifiers"][0] == "shift" => "shift-tab",
                    "Tab" => "tab",
                    key => key,
                };
                fx.keys(key)?;
            }
            other => anyhow::bail!("unsupported recorded step {other}"),
        }
        let expected = &recording["result"]["dom"][i];
        let state = fx.cx.update_window(fx.window, |_, window, cx| {
            fx.root.read(cx).composer(cx).read(cx).menu_state(window)
        })?;
        ensure!(
            state.0 == expected["open"].as_bool().unwrap(),
            "step {i}: open {state:?}"
        );
        ensure!(
            state.1.map_or(-1, |n| n as i64) == expected["focusedItem"].as_i64().unwrap(),
            "step {i}: focus {state:?}"
        );
        ensure!(
            state.2 == (expected["active"]["aria"] == "Change permissions"),
            "step {i}: trigger focus {state:?}"
        );
        let image = fx.cx.capture_screenshot(fx.window)?;
        if i == 0 {
            unhighlighted = Some(image.clone());
        }
        if let Some(item) = state.1 {
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
            let scale = image.width() as f32 / 1280.;
            let x = (790. * scale) as u32;
            let y = ([580., 628., 684., 740.][item] * scale) as u32;
            let base = unhighlighted.as_ref().unwrap().get_pixel(x, y);
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
        recording["result"]["requests"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    ensure!(
        fx.sent()? == before_requests,
        "menu navigation sent an app-server request"
    );
    Ok(())
}
