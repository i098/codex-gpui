// Linux uses a no-op text system. Glyph alpha requires the Mac renderer.
#![cfg(target_os = "macos")]

mod common;

use anyhow::Result;
use bridge::Event;
use common::Fx;
use image::RgbaImage;
use serde_json::{Value, json};
use std::time::Duration;

fn event(fx: &mut Fx, method: &str, params: Value) {
    fx.cx.update(|cx| {
        fx.root.update(cx, |root, cx| {
            root.on_event(
                Event::Notification {
                    method: method.into(),
                    params,
                },
                cx,
            );
        });
    });
}

fn start() -> Result<Fx> {
    let mut fx = Fx::with("dark", (1280., 832.), None, |line| {
        if let Some(thread) = line["msg"]["result"].get_mut("thread") {
            thread["turns"] = json!([]);
        }
    })?;
    fx.action(Box::new(app::GoToRecentChat(0)))?;
    fx.wait_thread(common::GREETING)?;
    event(
        &mut fx,
        "turn/started",
        json!({
            "threadId": common::GREETING,
            "turn": {"id": "fade-turn", "status": "inProgress", "items": []}
        }),
    );
    event(
        &mut fx,
        "item/started",
        json!({
            "threadId": common::GREETING, "turnId": "fade-turn",
            "item": {"type": "agentMessage", "id": "fade-message", "text": ""}
        }),
    );
    Ok(fx)
}

fn append(fx: &mut Fx, delta: &str) {
    event(
        fx,
        "item/agentMessage/delta",
        json!({
            "threadId": common::GREETING, "turnId": "fade-turn", "itemId": "fade-message", "delta": delta
        }),
    );
}

fn frame(fx: &mut Fx) -> Result<RgbaImage> {
    fx.cx.run_until_parked();
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.simulate_next_frame(cx);
    })?;
    fx.cx.run_until_parked();
    fx.shot()
}

fn ink(before: &RgbaImage, after: &RgbaImage) -> Vec<(u32, u32)> {
    let scale = before.width() / 1280;
    (80 * scale..350 * scale)
        .flat_map(|y| (400 * scale..1000 * scale).map(move |x| (x, y)))
        .filter(|&(x, y)| after[(x, y)][0] > before[(x, y)][0].saturating_add(80))
        .collect()
}

fn alpha(before: &RgbaImage, after: &RgbaImage, frame: &RgbaImage, mask: &[(u32, u32)]) -> f64 {
    let sum: f64 = mask
        .iter()
        .map(|&p| {
            (f64::from(frame[p][0]) - f64::from(before[p][0]))
                / (f64::from(after[p][0]) - f64::from(before[p][0]))
        })
        .sum();
    sum / mask.len() as f64
}

#[test]
fn streamed_glyphs_follow_original_without_restarting_older_text() -> Result<()> {
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/ms4.json"))?;
    let mut fx = start()?;
    let mut older = Vec::new();
    for step in recording["steps"].as_array().unwrap() {
        let before = frame(&mut fx)?;
        append(&mut fx, step["delta"].as_str().unwrap());
        let mut frames = Vec::new();
        let mut previous = 0;
        for sample in step["samples"].as_array().unwrap() {
            let ms = sample["ms"].as_u64().unwrap();
            fx.cx.advance_clock(Duration::from_millis(ms - previous));
            frames.push(frame(&mut fx)?);
            previous = ms;
        }
        let end = frames.last().unwrap();
        let mask = ink(&before, end);
        assert!(
            mask.len() > 10,
            "the streamed glyphs must paint in the transcript"
        );
        for (sample, frame) in step["samples"].as_array().unwrap().iter().zip(&frames) {
            let actual = alpha(&before, end, frame, &mask);
            let expected = sample["alpha"].as_f64().unwrap();
            assert!(
                (actual - expected).abs() < 0.015,
                "at {} ms: alpha {actual} != {expected}",
                sample["ms"]
            );
            for &p in &older {
                assert_eq!(frame[p], before[p], "a later delta faded an older glyph");
            }
        }
        older.extend(mask);
    }
    fx.cx.update(|cx| cx.set_reduce_motion(true));
    let before = frame(&mut fx)?;
    append(&mut fx, recording["reduced"]["delta"].as_str().unwrap());
    let first = frame(&mut fx)?;
    fx.cx.advance_clock(Duration::from_millis(700));
    let end = frame(&mut fx)?;
    let mask = ink(&before, &end);
    assert!(mask.len() > 10);
    assert!(
        (alpha(&before, &end, &first, &mask) - recording["reduced"]["alpha"].as_f64().unwrap())
            .abs()
            < 0.001
    );
    Ok(())
}

#[test]
fn unicode_suffix_and_decoded_markdown_keep_independent_glyph_ages() -> Result<()> {
    let mut fx = start()?;
    let empty = frame(&mut fx)?;
    append(&mut fx, "Caf");
    fx.cx.advance_clock(Duration::from_millis(700));
    let before = frame(&mut fx)?;
    let older = ink(&empty, &before);
    assert!(older.len() > 10);
    append(&mut fx, "é &amp; `résumé`");
    let first = frame(&mut fx)?;
    fx.cx.advance_clock(Duration::from_millis(100));
    let middle = frame(&mut fx)?;
    fx.cx.advance_clock(Duration::from_millis(600));
    let end = frame(&mut fx)?;
    let new = ink(&first, &end);
    assert!(
        new.len() > 10,
        "the Unicode suffix and decoded text must fade"
    );
    assert!((alpha(&first, &end, &middle, &new) - 0.1952).abs() < 0.015);
    for p in older {
        assert_eq!(
            first[p], before[p],
            "appending inside a word restarted its prefix"
        );
    }
    Ok(())
}
