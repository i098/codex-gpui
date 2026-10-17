//! HC-052: command-search ordering from the original Mac renderer.
#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use gpui::{EntityInputHandler as _, Focusable as _};
use image::GenericImageView as _;
use serde_json::Value;
use std::time::Duration;

const RECORDING: &str = include_str!("../../../refs/interactions/s025_rank.json");

#[test]
fn ambiguous_queries_match_the_original_order() -> Result<()> {
    let recording: Value = serde_json::from_str(RECORDING)?;
    let mut script = common::script(None, |_| {})?;
    // The original server reuses this unchanged response; the bridge consumes each pair once.
    let search = include_str!("../../../fixtures/sessions/s026-thread-search.jsonl")
        .lines()
        .take(2)
        .map(serde_json::from_str::<Value>)
        .collect::<serde_json::Result<Vec<_>>>()?;
    for _ in recording["queries"].as_array().context("queries")? {
        script.extend(search.iter().cloned());
    }
    let mut fx = common::Fx::scripted(script)?;
    for state in recording["queries"].as_array().context("queries")? {
        let query = state["query"].as_str().context("query")?;
        fx.click(common::SEARCH)?;
        fx.type_text(query)?;
        fx.advance(Duration::from_millis(500))?;
        let menu = fx
            .read(|root, _| root.command_menu().cloned())
            .context("command menu open")?;
        let actual: Vec<_> = fx.read(|_, cx| {
            menu.read(cx)
                .describe()
                .into_iter()
                .map(|(heading, rows)| {
                    let rows: Vec<_> = rows
                        .into_iter()
                        .map(|(text, _)| text.split(" | ").next().unwrap().replace(['[', ']'], ""))
                        .collect();
                    (heading.to_owned(), rows)
                })
                .collect()
        });
        let expected = state["groups"].as_array().context("groups")?;
        ensure!(
            actual
                .iter()
                .map(|(heading, _)| heading.as_str())
                .eq(expected
                    .iter()
                    .map(|group| group["heading"].as_str().unwrap())),
            "query {query:?}: groups {actual:#?}"
        );
        for ((heading, rows), group) in actual.iter().zip(expected) {
            let labels: Vec<_> = group["items"]
                .as_array()
                .context("items")?
                .iter()
                .map(|label| label.as_str().unwrap().to_owned())
                .collect();
            if let Some(scores) = group["scores"].as_array() {
                let mut remaining: Vec<_> = labels
                    .into_iter()
                    .zip(scores.iter().map(|s| s.as_i64().unwrap()))
                    .collect();
                let mut previous = i64::MAX;
                for label in rows {
                    let index = remaining
                        .iter()
                        .position(|(expected, _)| expected == label)
                        .with_context(|| {
                            format!("{query:?}, {heading}: unexpected row {label:?}")
                        })?;
                    let (_, score) = remaining.remove(index);
                    ensure!(
                        score <= previous,
                        "{query:?}, {heading}: score order {rows:?}"
                    );
                    previous = score;
                }
                ensure!(
                    remaining.is_empty(),
                    "{query:?}, {heading}: missing rows {remaining:?}"
                );
            } else {
                ensure!(
                    rows == &labels,
                    "{query:?}, {heading}: {rows:?}; expected {labels:?}"
                );
            }
        }
        if matches!(query, "tab" | "work" | "settings") {
            fx.shot()?
                .save(std::env::temp_dir().join(format!("s025-rank-query-{query}.png")))?;
        }
        fx.keys("escape")?;
    }
    Ok(())
}

fn point(value: &Value) -> (f32, f32) {
    (
        value[0].as_f64().unwrap() as f32,
        value[1].as_f64().unwrap() as f32,
    )
}

fn check_selection(
    fx: &mut common::Fx,
    selection: &Value,
    query: &str,
    selected: &str,
) -> Result<()> {
    let menu = fx
        .read(|root, _| root.command_menu().cloned())
        .context("menu open")?;
    let (actual, focused) = fx.cx.update_window(fx.window, |_, window, cx| {
        menu.update(cx, |menu, cx| {
            (
                menu.text_for_range(0..usize::MAX, &mut None, window, cx)
                    .unwrap(),
                menu.focus_handle(cx).is_focused(window),
            )
        })
    })?;
    ensure!(
        actual == query && focused,
        "query {actual:?}, focused {focused}"
    );
    let image = fx.shot()?;
    let scale = image.width() as f32 / 1280.;
    for row in selection["rows"].as_array().context("selection rows")? {
        if query != selection["query"].as_str().unwrap() && row["label"] != selected {
            continue;
        }
        let palette = if row["label"] == selected {
            "selectedRgba"
        } else {
            "plainRgba"
        };
        let expected: [u8; 4] = serde_json::from_value(selection[palette].clone())?;
        let (x, y) = point(&row["sample"]);
        let actual = image.get_pixel((x * scale) as u32, (y * scale) as u32).0;
        ensure!(
            actual
                .iter()
                .zip(expected)
                .all(|(&a, b)| a.abs_diff(b) <= 2),
            "{query:?}, selected {selected:?}, row {}: {actual:?}, expected {expected:?}",
            row["label"]
        );
    }
    Ok(())
}

