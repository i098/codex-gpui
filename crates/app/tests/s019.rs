//! S019: `@` opens the mention list in the thread composer; typing after it searches the
//! workspace files (`fuzzyFileSearch/sessionStart` + `sessionUpdate`, notifications
//! `sessionUpdated` and `sessionCompleted`). Replays refs/interactions/s019.json, recorded from
//! the original on route1 with a mock app-server that lists hello.py, README.md, src/math.py,
//! src/matrix.rs and docs/notes.md.
#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, script};
use gpui::EntityInputHandler as _;
use serde_json::{Value, json};

type RecordedSection = (Option<String>, Vec<(String, bool)>);

#[test]
fn proxy() {
    common::proxy_child();
}

/// Rows of the recorded `Add` and `Files` sections the port lists (the original's Plugins,
/// Agents, Pages and Spaces come from host services the port does not have).
fn recorded_sections(menu: &Value) -> Vec<RecordedSection> {
    menu["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| {
            matches!(
                s["id"].as_str(),
                Some("context-actions" | "files" | "search-results")
            )
        })
        .map(|s| {
            let heading = s["heading"]["text"].as_str().map(str::to_owned);
            let rows = s["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| {
                    let text = match i["text"].as_str().unwrap_or_default() {
                        // The Pages host-service error replaces the generic empty state in
                        // the original (`oYc`); without Pages (F4) it shows `No results`.
                        "Couldn’t search Pages" => "No results",
                        text => text,
                    };
                    (text.to_owned(), i["current"] == true)
                })
                .collect();
            (heading, rows)
        })
        .collect()
}

/// The recorded `sessionUpdated` + `sessionCompleted` of the route1 mock for `query`.
fn search_notifications(session: &str, query: &str, files: &[(&str, &str)]) -> Vec<Value> {
    let files: Vec<_> = files
        .iter()
        .map(|(path, name)| {
            json!({ "root": common::CWD, "path": path, "match_type": "file", "file_name": name, "score": 84, "indices": [] })
        })
        .collect();
    vec![
        json!({ "dir": "recv", "msg": { "method": "fuzzyFileSearch/sessionUpdated", "params": { "sessionId": session, "query": query, "files": files } } }),
        json!({ "dir": "recv", "msg": { "method": "fuzzyFileSearch/sessionCompleted", "params": { "sessionId": session } } }),
    ]
}

fn fuzzy_requests(sent: &[Value]) -> Vec<&Value> {
    sent.iter()
        .filter(|r| {
            r["method"]
                .as_str()
                .is_some_and(|m| m.starts_with("fuzzyFileSearch/"))
        })
        .collect()
}

/// Inserts `text` at the cursor in one edit, like the recorder's `Input.insertText`.
fn insert(fx: &mut Fx, text: &str) -> Result<()> {
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        composer.update(cx, |c, cx| c.replace_text_in_range(None, text, window, cx));
    })?;
    fx.cx.run_until_parked();
    Ok(())
}

