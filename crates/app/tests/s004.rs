//! RS-132 and RS-137: replay sidebar resizing and titlebar visibility from the original app.
mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use serde_json::Value;

#[test]
fn proxy() {
    proxy_child();
}

fn point(value: &Value) -> (f32, f32) {
    (
        value["x"].as_f64().unwrap() as f32,
        value["y"].as_f64().unwrap() as f32,
    )
}

fn check(fx: &mut Fx, sample: &Value) -> Result<()> {
    ensure!(
        fx.sidebar_open() == !sample["handle"].is_null(),
        "sidebar visibility differs after {}",
        sample["afterStep"]
    );
    if let Some(x) = sample["handle"]["bbox"]["x"].as_f64() {
        ensure!(
            fx.shell(|shell| f32::from(shell.sidebar_width)) == x as f32 + 8.,
            "sidebar width differs after {}",
            sample["afterStep"]
        );
    }
    let shot = fx.shot()?;
    shot.save(format!(
        "{}/s004-{}.png",
        env!("CARGO_TARGET_TMPDIR"),
        sample["afterStep"]
    ))?;
    for pixel in sample["pixels"].as_array().context("pixel samples")? {
        let (x, y) = point(pixel);
        let expected: [u8; 4] = serde_json::from_value(pixel["rgba"].clone())?;
        let actual = shot.get_pixel((x * 2.) as u32, (y * 2.) as u32).0;
        ensure!(
            actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
            "step {} at ({x}, {y}): {actual:?}, expected {expected:?}",
            sample["afterStep"]
        );
    }
    if let Some(text) = sample["tooltip"]["text"].as_str() {
        let shown = fx
            .cx
            .update(|cx| ui::tooltip::shown(cx).cloned())
            .context("tooltip")?;
        ensure!(
            format!("{}\n{}", shown.label, shown.shortcut.unwrap_or_default()) == text,
            "tooltip differs from the original"
        );
    }
    Ok(())
}

#[test]
fn sidebar_resize_and_toggle_match_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("s004: the Metal renderer requires macOS");
        return Ok(());
    }
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s004.json"))?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    let samples = rec["result"]["dom"].as_array().context("DOM samples")?;
    check(&mut fx, &samples[0])?;
    let sent_before = fx.sent()?.len();
    for (i, step) in rec["steps"].as_array().context("steps")?.iter().enumerate() {
        match step["type"].as_str().context("step type")? {
            "drag" => fx.drag(point(&step["from"]), point(&step["to"]), 10)?,
            "click" => {
                let bbox = &step["target"]["bbox"];
                let (x, y) = point(bbox);
                fx.click((
                    x + bbox["w"].as_f64().unwrap() as f32 / 2.,
                    y + bbox["h"].as_f64().unwrap() as f32 / 2.,
                ))?;
            }
            "hover" => {
                fx.hover((640., 22.))?;
                fx.hover(point(&step["at"]))?;
                fx.cx.advance_clock(std::time::Duration::from_millis(800));
                fx.cx.run_until_parked();
            }
            other => anyhow::bail!("unsupported step {other}"),
        }
        check(&mut fx, &samples[i + 1])?;
    }
    ensure!(
        fx.sent()?.len() == sent_before,
        "sidebar action sent an app-server request"
    );
    println!("s004: recorded actions match sidebar visibility, boundary pixels, and tooltips");
    Ok(())
}
