//! Replay the original Home menu interactions in `refs/interactions/f2a.json`.

use anyhow::{Result, ensure};
use common::Fx;
use serde_json::Value;
use ui::composer::Permission;

mod common;

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn permissions_popover() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = Fx::captured("dark", (1280., 832.), common::script(None, |_| {})?)?;
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/f2a.json"))?;
    let steps = rec["steps"].as_array().unwrap();
    let observed = rec["result"]["dom"].as_array().unwrap();
    let before_requests = fx.sent()?;
    let mut unhighlighted = None;
    for (i, step) in steps.iter().enumerate() {
        match step["type"].as_str().unwrap() {
            "click" => {
                let b = &step["target"]["bbox"];
                fx.click((
                    (b["x"].as_f64().unwrap() + b["w"].as_f64().unwrap() / 2.) as f32,
                    (b["y"].as_f64().unwrap() + b["h"].as_f64().unwrap() / 2.) as f32,
                ))?;
            }
            "key" => fx.keys(match step["key"].as_str().unwrap() {
                "ArrowDown" => "down",
                "ArrowUp" => "up",
                "Escape" => "escape",
                "Enter" => "enter",
                "Space" => "space",
                other => anyhow::bail!("unsupported key {other}"),
            })?,
            other => anyhow::bail!("unsupported step {other}"),
        }
        let expected = &observed[i];
        let (state, permission) = fx.cx.update_window(fx.window, |_, window, cx| {
            let composer = fx.root.read(cx).composer(cx);
            let composer = composer.read(cx);
            (composer.menu_state(window), composer.permission())
        })?;
        ensure!(
            state.0 == expected["open"].as_bool().unwrap(),
            "step {i}: open state {state:?}"
        );
        ensure!(
            state.1.map_or(-1, |n| n as i64) == expected["focusedItem"].as_i64().unwrap(),
            "step {i}: item focus {state:?}"
        );
        ensure!(
            state.2 == (expected["active"]["aria"] == "Change permissions"),
            "step {i}: trigger focus {state:?}"
        );
        let label = match permission {
            Permission::Ask => "Ask for approval",
            Permission::ApproveForMe => "Approve for me",
            Permission::FullAccess => "Full access",
            Permission::Custom => "Custom",
        };
        ensure!(
            label == expected["trigger"]["text"].as_str().unwrap(),
            "step {i}: selected {label}"
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
            // The port keeps its existing 320 px menu. Sample the same empty area in each row.
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
                    "step {i}: focus fill {painted:?}, expected {blended}"
                );
            }
        }
        if state.2 {
            let shadow = expected["active"]["computed"]["boxShadow"]
                .as_str()
                .unwrap();
            let color: Vec<u8> = shadow
                .rsplit_once("rgb(")
                .unwrap()
                .1
                .split(')')
                .next()
                .unwrap()
                .split(',')
                .map(|s| s.trim().parse())
                .collect::<Result<_, _>>()?;
            let scale = image.width() as f32 / 1280.;
            let pixel = image.get_pixel((481. * scale) as u32, (790. * scale) as u32);
            ensure!(
                pixel.0[..3] == color,
                "step {i}: trigger ring {pixel:?}, expected {color:?}"
            );
        }
    }
    ensure!(rec["result"]["requests"].as_array().unwrap().is_empty());
    ensure!(
        fx.sent()? == before_requests,
        "menu input sent an app-server request"
    );
    Ok(())
}