#[test]
fn mention_list_replay() -> Result<()> {
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s019.json"))?;
    // The replay answers `fuzzyFileSearch/*` like the route1 mock: `{}` to each request, then
    // the notifications of the query after `sessionUpdate`. The replay matches requests by
    // method and emits what follows the recorded response.
    let mut lines = script(None, |_| {})?;
    lines.extend(common::exchange("fuzzyFileSearch/sessionStart", json!({})));
    lines.extend(common::exchange("fuzzyFileSearch/sessionUpdate", json!({})));
    lines.extend(search_notifications(
        "fuzzy-1",
        "mat",
        &[("src/math.py", "math.py"), ("src/matrix.rs", "matrix.rs")],
    ));
    lines.extend(common::exchange("fuzzyFileSearch/sessionUpdate", json!({})));
    lines.extend(search_notifications("fuzzy-1", "matzzz", &[]));
    lines.extend(common::exchange("fuzzyFileSearch/sessionStop", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let before = fx.sent()?.len();
    let mut session: Option<String> = None;
    let mut seen = 0;
    for (index, step) in recording["steps"]
        .as_array()
        .context("steps")?
        .iter()
        .enumerate()
    {
        match (step["type"].as_str(), index) {
            (Some("click"), 0) => {
                fx.keys("cmd-1")?;
                fx.wait_thread(common::GREETING)?;
            }
            (Some("click"), _) => fx.click(fx.at("thread", "Work with Codex")?)?,
            (Some("type"), _) => {
                insert(&mut fx, step["text"].as_str().context("text")?)?;
                if let Some(requests) = step["requests"].as_array() {
                    // The port awaits `sessionStart` before `sessionUpdate`, like the
                    // original; the replay answers at once (the original was sampled 150 ms
                    // after typing).
                    let want = requests.len();
                    fx.wait("search requests", |fx| {
                        fx.sent()
                            .is_ok_and(|sent| fuzzy_requests(&sent[before..]).len() >= want)
                    })?;
                    fx.wait("search answer", |fx| {
                        fx.read(|root, cx| root.composer(cx).read(cx).mention_state())
                            .is_some_and(|s| s[0].1.iter().all(|(row, _)| row != "Searching…"))
                    })?;
                }
            }
            (Some("wait"), _) => fx.wait("file search results", |fx| {
                fx.read(|root, cx| root.composer(cx).read(cx).mention_state())
                    .is_some_and(|s| s[0].1.iter().any(|(row, _)| row.starts_with("math.py")))
            })?,
            (Some("key"), _) => fx.keys(step["portKey"].as_str().context("portKey")?)?,
            other => anyhow::bail!("unknown recorded step: {other:?}"),
        }
        let expected = &step["expect"];
        if expected.is_null() {
            continue;
        }
        ensure!(
            fx.composer_text()? == expected["text"],
            "step {index}: editor text differs"
        );
        ensure!(
            fx.composer_focused()? == expected["focused"],
            "step {index}: editor focus differs"
        );
        let state = fx.read(|root, cx| root.composer(cx).read(cx).mention_state());
        if expected["menu"].is_null() {
            ensure!(state.is_none(), "step {index}: mention list must close");
            continue;
        }
        let state = state.context(format!("step {index}: mention list did not open"))?;
        // The replay answers at once, so the files of a pending query (recorded 150 ms after
        // typing) may already be listed: the next `wait` step records the order then.
        let order = match recording["steps"].get(index + 1) {
            Some(next) if next["type"] == "wait" => &next["expect"]["menu"],
            _ => &expected["menu"],
        };
        let recorded = recorded_sections(order);
        for ((heading, rows), (want_heading, want_rows)) in state.iter().zip(&recorded) {
            ensure!(
                heading.map(str::to_owned) == *want_heading,
                "step {index}: section heading {heading:?} differs from {want_heading:?}"
            );
            // The ranked list also holds plugin rows the port does not have (Browser,
            // Computer, Pets): the port's rows must appear in the recorded order.
            let mut recorded = want_rows.iter();
            for row in rows {
                // The recording keeps 120 chars of a row's text.
                ensure!(
                    recorded.any(|(text, current)| row.0.starts_with(text) && *current == row.1),
                    "step {index}: row {row:?} not in the recorded order {want_rows:?}"
                );
            }
        }
        ensure!(
            state.len() == recorded.len(),
            "step {index}: {} sections, recorded {}",
            state.len(),
            recorded.len()
        );
        if let Some(requests) = step["requests"].as_array() {
            // Each step records the requests since its own start.
            let sent = fx.sent()?;
            let fuzzy = fuzzy_requests(&sent[before..]);
            ensure!(
                fuzzy.len() == seen + requests.len(),
                "step {index}: sent {fuzzy:?}, recorded {requests:?}"
            );
            for (sent, want) in fuzzy[seen..].iter().zip(requests) {
                ensure!(
                    sent["method"] == want["method"],
                    "step {index}: {sent} vs {want}"
                );
                for key in ["roots", "query"] {
                    ensure!(
                        sent["params"][key] == want["params"][key],
                        "step {index}: {key} {} vs {}",
                        sent["params"][key],
                        want["params"][key]
                    );
                }
                let id = sent["params"]["sessionId"].as_str().context("sessionId")?;
                ensure!(
                    session.get_or_insert_with(|| id.to_owned()) == id,
                    "step {index}: one session per `@` token"
                );
            }
            seen = fuzzy.len();
        }
        let image = fx.shot()?;
        if index == 2 || index == 4 {
            image.save(std::env::temp_dir().join(format!("cgx-s019-port-{index}.png")))?;
        }
        let scale = image.width() / 1280;
        let b = &expected["menu"]["bbox"];
        let (x, w) = (
            b["x"].as_u64().context("x")? as u32,
            b["w"].as_u64().context("w")? as u32,
        );
        // The port's popover ends 4 px above the card like the original's; its top depends
        // on the rows it lists, so the surface color is probed under the last row's middle
        // and the backdrop right under the recorded bottom edge.
        let bottom = (b["y"].as_u64().context("y")? + b["h"].as_u64().context("h")?) as u32;
        let probe = |x: u32, y: u32| image.get_pixel(x * scale, y * scale).0[..3].to_vec();
        ensure!(
            probe(x + w / 2, bottom - 7) == [45, 45, 45],
            "step {index}: recorded menu surface differs"
        );
        ensure!(
            probe(x + w / 2, bottom + 1) == [24, 24, 24],
            "step {index}: the list must end at the recorded bottom edge"
        );
    }
    let sent = fx.sent()?;
    let stop = sent[before..]
        .iter()
        .filter(|r| r["method"] == "fuzzyFileSearch/sessionStop")
        .count();
    ensure!(
        stop == 1,
        "Escape must stop the search session once, sent {stop}"
    );
    Ok(())
}
