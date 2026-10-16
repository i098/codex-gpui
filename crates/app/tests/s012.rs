//! S012: the transcript link context menu (TH-130) and closing the find bar (TH-051), replayed
//! from refs/interactions/s012.json on the port.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, script};
use serde_json::Value;

#[test]
fn proxy() {
    common::proxy_child();
}

fn center(bbox: &Value) -> (f32, f32) {
    let f = |k: &str| bbox[k].as_f64().unwrap() as f32;
    (f("x") + f("w") / 2., f("y") + f("h") / 2.)
}

fn at(step: &Value) -> (f32, f32) {
    let at = &step["at"];
    (
        at["x"].as_f64().unwrap() as f32,
        at["y"].as_f64().unwrap() as f32,
    )
}

fn menu(fx: &Fx) -> Vec<(String, String, bool)> {
    fx.read(|root, cx| {
        root.thread().map_or_else(Vec::new, |thread| {
            thread
                .read(cx)
                .context_menu_items()
                .map(|(id, label, enabled)| (id.to_owned(), label.to_owned(), enabled))
                .collect()
        })
    })
}

fn menu_at(fx: &Fx) -> Option<(f32, f32)> {
    fx.read(|root, cx| {
        root.thread()?
            .read(cx)
            .context_menu_at()
            .map(|p| (f32::from(p.x), f32::from(p.y)))
    })
}

fn find(fx: &Fx) -> Option<(String, usize, usize)> {
    fx.read(|root, cx| {
        root.thread()?
            .read(cx)
            .find_state()
            .map(|(query, active, count)| (query.to_owned(), active, count))
    })
}

fn focused(fx: &mut Fx) -> Result<bool> {
    fx.cx
        .update_window(fx.window, |_, window, cx| window.focused(cx).is_some())
}

/// Highlight px (active orange, other yellow) in the transcript.
fn highlights(fx: &mut Fx) -> Result<(usize, usize)> {
    let image = fx.shot()?;
    let scale = image.width() / 1280;
    Ok(image
        .enumerate_pixels()
        .filter(|(x, y, _)| *x > 340 * scale && *y > 90 * scale && *y < 710 * scale)
        .fold((0, 0), |(orange, yellow), (_, _, pixel)| {
            (
                orange + usize::from(pixel.0[..3] == [251, 106, 34]),
                yellow + usize::from(pixel.0[..3] == [255, 210, 64]),
            )
        }))
}

/// Center of the menu row `index`: the popover pads 4 px, rows are 28.5625 px, the separator
/// 9 px (`thread/menu.rs`).
fn row_center((x, y): (f32, f32), index: usize) -> (f32, f32) {
    let above: f32 = (0..index).map(|i| if i == 4 { 9. } else { 28.5625 }).sum();
    (x + 4. + 110., y + 4. + above + 28.5625 / 2.)
}

