//! S020: replay the original's mention keys, file and skill chips, atomic deletion and
//! submitted markdown. The shared Fx drives the real headless Root and captures its bridge.
//! route1 refuses sends, so its final editor stays filled; the port's replay accepts them.
#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, script};
use gpui::EntityInputHandler as _;
use serde_json::{Value, json};

#[test]
fn proxy() {
    common::proxy_child();
}

fn search_script(session: usize, query: &str, files: &[(&str, &str)]) -> Vec<Value> {
    let session = format!("fuzzy-{session}");
    let files: Vec<_> = files
        .iter()
        .map(|(path, name)| {
            json!({
                "root": common::CWD, "path": path, "file_name": name,
                "match_type": "file", "score": 84, "indices": []
            })
        })
        .collect();
    let mut lines = common::exchange("fuzzyFileSearch/sessionStart", json!({})).to_vec();
    lines.extend(common::exchange("fuzzyFileSearch/sessionUpdate", json!({})));
    lines.push(json!({"dir": "recv", "msg": {
        "method": "fuzzyFileSearch/sessionUpdated",
        "params": {"sessionId": session, "query": query, "files": files}
    }}));
    lines.push(json!({"dir": "recv", "msg": {
        "method": "fuzzyFileSearch/sessionCompleted", "params": {"sessionId": session}
    }}));
    lines.extend(common::exchange("fuzzyFileSearch/sessionStop", json!({})));
    lines
}

fn insert(fx: &mut Fx, text: &str) -> Result<()> {
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        composer.update(cx, |c, cx| c.replace_text_in_range(None, text, window, cx));
    })?;
    fx.cx.run_until_parked();
    Ok(())
}

fn fuzzy_requests(sent: &[Value]) -> Vec<Value> {
    sent.iter()
        .filter(|r| {
            r["method"]
                .as_str()
                .is_some_and(|m| m.starts_with("fuzzyFileSearch/"))
        })
        .map(|r| {
            json!({"method": r["method"], "params": {
                "roots": r["params"]["roots"], "query": r["params"]["query"]
            }})
        })
        .collect()
}

