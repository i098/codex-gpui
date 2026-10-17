//! CP-039/CP-040: the Add trigger shares the mention list without inserting `@`.
#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use common::Fx;
use gpui::EntityInputHandler as _;
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path, time::Duration};

fn insert(fx: &mut Fx, text: &str) -> Result<()> {
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        composer.update(cx, |c, cx| c.replace_text_in_range(None, text, window, cx));
    })?;
    fx.cx.run_until_parked();
    Ok(())
}

fn center(b: &Value) -> (f32, f32) {
    let n = |k| b[k].as_f64().unwrap() as f32;
    (n("x") + n("w") / 2., n("y") + n("h") / 2.)
}

#[test]
fn add_menu_replay() -> Result<()> {
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s018.json"))?;
    let proof = Path::new(env!("CARGO_TARGET_TMPDIR")).join("s018");
    fs::create_dir_all(&proof)?;
    let mut fx = Fx::new()?;
    let add = center(&recording["trigger"]);

    for (index, step) in recording["steps"]
        .as_array()
        .context("steps")?
        .iter()
        .enumerate()
    {
        match step["type"].as_str() {
            Some("click") => fx.click(add)?,
            Some("key") => fx.keys(step["portKey"].as_str().context("portKey")?)?,
            Some("type") => insert(&mut fx, step["text"].as_str().context("text")?)?,
            other => anyhow::bail!("unknown original step {other:?}"),
        }
        let expected = &step["expect"];
        ensure!(
            fx.composer_text()? == expected["text"],
            "step {index}: draft differs"
        );
        ensure!(
            fx.composer_focused()? == expected["focused"],
            "step {index}: focus differs"
        );
        let state = fx.read(|root, cx| root.composer(cx).read(cx).mention_state());
        ensure!(
            state.is_some() == expected["expanded"],
            "step {index}: Add expansion differs"
        );
        if expected["menu"].is_null() {
            ensure!(state.is_none(), "step {index}: list did not close");
            continue;
        }
        let state = state.context("Add list")?;
        let sections = expected["menu"]["sections"]
            .as_array()
            .context("sections")?;
        ensure!(
            state.len() == sections.len(),
            "step {index}: section count differs"
        );
        for ((heading, rows), section) in state.iter().zip(sections) {
            ensure!(
                *heading == section["heading"].as_str(),
                "step {index}: heading differs"
            );
            let items = section["items"].as_array().context("items")?;
            ensure!(rows.len() == items.len(), "step {index}: row count differs");
            for ((text, selected), item) in rows.iter().zip(items) {
                ensure!(
                    *text == item["text"] && *selected == item["current"],
                    "step {index}: row {text:?} differs"
                );
            }
        }
        let image = fx.shot()?;
        image.save(proof.join(format!("menu-{index}.png")))?;
        let b = &expected["menu"]["bbox"];
        let scale = image.width() as f64 / 1280.;
        let x = (b["x"].as_f64().unwrap() + b["w"].as_f64().unwrap() / 2.) * scale;
        let y = (b["y"].as_f64().unwrap() + b["h"].as_f64().unwrap() + 1.) * scale;
        ensure!(
            image.get_pixel(x as u32, y as u32).0[..3] == [45, 45, 45],
            "step {index}: menu surface differs"
        );
        let probe = &recording["result"]["dom"][0];
        if probe["afterStep"].as_u64() == Some(index as u64) {
            let b = &probe["bbox"];
            let [x, y, w, h] = ["x", "y", "w", "h"].map(|key| b[key].as_f64().unwrap() * scale);
            let [left, top, right, bottom] = [x, y, x + w, y + h].map(|v| v as u32);
            let mut counts = HashMap::new();
            for y in top..bottom {
                for x in left..right {
                    let p = image.get_pixel(x, y).0;
                    *counts.entry([p[0], p[1], p[2]]).or_insert(0usize) += 1;
                }
            }
            // The recorded 8% white selection over the 45-grey surface.
            ensure!(
                counts
                    .into_iter()
                    .max_by_key(|(_, count)| *count)
                    .map(|(color, _)| color)
                    == Some([62, 62, 62]),
                "step {index}: recorded selection fill differs"
            );
        }
    }
    Ok(())
}

#[test]
fn add_tooltip_replay() -> Result<()> {
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s018.json"))?;
    let proof = Path::new(env!("CARGO_TARGET_TMPDIR")).join("s018");
    fs::create_dir_all(&proof)?;
    let mut fx = Fx::new()?;
    let add = center(&recording["trigger"]);
    fx.shot()?;
    fx.hover(add)?;
    fx.advance(Duration::from_millis(199))?;
    ensure!(
        fx.cx.update(|cx| ui::tooltip::shown(cx).is_none()),
        "cold tooltip opened early"
    );
    fx.advance(Duration::from_millis(1))?;
    let image = fx.shot()?;
    image.save(proof.join("tooltip.png"))?;
    let tip = fx
        .cx
        .update(|cx| ui::tooltip::shown(cx).cloned())
        .context("Add tooltip")?;
    ensure!(
        tip.label == "Add files and more" && tip.shortcut.as_deref() == Some("@"),
        "tooltip content differs"
    );
    let expected_tip =
        ["x", "y", "w", "h"].map(|key| recording["tooltip"]["bbox"][key].as_f64().unwrap() as f32);
    let actual_tip = [
        f32::from(tip.bounds.origin.x),
        f32::from(tip.bounds.origin.y),
        f32::from(tip.bounds.size.width),
        f32::from(tip.bounds.size.height),
    ];
    ensure!(
        actual_tip
            .iter()
            .zip(expected_tip)
            .all(|(got, want)| (got - want).abs() <= 1.)
            && (f32::from(tip.bounds.bottom()) - expected_tip[1] - expected_tip[3]).abs() <= 1.,
        "tooltip anchor differs: port {actual_tip:?}, original {expected_tip:?}"
    );
    fx.click(add)?;
    ensure!(
        fx.cx.update(|cx| ui::tooltip::shown(cx).is_none()),
        "a click must close the tooltip"
    );
    Ok(())
}

#[test]
fn add_query_keeps_the_draft_and_closes_when_replaced_before_its_anchor() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Do anything")?)?;
    insert(&mut fx, "é🐚 ")?;
    fx.click((462.5, 790.))?;
    insert(&mut fx, "files")?;
    ensure!(
        fx.composer_text()? == "é🐚 files",
        "the Add trigger changed the draft"
    );
    let state = fx
        .read(|r, cx| r.composer(cx).read(cx).mention_state())
        .context("query list")?;
    ensure!(
        state == vec![(None, vec![("Files and folders".into(), true)])],
        "query includes text before the Add anchor"
    );
    fx.keys("cmd-a")?;
    insert(&mut fx, "replacement")?;
    ensure!(
        fx.composer_text()? == "replacement",
        "replacement changed the draft"
    );
    ensure!(
        fx.read(|r, cx| r.composer(cx).read(cx).mention_state())
            .is_none(),
        "replacement before the query anchor must close the list"
    );
    fx.keys("cmd-a")?;
    insert(&mut fx, "")?;
    fx.click((462.5, 790.))?;
    fx.keys("escape")?;
    insert(&mut fx, "@")?;
    ensure!(
        fx.composer_text()? == "@"
            && fx
                .read(|r, cx| r.composer(cx).read(cx).mention_state())
                .is_some(),
        "dismissing an empty Add query must not suppress a typed @"
    );
    Ok(())
}
