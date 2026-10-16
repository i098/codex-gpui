mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, script};
use serde_json::Value;

#[test]
fn proxy() {
    common::proxy_child();
}

fn find(fx: &Fx) -> Option<(String, usize, usize)> {
    fx.read(|root, cx| {
        root.thread()?
            .read(cx)
            .find_state()
            .map(|(query, active, count)| (query.to_owned(), active, count))
    })
}

#[test]
fn recorded_thread_find_query_and_navigation() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s013.json"))?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    let checkpoints = recording["result"]["checkpoints"]
        .as_array()
        .context("checkpoints")?;
    let mut before_find = 0;
    for (index, step) in recording["steps"]
        .as_array()
        .context("steps")?
        .iter()
        .enumerate()
    {
        match step["type"].as_str() {
            Some("click") => {
                let bbox = &step["target"]["bbox"];
                let center = |p, size| -> Result<f32> {
                    Ok((bbox[p].as_f64().context("position")?
                        + bbox[size].as_f64().context("size")? / 2.) as f32)
                };
                fx.click((center("x", "w")?, center("y", "h")?))?;
                if let Some(id) = step["waitThread"].as_str() {
                    fx.wait_thread(id)?;
                    before_find = fx.sent()?.len();
                }
            }
            Some("key") => {
                let key = step["portKey"]
                    .as_str()
                    .or(step["key"].as_str())
                    .context("key")?;
                let key = key.replace("Shift+", "shift-").to_lowercase();
                fx.keys(&key)?;
            }
            Some("type") => {
                let text = step["text"].as_str().context("text")?.to_owned();
                fx.cx
                    .update(|cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text)));
                fx.keys("cmd-v")?;
            }
            other => anyhow::bail!("unsupported recorded step: {other:?}"),
        }
        for expected in checkpoints
            .iter()
            .filter(|entry| entry["afterStep"] == index)
        {
            let image = fx.shot()?;
            let marks = expected["marks"].as_array().context("marks")?;
            let active = marks
                .iter()
                .position(|mark| mark["active"] == true)
                .unwrap_or(0);
            ensure!(
                find(&fx)
                    == Some((
                        expected["query"].as_str().context("query")?.to_owned(),
                        active,
                        marks.len()
                    )),
                "step {index}: find state differs from the original: {:?}",
                find(&fx)
            );
            ensure!(
                fx.composer_text()?.is_empty(),
                "the query entered the composer"
            );
            ensure!(!fx.composer_focused()?, "the composer kept focus");
            let scale = image.width() / 1280;
            ensure!(
                image.get_pixel(1100 * scale, 11 * scale).0[..3] == [20, 20, 20],
                "find bar surface differs"
            );
            let highlighted = image
                .enumerate_pixels()
                .filter(|(x, y, _)| *x > 340 * scale && *y > 90 * scale && *y < 710 * scale)
                .fold((0, 0), |(orange, yellow), (_, _, pixel)| {
                    (
                        orange + usize::from(pixel.0[..3] == [251, 106, 34]),
                        yellow + usize::from(pixel.0[..3] == [255, 210, 64]),
                    )
                });
            if marks.is_empty() {
                ensure!(highlighted == (0, 0), "cleared query left highlights");
            } else {
                ensure!(
                    highlighted.0 > 0 && highlighted.1 > 0,
                    "active or inactive highlight is missing"
                );
            }
            if index == 7 {
                image.save(std::env::temp_dir().join("cgx-s013-port.png"))?;
            }
        }
    }
    ensure!(
        fx.sent()?.len() == before_find,
        "search sent app-server requests"
    );
    fx.type_text("subtract")?;
    fx.keys("cmd-g")?;
    ensure!(
        find(&fx) == Some(("subtract".into(), 1, 5)),
        "Cmd+G did not select the next match"
    );
    fx.keys("cmd-shift-g")?;
    ensure!(
        find(&fx) == Some(("subtract".into(), 0, 5)),
        "Cmd+Shift+G did not select the previous match"
    );
    fx.menu("Edit", "Find")?;
    fx.type_text("math")?;
    fx.shot()?;
    ensure!(
        find(&fx).is_some_and(|(query, _, count)| query == "math" && count == 3),
        "Find did not select the query for replacement"
    );
    fx.keys("cmd-f")?;
    fx.type_text("subtract")?;
    fx.resize(1280., 400.)?;
    fx.keys("shift-enter")?;
    fx.shot()?;
    fx.cx.advance_clock(std::time::Duration::from_millis(350));
    fx.cx.run_until_parked();
    ensure!(
        find(&fx) == Some(("subtract".into(), 4, 5)),
        "previous-match wrap selected the wrong match"
    );
    let image = fx.shot()?;
    image.save(std::env::temp_dir().join("cgx-s013-scroll.png"))?;
    let scale = image.width() / 1280;
    ensure!(
        image.enumerate_pixels().any(|(x, y, pixel)| {
            x > 340 * scale && y > 90 * scale && y < 254 * scale && pixel.0[..3] == [251, 106, 34]
        }),
        "previous-match wrap did not bring the last match into view"
    );
    Ok(())
}