#[test]
fn mention_keys_chips_and_prompt_replay() -> Result<()> {
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s020.json"))?;
    for (run, rec) in [&recording, &recording["second"]].into_iter().enumerate() {
        let mut lines = script(None, |_| {})?;
        lines.extend(search_script(
            run + 1,
            "mat",
            &[("src/math.py", "math.py"), ("src/matrix.rs", "matrix.rs")],
        ));
        if run == 1 {
            lines.extend(search_script(3, "setup", &[]));
        }
        lines.extend(common::exchange("thread/resume", json!({})));
        lines.extend(common::exchange(
            "turn/start",
            json!({"turn": {"id": "s020-turn", "status": "inProgress", "items": []}}),
        ));
        let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
        let before = fx.sent()?.len();
        for (i, step) in rec["steps"].as_array().context("steps")?.iter().enumerate() {
            match (step["type"].as_str(), i) {
                (Some("click"), 0) => {
                    fx.keys("cmd-1")?;
                    fx.wait_thread(common::GREETING)?;
                }
                (Some("click"), _) => fx.click(fx.at("thread", "Work with Codex")?)?,
                (Some("type"), _) => {
                    let text = step["text"].as_str().context("text")?;
                    insert(&mut fx, text)?;
                    if matches!(text, "mat" | "setup") {
                        fx.wait("mention search answer", |fx| {
                            fx.read(|r, cx| r.composer(cx).read(cx).mention_state())
                                .is_some_and(|s| {
                                    s[0].1.iter().any(|(row, _)| {
                                        row.starts_with(if text == "mat" {
                                            "math.py"
                                        } else {
                                            "Setup Matt"
                                        })
                                    })
                                })
                        })?;
                    }
                }
                (Some("key"), _) => fx.keys(step["portKey"].as_str().context("portKey")?)?,
                other => anyhow::bail!("unknown step {other:?}"),
            }
            let expected = &step["expect"];
            if expected.is_null() {
                continue;
            }
            if !step["prompt"].is_null() {
                fx.wait("turn/start", |fx| {
                    fx.sent()
                        .is_ok_and(|s| s[before..].iter().any(|r| r["method"] == "turn/start"))
                })?;
                let sent = fx.sent()?;
                let turn = sent[before..]
                    .iter()
                    .find(|r| r["method"] == "turn/start")
                    .context("turn/start")?;
                ensure!(
                    turn["params"]["input"][0]["text"] == rec["result"]["prompt"],
                    "run {run} step {i}: submitted markdown differs: {turn}"
                );
                continue;
            }
            ensure!(
                fx.composer_text()?.replace("\u{2003}\u{2004}", "") == expected["text"],
                "run {run} step {i}: editor text differs"
            );
            ensure!(
                fx.composer_focused()? == expected["focused"],
                "run {run} step {i}: focus differs"
            );
            let chips = fx.read(|r, cx| r.composer(cx).read(cx).chips());
            let want_chips: Vec<_> = expected["chips"]
                .as_array()
                .context("chips")?
                .iter()
                .map(|c| c["label"].as_str().unwrap())
                .collect();
            ensure!(
                chips.iter().map(|c| c.0.as_str()).eq(want_chips),
                "run {run} step {i}: chips differ: {chips:?}"
            );
            let menu = fx.read(|r, cx| r.composer(cx).read(cx).mention_state());
            if expected["menu"].is_null() {
                ensure!(menu.is_none(), "run {run} step {i}: list must close");
            } else {
                let menu = menu.context("mention list")?;
                // Host-only plugins are absent in the port. These recorded keys select the
                // same first/last rows on both lists, without traversing the plugin rows.
                let want = expected["menu"]["sections"]
                    .as_array()
                    .context("sections")?
                    .iter()
                    .flat_map(|s| s["items"].as_array().into_iter().flatten())
                    .find(|r| r["current"] == true)
                    .and_then(|r| r["text"].as_str())
                    .context("highlighted row")?;
                let selected = menu
                    .iter()
                    .flat_map(|s| &s.1)
                    .find(|r| r.1)
                    .context("port highlighted row")?;
                ensure!(
                    selected.0.starts_with(want),
                    "run {run} step {i}: {:?} vs {want}",
                    selected.0
                );
                let image = fx.shot()?;
                let b = &expected["menu"]["bbox"];
                let x = b["x"].as_u64().context("x")? as u32
                    + b["w"].as_u64().context("width")? as u32 / 2;
                let bottom = (b["y"].as_u64().context("y")? + b["h"].as_u64().context("h")?) as u32;
                let scale = image.width() / 1280;
                // The last row ends 5 px above the bottom. Its highlight covers bottom-7;
                // bottom-3 is padding in both the original and the port, below every row.
                ensure!(
                    image.get_pixel(x * scale, (bottom - 3) * scale).0[..3] == [45, 45, 45],
                    "run {run} step {i}: list surface differs"
                );
                ensure!(
                    image.get_pixel(x * scale, (bottom + 1) * scale).0[..3] == [24, 24, 24],
                    "run {run} step {i}: list bottom edge differs"
                );
            }
            ensure!(
                !fx.sent()?[before..]
                    .iter()
                    .any(|r| r["method"] == "turn/start"),
                "run {run} step {i}: Enter or Tab must not send while the list is open"
            );
            if !chips.is_empty() && step["type"] == "key" {
                let image = fx.shot()?;
                image.save(std::env::temp_dir().join(format!("s020-port-{run}-{i}.png")))?;
                // The recorded chip's color is color(srgb .338039 .523137 .818039).
                let scale = image.width() / 1280;
                let b = &expected["chips"][0]["bbox"];
                let x = b["x"].as_u64().context("chip x")? as u32;
                let y = b["y"].as_u64().context("chip y")? as u32;
                let w = b["w"].as_u64().context("chip width")? as u32;
                ensure!(
                    (y * scale..(y + 20) * scale).any(|py| (x * scale..(x + w) * scale).any(
                        |px| {
                            let p = image.get_pixel(px, py).0;
                            p[..3]
                                .iter()
                                .zip([86u8, 133, 209])
                                .all(|(a, b)| a.abs_diff(b) <= 2)
                        }
                    )),
                    "run {run} step {i}: chip color differs"
                );
            }
        }
        let sent = fx.sent()?;
        let want = rec["result"]["requests"].as_array().context("requests")?;
        ensure!(
            fuzzy_requests(&sent[before..]) == fuzzy_requests(want),
            "run {run}: file search requests differ"
        );
    }
    Ok(())
}
