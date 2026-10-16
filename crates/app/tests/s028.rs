mod common;

use anyhow::{Context as _, Result, ensure};
use bridge::Event;
use common::{Fx, script};
use serde_json::Value;

#[test]
fn proxy() {
    common::proxy_child();
}

fn files(fx: &Fx) -> Vec<(String, usize, usize)> {
    fx.read(|root, cx| {
        root.diff_panel().map_or_else(Vec::new, |panel| {
            panel
                .read(cx)
                .files
                .iter()
                .map(|f| (format!("{}{}", f.dir, f.name), f.added, f.removed))
                .collect()
        })
    })
}

fn search(fx: &Fx) -> Option<(String, usize)> {
    fx.read(|root, cx| {
        root.diff_panel()?
            .read(cx)
            .search()
            .map(|(query, count)| (query.to_owned(), count))
    })
}

fn notify(fx: &mut Fx, method: &str, params: Value) {
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            Event::Notification {
                method: method.into(),
                params,
            },
            cx,
        )
    });
    fx.cx.run_until_parked();
}

fn center(bbox: &Value) -> Result<(f32, f32)> {
    let x = bbox["x"].as_f64().context("x")? + bbox["w"].as_f64().context("w")? / 2.;
    let y = bbox["y"].as_f64().context("y")? + bbox["h"].as_f64().context("h")? / 2.;
    Ok((x as f32, y as f32))
}

fn check_find(fx: &mut Fx, expected: &Value) -> Result<()> {
    let query = expected["input"]["value"]
        .as_str()
        .context("recorded query")?;
    let marks = expected["marks"].as_array().context("recorded marks")?;
    ensure!(
        search(fx) == Some((query.to_owned(), marks.len())),
        "recorded search mismatch: {:?}",
        search(fx)
    );
    ensure!(!fx.composer_focused()?, "find must own keyboard focus");
    let image = fx.shot()?;
    let bar = &expected["bar"]["bbox"];
    let scale = image.width() / 1280;
    let x =
        (bar["x"].as_u64().context("bar x")? + bar["w"].as_u64().context("bar width")? / 2) as u32;
    let y = bar["y"].as_u64().context("bar y")? as u32 + 3;
    ensure!(
        image.get_pixel(x * scale, y * scale).0[..3] == [20, 20, 20],
        "find bar must use the recorded surface"
    );
    for mark in marks {
        let b = &mark["bbox"];
        let x = b["x"].as_u64().context("mark x")? as u32;
        let y = b["y"].as_u64().context("mark y")? as u32;
        let color = match mark["computed"]["backgroundColor"].as_str() {
            Some("rgb(251, 106, 34)") => [251, 106, 34],
            Some("rgb(255, 210, 64)") => [255, 210, 64],
            other => anyhow::bail!("unknown recorded mark color: {other:?}"),
        };
        let found = (x.saturating_sub(3)..x + 25).any(|x| {
            (y.saturating_sub(3)..y + 20)
                .any(|y| image.get_pixel(x * scale, y * scale).0[..3] == color)
        });
        ensure!(found, "missing recorded highlight at {x},{y}");
    }
    image.save(std::env::temp_dir().join("cgx-s028-port.png"))?;
    Ok(())
}

#[test]
fn streamed_diff_and_find_replay() -> Result<()> {
    if !cfg!(target_os = "macos") {
        eprintln!("S028 rendering requires the macOS headless renderer");
        return Ok(());
    }
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s028.json"))?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    let mut before_find = 0;
    for step in recording["steps"].as_array().context("steps")? {
        match step["type"].as_str() {
            Some("click") => {
                let at = if let Some(reference) = step["portReference"].as_object() {
                    fx.at(
                        reference["screen"].as_str().context("reference screen")?,
                        reference["text"].as_str().context("reference text")?,
                    )?
                } else {
                    center(&step["target"]["bbox"])?
                };
                fx.click(at)?;
                if let Some(id) = step["waitThread"].as_str() {
                    fx.wait_thread(id)?;
                }
                if step["target"]["text"] == "View changes" {
                    ensure!(
                        fx.read(|root, _| root.side_panel()) == Some(app::SidePanel::Changes),
                        "View changes must open the panel"
                    );
                }
            }
            Some("bridge") => {
                notify(
                    &mut fx,
                    step["method"].as_str().context("method")?,
                    step["params"].clone(),
                );
                let expected: Vec<_> = step["expect"]["text"]
                    .as_str()
                    .context("file list")?
                    .lines()
                    .collect();
                let added = expected[4].trim_start_matches('+').parse::<usize>()?;
                let removed = expected[5].trim_start_matches('-').parse::<usize>()?;
                ensure!(
                    files(&fx) == [(expected[3].to_owned(), added, removed)],
                    "streamed file list differs from the original"
                );
            }
            Some("key") => {
                before_find = fx.sent()?.len();
                fx.keys(step["portKey"].as_str().context("port key")?)?;
            }
            Some("type") => fx.type_text(step["text"].as_str().context("text")?)?,
            other => anyhow::bail!("unknown step: {other:?}"),
        }
        let expected = &step["expect"];
        if expected.get("marks").is_some() {
            check_find(&mut fx, expected)?;
        }
        if let Some(find) = expected.get("find") {
            check_find(&mut fx, find)?;
        }
    }
    check_find(&mut fx, &recording["result"])?;
    ensure!(
        fx.sent()?.len() == before_find,
        "Last Turn find must not send app-server requests"
    );
    let current_files = files(&fx);
    let delta = recording["steps"]
        .as_array()
        .context("steps")?
        .last()
        .context("last delta")?;
    for (field, value) in [("threadId", "another-thread"), ("turnId", "older-turn")] {
        let mut params = delta["params"].clone();
        params[field] = value.into();
        params["diff"] = "".into();
        notify(&mut fx, "turn/diff/updated", params);
        ensure!(
            files(&fx) == current_files,
            "an unrelated diff must not replace the current turn"
        );
    }
    fx.keys("alt-cmd-b alt-cmd-b")?;
    ensure!(
        files(&fx) == current_files,
        "reopening must retain the streamed aggregate"
    );
    let panel_click = recording["steps"]
        .as_array()
        .context("steps")?
        .iter()
        .find(|step| step["target"]["selector"] == "[role=tabpanel]")
        .context("panel click")?;
    fx.click(center(&panel_click["target"]["bbox"])?)?;
    fx.keys("escape")?;
    ensure!(search(&fx).is_none(), "Escape must close find");
    Ok(())
}
