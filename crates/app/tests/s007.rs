//! TH-066: replay the original Markdown message and its disabled task checkbox.
//! The recorded white-pixel mask checks heading sizes, emphasis, list markers, nesting,
//! quote indentation, and task states. Linux and Metal may shift glyph edges by one CSS pixel.
mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use image::RgbaImage;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Ink {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    rows: Vec<Vec<[u32; 2]>>,
}

fn bright(image: &RgbaImage, x: i32, y: i32) -> bool {
    x >= 0
        && y >= 0
        && x < image.width() as i32
        && y < image.height() as i32
        && image.get_pixel(x as u32, y as u32)[0] > 128
}

fn mismatch(image: &RgbaImage, ink: &Ink) -> f64 {
    let mut mask = vec![false; (ink.w * ink.h) as usize];
    for (y, spans) in ink.rows.iter().enumerate() {
        for &[start, end] in spans {
            mask[y * ink.w as usize + start as usize..y * ink.w as usize + end as usize].fill(true);
        }
    }
    let recorded = |x: i32, y: i32| {
        x >= 0
            && y >= 0
            && x < ink.w as i32
            && y < ink.h as i32
            && mask[(y as u32 * ink.w + x as u32) as usize]
    };
    let (mut missing, mut extra, mut expected, mut actual) = (0, 0, 0, 0);
    for y in 0..ink.h as i32 {
        for x in 0..ink.w as i32 {
            let want = recorded(x, y);
            let got = bright(image, ink.x + x, ink.y + y);
            expected += usize::from(want);
            actual += usize::from(got);
            if want
                && !(-2..=2)
                    .any(|dy| (-2..=2).any(|dx| bright(image, ink.x + x + dx, ink.y + y + dy)))
            {
                missing += 1;
            }
            if got && !(-2..=2).any(|dy| (-2..=2).any(|dx| recorded(x + dx, y + dy))) {
                extra += 1;
            }
        }
    }
    (missing as f64 / expected.max(1) as f64).max(extra as f64 / actual.max(1) as f64)
}

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn markdown_blocks_match_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("s007: the Metal renderer requires macOS");
        return Ok(());
    }
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s007.json"))?;
    let ink: Ink = serde_json::from_value(rec["result"]["ink"].clone())?;
    let mut lines = script(None, |_| {})?;
    let source = rec["fixture"]["source"]
        .as_str()
        .context("fixture source")?;
    for line in &mut lines {
        if let Some(turns) = line
            .pointer_mut("/msg/result/thread/turns")
            .and_then(Value::as_array_mut)
        {
            for turn in turns {
                for item in turn["items"].as_array_mut().context("turn items")? {
                    if item["id"] == rec["fixture"]["itemId"] {
                        item["text"] = source.into();
                    }
                }
            }
        }
    }
    let mut fx = Fx::captured("dark", (1280., 1200.), lines)?;
    let steps = rec["steps"].as_array().context("steps")?;
    let bbox = &steps[0]["target"]["bbox"];
    fx.click((
        bbox["x"].as_f64().unwrap() as f32 + 20.,
        bbox["y"].as_f64().unwrap() as f32 + 10.,
    ))?;
    fx.wait_thread(GREETING)?;
    let pointer = &steps[2]["at"];
    let rest = (
        pointer["x"].as_f64().unwrap() as f32,
        pointer["y"].as_f64().unwrap() as f32,
    );
    fx.hover(rest)?;
    let before = fx.shot()?;
    before.save(format!("{}/s007-port.png", env!("CARGO_TARGET_TMPDIR")))?;
    let error = mismatch(&before, &ink);
    ensure!(
        error < 0.12,
        "Markdown layout differs: {:.1}%",
        error * 100.
    );
    for sample in rec["result"]["samples"]
        .as_array()
        .context("pixel samples")?
    {
        let (x, y) = (
            sample["x"].as_u64().unwrap() as u32,
            sample["y"].as_u64().unwrap() as u32,
        );
        let expected: [u8; 4] = serde_json::from_value(sample["rgba"].clone())?;
        ensure!(
            before.get_pixel(x, y).0 == expected,
            "{} color differs",
            sample["name"]
        );
    }
    let sent_before = fx.sent()?.len();
    let bbox = &steps[1]["target"]["bbox"];
    let at = (
        bbox["x"].as_f64().unwrap() as f32 + 7.,
        bbox["y"].as_f64().unwrap() as f32 + 7.,
    );
    fx.click(at)?;
    fx.hover(rest)?;
    let after = fx.shot()?;
    ensure!(
        fx.sent()?.len() == sent_before,
        "disabled task sent an app-server request"
    );
    ensure!(
        changed(
            &before,
            &after,
            [
                ink.x as f32 / 2.,
                ink.y as f32 / 2.,
                ink.w as f32 / 2.,
                ink.h as f32 / 2.
            ]
        ) == 0,
        "disabled task changed the Markdown"
    );
    ensure!(
        fx.thread_id().as_deref() == Some(GREETING),
        "task click changed the thread"
    );
    println!(
        "s007: original Markdown mask matches ({:.2}% error); disabled task stays unchanged",
        error * 100.
    );
    Ok(())
}
