//! TH-067, TH-068, TH-072: replay the original code block, copy, and URL interactions.
mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use gpui::MouseButton;
use serde_json::Value;
use std::time::Duration;

fn center(bbox: &Value) -> (f32, f32) {
    (
        (bbox["x"].as_f64().unwrap() + bbox["w"].as_f64().unwrap() / 2.) as f32,
        (bbox["y"].as_f64().unwrap() + bbox["h"].as_f64().unwrap() / 2.) as f32,
    )
}

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn fenced_code_copy_and_urls_match_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("s008: the Metal replay requires macOS");
        return Ok(());
    }
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s008.json"))?;
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        if let Some(turns) = line
            .pointer_mut("/msg/result/thread/turns")
            .and_then(Value::as_array_mut)
        {
            for turn in turns {
                for item in turn["items"].as_array_mut().context("turn items")? {
                    if item["id"] == rec["fixture"]["itemId"] {
                        item["text"] = rec["fixture"]["source"].clone();
                    }
                }
            }
        }
    }
    let viewport = &rec["viewport"];
    let mut fx = Fx::captured(
        "dark",
        (
            viewport["w"].as_f64().unwrap() as f32,
            viewport["h"].as_f64().unwrap() as f32,
        ),
        lines,
    )?;
    let steps = rec["steps"].as_array().context("steps")?;
    fx.click(center(&steps[0]["target"]["bbox"]))?;
    fx.wait_thread(GREETING)?;
    fx.hover((1270., 820.))?;
    let image = fx.shot()?;
    image.save(format!("{}/s008-port.png", env!("CARGO_TARGET_TMPDIR")))?;
    for sample in rec["result"]["samples"].as_array().context("samples")? {
        let (x, y) = (
            sample["x"].as_u64().unwrap() as u32,
            sample["y"].as_u64().unwrap() as u32,
        );
        let expected: [u8; 4] = serde_json::from_value(sample["rgba"].clone())?;
        ensure!(
            image.get_pixel(x, y).0 == expected,
            "code surface at {x},{y}: {:?} != {expected:?}",
            image.get_pixel(x, y).0
        );
    }
    for token in rec["result"]["dom"]
        .as_array()
        .context("DOM")?
        .iter()
        .filter(|d| {
            d["selector"]
                .as_str()
                .is_some_and(|s| s.starts_with(".hljs-"))
        })
    {
        let color = token["computed"]["color"].as_str().context("token color")?;
        let rgb: Vec<u8> = color
            .trim_start_matches("rgb(")
            .trim_end_matches(')')
            .split(',')
            .map(|s| s.trim().parse())
            .collect::<std::result::Result<_, _>>()?;
        let b = &token["bbox"];
        let x = b["x"].as_u64().unwrap() as u32 * 2;
        let y = b["y"].as_u64().unwrap() as u32 * 2;
        let w = b["w"].as_u64().unwrap() as u32 * 2;
        let h = b["h"].as_u64().unwrap() as u32 * 2;
        let count = (y.saturating_sub(2)..y + h + 2)
            .flat_map(|y| (x.saturating_sub(2)..x + w + 2).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                image.get_pixel(x, y).0[..3]
                    .iter()
                    .zip(&rgb)
                    .all(|(a, b)| a.abs_diff(*b) <= 4)
            })
            .count();
        ensure!(
            count >= 5,
            "{} has no recorded syntax color at its recorded position",
            token["text"]
        );
    }
    let sent_before = fx.sent()?;
    let at = center(&steps[2]["target"]["bbox"]);
    fx.hover(at)?;
    fx.cx.advance_clock(Duration::from_millis(800));
    fx.cx.run_until_parked();
    fx.hover((1270., 820.))?;
    let before = fx.shot()?;
    fx.click(at)?;
    fx.hover((1270., 820.))?;
    let copied = fx
        .cx
        .update(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    let requests = rec["result"]["requests"].as_array().context("requests")?;
    ensure!(
        copied.as_deref() == requests[0]["args"][0].as_str(),
        "code copy differs from the original clipboard request: {copied:?}"
    );
    let checked = fx.shot()?;
    let b = &rec["result"]["copied"]["bbox"];
    let region = [
        b["x"].as_f64().unwrap() as f32,
        b["y"].as_f64().unwrap() as f32,
        b["w"].as_f64().unwrap() as f32,
        b["h"].as_f64().unwrap() as f32,
    ];
    ensure!(
        changed(&before, &checked, region) > 20,
        "copy did not show its checkmark"
    );
    fx.cx.advance_clock(Duration::from_millis(1800));
    fx.cx.run_until_parked();
    ensure!(
        changed(&checked, &fx.shot()?, region) == 0,
        "copied state ended before the original"
    );
    fx.cx.advance_clock(Duration::from_millis(400));
    fx.cx.run_until_parked();
    ensure!(
        changed(&before, &fx.shot()?, region) == 0,
        "copy did not reset after two seconds"
    );
    for (step, request) in steps[5..].iter().zip(&requests[1..]) {
        let button = if step["button"] == "middle" {
            MouseButton::Middle
        } else {
            MouseButton::Left
        };
        fx.press(center(&step["target"]["bbox"]), button)?;
        ensure!(
            fx.cx.take_opened_url().as_deref() == request["url"].as_str(),
            "URL click did not open the recorded system-browser destination"
        );
        ensure!(
            fx.read(|r, _| r.side_panel()).is_none(),
            "URL click opened Changes"
        );
    }
    ensure!(
        fx.sent()? == sent_before,
        "markdown controls sent an app-server request"
    );
    println!(
        "s008: code colors, clipboard, two-second feedback, and left/middle URL clicks match the original"
    );
    Ok(())
}
