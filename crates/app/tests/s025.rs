//! HC-053: native command-query editing, recorded on the background Mac renderer.
#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, bail, ensure};
use gpui::{ClipboardItem, EntityInputHandler as _, Focusable as _};
use serde_json::Value;

const RECORDING: &str = include_str!("../../../refs/interactions/s025.json");

fn point(value: &Value) -> (f32, f32) {
    (
        value["x"].as_f64().unwrap() as f32,
        value["y"].as_f64().unwrap() as f32,
    )
}

#[test]
fn command_query_edits_match_the_original() -> Result<()> {
    let recording: Value = serde_json::from_str(RECORDING)?;
    let mut fx = common::Fx::new()?;
    for (index, step) in recording["steps"]
        .as_array()
        .context("steps")?
        .iter()
        .enumerate()
    {
        match step["type"].as_str().context("step type")? {
            "click" => {
                let at = if step["at"].is_object() {
                    point(&step["at"])
                } else {
                    let b = &step["target"]["bbox"];
                    let (x, y) = point(b);
                    (
                        x + b["w"].as_f64().unwrap() as f32 / 2.,
                        y + b["h"].as_f64().unwrap() as f32 / 2.,
                    )
                };
                fx.click(at)?;
            }
            "drag" => fx.drag(
                point(&step["from"]),
                point(&step["to"]),
                step["moves"].as_u64().unwrap() as u32,
            )?,
            "type" => fx.type_text(step["text"].as_str().context("text")?)?,
            "key" => {
                if let Some(text) = step["clipboard"]["text"].as_str() {
                    fx.cx
                        .update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text.into())));
                }
                fx.keys(step["portKey"].as_str().context("portKey")?)?;
            }
            kind => bail!("unsupported S025 step {kind}"),
        }
        let menu = fx
            .read(|root, _| root.command_menu().cloned())
            .context("command menu open")?;
        let (value, selection, focused) = fx.cx.update_window(fx.window, |_, window, cx| {
            menu.update(cx, |menu, cx| {
                (
                    menu.text_for_range(0..usize::MAX, &mut None, window, cx)
                        .unwrap(),
                    menu.selected_text_range(false, window, cx).unwrap(),
                    menu.focus_handle(cx).is_focused(window),
                )
            })
        })?;
        let expected = &step["expected"];
        ensure!(
            value == expected["value"].as_str().unwrap(),
            "step {index}: query {value:?}, expected {expected}"
        );
        ensure!(
            selection.range.start == expected["start"].as_u64().unwrap() as usize
                && selection.range.end == expected["end"].as_u64().unwrap() as usize
                && selection.reversed == (expected["direction"] == "backward"),
            "step {index}: selection {selection:?}, expected {expected}"
        );
        ensure!(
            focused == expected["focused"].as_bool().unwrap(),
            "step {index}: input focus"
        );
        if let Some(expected) = step["clipboardExpected"].as_str() {
            let copied = fx
                .cx
                .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
            ensure!(
                copied.as_deref() == Some(expected),
                "step {index}: copied text"
            );
        }
        if let Some(name) = step["checkpoint"].as_str() {
            let image = fx.shot()?;
            image.save(std::env::temp_dir().join(format!("s025-{name}.png")))?;
            if step["pixel"].is_object() {
                let (x, y) = point(&step["pixel"]["at"]);
                let scale = image.width() as f32 / 1280.;
                let pixel = image.get_pixel((x * scale) as u32, (y * scale) as u32).0;
                let expected: [u8; 4] = serde_json::from_value(step["pixel"]["rgba"].clone())?;
                ensure!(
                    pixel.iter().zip(expected).all(|(&a, b)| a.abs_diff(b) <= 2),
                    "native selection pixel: {pixel:?}, expected {expected:?}"
                );
            }
        }
    }
    fx.keys("escape")?;
    ensure!(fx.composer_focused()?, "dismissal restores composer focus");
    ensure!(fx.read(|root, _| root.command_menu().is_none()));

    // Caret movement on an empty query must not reset the selected result.
    fx.keys("cmd-k down left right enter")?;
    fx.wait_thread(common::SUBTRACT)?;
    ensure!(
        fx.read(|root, _| root.command_menu().is_none()),
        "activation closes the menu"
    );
    Ok(())
}
