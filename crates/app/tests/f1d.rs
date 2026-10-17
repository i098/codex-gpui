mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, ROOT, script};
use gpui::{Focusable as _, KeyUpEvent, Keystroke, PlatformInput};
use serde_json::Value;
use std::{fs, ops::Range, path::PathBuf, time::Duration};

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

fn step(fx: &mut Fx, input: &Value) -> Result<()> {
    match input["type"].as_str() {
        Some("key") => {
            let prefix = if input["shift"] == true { "shift-" } else { "" };
            let key = input["key"].as_str().context("key")?.to_lowercase();
            let key = Keystroke::parse(&format!("{prefix}{key}"))?;
            fx.cx.update_window(fx.window, |_, window, cx| {
                window.dispatch_keystroke(key.clone(), cx);
                window.dispatch_event(PlatformInput::KeyUp(KeyUpEvent { keystroke: key }), cx);
            })?;
        }
        kind => {
            let [x, y, w, h] = rect(&input["target"]["bbox"])?;
            let at = (x + w / 2., y + h / 2.);
            match kind {
                Some("click") => fx.click(at)?,
                Some("hover") => fx.hover(at)?,
                _ => anyhow::bail!("unsupported input {input}"),
            }
        }
    }
    fx.cx.run_until_parked();
    fx.advance(Duration::from_millis(
        input["waitMs"].as_u64().unwrap_or(160),
    ))
}

#[test]
fn skip_link_moves_focus_to_content() -> Result<()> {
    replay(0..8)
}

#[test]
fn full_label_tooltip_requires_overflow() -> Result<()> {
    replay(8..15)
}

fn replay(steps: Range<usize>) -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("f1d: requires the macOS headless renderer");
        return Ok(());
    }
    let recording: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/f1d.json"))?)?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    let shots = std::env::var_os("F1D_SHOTS").map(PathBuf::from);
    if let Some(dir) = &shots {
        fs::create_dir_all(dir)?;
    }
    fx.shot()?;
    let before = fx.sent()?.len();
    let mut home_focus = None;
    let mut skip_focus = None;
    let mut pixel_mismatches = Vec::new();
    for index in steps {
        let input = &recording["steps"][index];
        step(&mut fx, input)?;
        let image = fx.shot()?;
        if let Some(dir) = &shots {
            image.save(dir.join(format!("step-{index}.png")))?;
        }
        let observation = &recording["result"]["dom"][index];
        let composer = fx.composer();
        let expected_skip = observation["skipVisible"]
            .as_bool()
            .context("skip visibility")?;
        let (skip_focused, main_focused) = fx.cx.update_window(fx.window, |_, window, cx| {
            let current = window.focused(cx);
            if index == 0 {
                home_focus = current.clone();
            }
            if expected_skip && skip_focus.is_none() {
                skip_focus = current.clone();
            }
            let composer = composer.focus_handle(cx);
            let main_focused = current.as_ref().is_some_and(|focus| {
                focus != &composer
                    && focus.contains(&composer, window)
                    && home_focus
                        .as_ref()
                        .is_some_and(|home| !focus.contains(home, window))
            });
            (current.is_some() && current == skip_focus, main_focused)
        })?;
        ensure!(
            skip_focused
                == observation["skipVisible"]
                    .as_bool()
                    .context("skip visibility")?,
            "step {index}: skip link focus differs from the original"
        );
        ensure!(
            main_focused
                == observation["mainFocused"]
                    .as_bool()
                    .context("content focus")?,
            "step {index}: content focus differs from the original"
        );
        let scale = image.width() as f32 / 1280.;
        for sample in observation["pixels"].as_array().context("pixels")? {
            let x = sample["x"].as_f64().context("pixel x")? as f32;
            let y = sample["y"].as_f64().context("pixel y")? as f32;
            let got = image.get_pixel((x * scale) as u32, (y * scale) as u32).0;
            let want: [u8; 3] = serde_json::from_value(sample["rgb"].clone())?;
            if got[..3] != want {
                pixel_mismatches.push(format!("step {index}: ({x}, {y}) {got:?} != {want:?}"));
            }
        }
        if let Some(expected) = observation.get("tooltip") {
            let shown = fx.cx.update(|cx| ui::tooltip::shown(cx).cloned());
            if expected.is_null() {
                ensure!(
                    shown.is_none(),
                    "step {index}: unexpected tooltip {shown:?}"
                );
            } else {
                let shown = shown.with_context(|| format!("step {index}: missing tooltip"))?;
                ensure!(shown.label == expected["label"].as_str().context("tooltip label")?);
                ensure!(
                    shown.shortcut.is_none(),
                    "step {index}: unexpected shortcut"
                );
                let got = [
                    shown.bounds.origin.x,
                    shown.bounds.origin.y,
                    shown.bounds.size.width,
                    shown.bounds.size.height,
                ]
                .map(f32::from);
                let want = rect(&expected["bbox"])?;
                ensure!(
                    got.iter().zip(want).all(|(a, b)| (a - b).abs() <= 1.),
                    "step {index}: tooltip bounds {got:?} != {want:?}"
                );
            }
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
                        .any(|s| m.ends_with(s))
            })
        })
        .collect();
    ensure!(serde_json::to_value(writes)? == recording["result"]["requests"]);
    ensure!(fx.route() == app::Route::Home);
    ensure!(
        pixel_mismatches.is_empty(),
        "recorded pixel mismatches:\n{}",
        pixel_mismatches.join("\n")
    );
    println!("f1d: recorded interaction and request checks passed");
    Ok(())
}
