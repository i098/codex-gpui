//! Replays F1b on the real Root through the shared app-server capture helper.
//! Release outside cancels the row click before the Tab sequence.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, ROOT, script};
use gpui::{
    KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput,
    point, px,
};
use serde_json::Value;
use std::{fs, time::Duration};

#[test]
fn proxy() {
    common::proxy_child();
}

fn rect(value: &Value) -> Result<[f32; 4]> {
    let number = |key: &str| {
        value[key]
            .as_f64()
            .map(|n| n as f32)
            .with_context(|| format!("missing {key}"))
    };
    Ok([number("x")?, number("y")?, number("w")?, number("h")?])
}

fn step(fx: &mut Fx, step: &Value) -> Result<()> {
    if step["type"] == "key" {
        let prefix = if step["shift"] == true { "shift-" } else { "" };
        let key = step["key"].as_str().context("key")?.to_lowercase();
        let key = Keystroke::parse(&format!("{prefix}{key}"))?;
        fx.cx.update_window(fx.window, |_, window, cx| {
            window.dispatch_keystroke(key.clone(), cx);
            window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke: key }), cx);
        })?;
    } else {
        let [x, y, w, h] = rect(&step["target"]["bbox"])?;
        let at = (x + w / 2., y + h / 2.);
        let phase = step["phase"].as_str();
        match (step["type"].as_str(), phase) {
            (Some("hover"), _) => fx.hover(at)?,
            (Some("click"), None) => fx.click(at)?,
            (Some("click"), Some("down")) => {
                fx.hover(at)?;
                fx.input(PlatformInput::MouseDown(MouseDownEvent {
                    position: point(px(at.0), px(at.1)),
                    button: MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }))?;
            }
            (Some("click"), Some("up")) => fx.input(PlatformInput::MouseUp(MouseUpEvent {
                position: point(px(at.0), px(at.1)),
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
                click_count: 1,
            }))?,
            _ => anyhow::bail!("unsupported step {step}"),
        }
    }
    fx.cx.run_until_parked();
    fx.cx.advance_clock(Duration::from_millis(
        step["waitMs"].as_u64().unwrap_or(120),
    ));
    fx.cx.run_until_parked();
    Ok(())
}

#[test]
fn sidebar_focus_and_tooltips_match_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("f1b: requires the macOS headless renderer");
        return Ok(());
    }
    let recording: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/f1b.json"))?)?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.shot()?;
    let before = fx.sent()?.len();
    for (index, input) in recording["steps"]
        .as_array()
        .context("steps")?
        .iter()
        .enumerate()
    {
        step(&mut fx, input)?;
        let image = fx.shot()?;
        let observation = &recording["result"]["dom"][index];
        let scale = image.width() as f32 / 1280.;
        for sample in observation["pixels"].as_array().into_iter().flatten() {
            let x = sample["x"].as_f64().context("pixel x")? as f32;
            let y = sample["y"].as_f64().context("pixel y")? as f32;
            let got = image.get_pixel((x * scale) as u32, (y * scale) as u32).0;
            let want = sample["rgb"].as_array().context("pixel rgb")?;
            let tolerance = sample["tolerance"].as_u64().unwrap_or(1) as u8;
            ensure!(
                (0..3).all(|c| got[c].abs_diff(want[c].as_u64().unwrap() as u8) <= tolerance),
                "step {index}: ({x}, {y}) {got:?} != {want:?}"
            );
        }
        for action in observation["assertActions"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let [x, y, w, h] = rect(&action["bbox"])?;
            let mut brightest = 0;
            for y in ((y + 5.) * scale) as u32..((y + h - 5.) * scale) as u32 {
                for x in ((x + 5.) * scale) as u32..((x + w - 5.) * scale) as u32 {
                    let p = image.get_pixel(x, y).0;
                    if p[0] == p[1] && p[1] == p[2] {
                        brightest = brightest.max(p[0]);
                    }
                }
            }
            let want = action["brightest"].as_u64().context("action glyph")? as u8;
            ensure!(
                brightest.abs_diff(want) <= 16,
                "step {index}: {} glyph {brightest} != {want}",
                action["label"]
            );
        }
        if let Some(tip) = observation.get("assertTooltip") {
            let shown = fx.cx.update(|cx| ui::tooltip::shown(cx).cloned());
            let shown = shown.with_context(|| format!("step {index}: tooltip is missing"))?;
            ensure!(
                shown.label == tip["label"].as_str().context("tooltip label")?,
                "step {index}: wrong tooltip label"
            );
            ensure!(
                shown.shortcut.as_deref() == tip["kbd"].as_str(),
                "step {index}: wrong shortcut"
            );
            let expected = rect(&tip["bbox"])?;
            let bounds = shown.bounds;
            let got = [
                bounds.origin.x,
                bounds.origin.y,
                bounds.size.width,
                bounds.size.height,
            ]
            .map(f32::from);
            ensure!(
                got.iter().zip(expected).all(|(a, b)| (a - b).abs() <= 3.),
                "step {index}: tooltip bounds {got:?} != {expected:?}"
            );
        }
    }
    let writes: Vec<_> = fx
        .sent()?
        .into_iter()
        .skip(before)
        .filter(|r| {
            r["method"].as_str().is_some_and(|m| {
                m != "initialized"
                    && !["/read", "/list", "/get", "/search"]
                        .iter()
                        .any(|suffix| m.ends_with(suffix))
            })
        })
        .collect();
    ensure!(
        serde_json::to_value(writes)? == recording["result"]["requests"],
        "focus or hover sent an app-server write"
    );
    ensure!(
        fx.route() == app::Route::Home,
        "focus and hover must not open a chat"
    );
    println!(
        "f1b: 30 recorded steps matched focus rings, action visibility, tooltips, and requests"
    );
    Ok(())
}