#[test]
fn link_context_menu_and_find_close_match_original() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("s012: the Metal replay requires macOS");
        return Ok(());
    }
    let rec: Value = serde_json::from_str(include_str!("../../../refs/interactions/s012.json"))?;
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
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let steps = rec["steps"].as_array().context("steps")?;
    let menus = rec["result"]["contextMenus"]
        .as_array()
        .context("contextMenus")?;
    let finds = rec["result"]["find"].as_array().context("find")?;
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    for (index, step) in steps.iter().enumerate() {
        match step["type"].as_str() {
            Some("click") => {
                fx.click(center(&step["target"]["bbox"]))?;
                if let Some(id) = step["waitThread"].as_str() {
                    fx.wait_thread(id)?;
                }
            }
            // The port reads the fixture item through thread/read.
            Some("bridge") => {}
            Some("context-menu") => {
                let expected = menus
                    .iter()
                    .find(|m| m["afterStep"] == index)
                    .context("recorded context menu")?;
                let before = fx.sent()?.len();
                fx.right_click(at(step))?;
                if expected["shown"] != true {
                    ensure!(
                        menu(&fx).is_empty(),
                        "right-click on plain text opened a menu: {:?}",
                        menu(&fx)
                    );
                    continue;
                }
                let items: Vec<(String, String, bool)> = expected["menu"]["items"]
                    .as_array()
                    .context("items")?
                    .iter()
                    .map(|item| {
                        (
                            item["id"].as_str().unwrap().to_owned(),
                            item["label"].as_str().unwrap().to_owned(),
                            item["enabled"] == true && item["type"] != "separator",
                        )
                    })
                    .collect();
                ensure!(menu(&fx) == items, "link menu differs: {:?}", menu(&fx));
                let pos = &expected["menu"]["pos"];
                let anchored = menu_at(&fx).context("menu position")?;
                ensure!(
                    (anchored.0 - pos["x"].as_f64().unwrap() as f32).abs() <= 1.
                        && (anchored.1 - pos["y"].as_f64().unwrap() as f32).abs() <= 1.,
                    "menu anchored at {anchored:?}, the original at {pos}"
                );
                if index == 2 {
                    fx.shot()?.save(out.join("s012-link-menu.png"))?;
                }
                match step["pick"].as_str() {
                    None => {
                        fx.keys("down")?;
                        fx.keys("escape")?;
                        ensure!(menu(&fx).is_empty(), "Escape must close the link menu");
                    }
                    Some("copy-link") => {
                        let row = items.iter().position(|item| item.0 == "copy-link").unwrap();
                        fx.click(row_center(anchored, row))?;
                        ensure!(menu(&fx).is_empty(), "Copy link must close the menu");
                        let text = fx
                            .cx
                            .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
                        let written = expected["requests"]
                            .as_array()
                            .and_then(|r| r.iter().find(|r| r["svc"] == "clipboard.writeText"))
                            .and_then(|r| r["args"].as_str())
                            .and_then(|args| serde_json::from_str::<Vec<String>>(args).ok())
                            .and_then(|args| args.into_iter().next())
                            .context("recorded clipboard write")?;
                        ensure!(
                            text.as_deref() == Some(written.as_str()),
                            "Copy link wrote {text:?}, the original {written:?}"
                        );
                    }
                    Some("open-in-external-browser") => {
                        // Keyboard: Down lands on `New tab`, then `New window`, `Default browser`.
                        for _ in 0..3 {
                            fx.keys("down")?;
                        }
                        fx.keys("enter")?;
                        ensure!(menu(&fx).is_empty(), "Enter must close the menu");
                        let url = expected["requests"]
                            .as_array()
                            .and_then(|r| r.iter().find(|r| r["type"] == "open-in-browser"))
                            .and_then(|r| r["url"].as_str())
                            .context("recorded open-in-browser")?;
                        ensure!(
                            fx.cx.take_opened_url().as_deref() == Some(url),
                            "Default browser did not open {url}"
                        );
                    }
                    other => anyhow::bail!("unknown pick {other:?}"),
                }
                ensure!(
                    fx.sent()?.len() == before,
                    "the link menu sent app-server requests"
                );
            }
            Some("key") => {
                let key = step["portKey"].as_str().or(step["key"].as_str()).unwrap();
                fx.keys(&key.to_lowercase())?;
            }
            Some("type") => fx.type_text(step["text"].as_str().context("text")?)?,
            other => anyhow::bail!("unsupported recorded step: {other:?}"),
        }
        if let Some(expected) = finds.iter().find(|f| f["beforeStep"] == index) {
            // Checked right before the closing step ran: the bar, its query and highlights.
            let (orange, yellow) = highlights(&mut fx)?;
            let after = &expected["after"];
            ensure!(
                find(&fx).is_none(),
                "{}: the find bar stayed open",
                expected["close"]
            );
            ensure!(
                after["marks"].as_array().is_some_and(Vec::is_empty) && (orange, yellow) == (0, 0),
                "{}: highlights stayed after closing: {orange} active, {yellow} other",
                expected["close"]
            );
            // The original's activeElement is the body: nothing of the app, keys still work
            // (the next Cmd+F reopens the bar). The port's body is the root's focus handle.
            ensure!(
                after["activeElement"]["tag"] == "body"
                    && focused(&mut fx)?
                    && !fx.composer_focused()?,
                "{}: focus did not go to the root",
                expected["close"]
            );
        } else if let Some(expected) = finds.iter().find(|f| f["beforeStep"] == index + 1) {
            let before = &expected["before"];
            let marks = before["marks"].as_array().context("marks")?;
            ensure!(
                find(&fx)
                    == Some((
                        before["input"]["value"].as_str().unwrap().to_owned(),
                        0,
                        marks.len()
                    )),
                "find state before {}: {:?}",
                expected["close"],
                find(&fx)
            );
            let (orange, yellow) = highlights(&mut fx)?;
            ensure!(orange > 0 && yellow > 0, "highlights missing before close");
            ensure!(focused(&mut fx)?, "the find input is not focused");
        }
    }
    Ok(())
}