fn check_destination(
    fx: &mut common::Fx,
    recording: &Value,
    heading: &str,
) -> Result<image::RgbaImage> {
    ensure!(fx.route() == app::Route::Settings);
    ensure!(fx.read(|root, _| root.command_menu().is_none()));
    let image = fx.shot()?;
    let probe = &recording["headingProbe"];
    let region = probe["region"].as_array().context("heading region")?;
    let scale = image.width() as f32 / 1280.;
    let pixel = |i: usize| (region[i].as_f64().unwrap() as f32 * scale) as u32;
    let crop = image::imageops::crop_imm(&image, pixel(0), pixel(1), pixel(2), pixel(3));
    let threshold = probe["foregroundAbove"]
        .as_u64()
        .context("heading foreground")? as u8;
    let (mut left, mut top, mut right, mut bottom) = (crop.width(), crop.height(), 0, 0);
    for (x, y, color) in crop.pixels() {
        if color[0] > threshold {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
    }
    ensure!(
        left <= right && top <= bottom,
        "missing rendered heading {heading}"
    );
    let (width, height) = (right - left + 1, bottom - top + 1);
    let expected = &probe["headings"][heading];
    ensure!(
        width as u64 == expected["width"] && height as u64 == expected["height"],
        "wrong rendered heading for {heading}: {width}x{height}"
    );
    let mut bits = vec![0u8; (width as usize * height as usize).div_ceil(8)];
    for y in top..=bottom {
        for x in left..=right {
            if crop.get_pixel(x, y)[0] > threshold {
                let index = ((y - top) * width + x - left) as usize;
                bits[index / 8] |= 1 << (index % 8);
            }
        }
    }
    let expected_bits = STANDARD.decode(expected["bits"].as_str().context("heading glyphs")?)?;
    ensure!(
        bits == expected_bits,
        "rendered heading does not match original {heading}"
    );
    Ok(image)
}

#[test]
fn pointer_and_keys_keep_the_recorded_result_selected() -> Result<()> {
    let recording: Value = serde_json::from_str(RECORDING)?;
    let selection = &recording["selection"];
    let query = selection["query"].as_str().context("selection query")?;
    let mut fx = common::Fx::new()?;
    fx.click(common::SEARCH)?;
    fx.type_text(query)?;
    check_selection(
        &mut fx,
        selection,
        query,
        selection["initialSelected"].as_str().unwrap(),
    )?;
    for step in selection["steps"].as_array().context("selection steps")? {
        match step["type"].as_str().context("selection step")? {
            "key" => fx.keys(step["key"].as_str().unwrap())?,
            "hover" => fx.hover(point(&step["at"]))?,
            "type" => fx.type_text(step["text"].as_str().unwrap())?,
            kind => anyhow::bail!("unsupported selection step {kind}"),
        }
        check_selection(
            &mut fx,
            selection,
            step["query"].as_str().unwrap(),
            step["selected"].as_str().unwrap(),
        )?;
        if let Some(name) = step["checkpoint"].as_str() {
            fx.shot()?
                .save(std::env::temp_dir().join(format!("s025-rank-{name}.png")))?;
        }
    }
    fx.keys(selection["enter"]["key"].as_str().unwrap())?;
    check_destination(
        &mut fx,
        &recording,
        selection["enter"]["heading"].as_str().unwrap(),
    )?
    .save(std::env::temp_dir().join("s025-rank-enter-worktrees.png"))?;

    for click in [&recording["rowClick"], &recording["aliasClick"]] {
        fx.click(common::RAIL_HOME)?;
        fx.click(common::SEARCH)?;
        fx.type_text(click["query"].as_str().unwrap())?;
        fx.click(point(&click["at"]))?;
        check_destination(&mut fx, &recording, click["heading"].as_str().unwrap())?.save(
            std::env::temp_dir().join(format!(
                "s025-rank-{}.png",
                click["checkpoint"].as_str().unwrap()
            )),
        )?;
    }
    Ok(())
}
