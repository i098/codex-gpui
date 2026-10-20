#![cfg(target_os = "macos")]
//! Slash commands that run existing chat actions: `/compact`, `/fork`, `/rename`, `/pin`,
//! `/model`, `/reasoning` and `/project`. Each test types the command, presses Enter and
//! compares the requests the app-server received (`Fx::captured`) and the resulting UI.

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host, LocalHost};
use common::{Fx, GREETING, SUBTRACT, exchange, recorded_thread, script};
use serde_json::{Value, json};
use std::sync::Arc;

#[test]
fn proxy() {
    common::proxy_child();
}

fn running(line: &mut Value) {
    if line.pointer("/msg/result/thread/id") == Some(&json!(GREETING))
        && let Some(status) = line.pointer_mut("/msg/result/thread/turns/0/status")
    {
        *status = json!("inProgress");
    }
}

fn captured(lines: Vec<Value>) -> Result<Fx> {
    Fx::captured("dark", (1280., 832.), lines)
}

/// Saves the window for the before/after review (`CGX_SLASH_SHOTS` names the directory).
fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(dir) = std::env::var_os("CGX_SLASH_SHOTS") {
        fx.shot()?
            .save(std::path::Path::new(&dir).join(format!("{name}.png")))?;
    }
    Ok(())
}

/// Opens the recorded chat and focuses its docked composer.
fn open_chat(fx: &mut Fx) -> Result<()> {
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.click(fx.at("thread", "Work with Codex")?)?;
    ensure!(fx.composer_focused()?, "the composer must have focus");
    Ok(())
}

fn rows(fx: &Fx) -> Result<Vec<String>> {
    let (rows, _) = fx
        .read(|root, cx| root.composer(cx).read(cx).slash_state())
        .context("slash menu must be open")?;
    Ok(rows
        .into_iter()
        .map(|row| row.split('\n').next().unwrap().to_owned())
        .collect())
}

fn requests(fx: &mut Fx, method: &str) -> Result<Vec<Value>> {
    Ok(fx
        .sent()?
        .into_iter()
        .filter(|request| request["method"] == method)
        .collect())
}

fn config_write(fx: &mut Fx) -> Result<Value> {
    let writes = requests(fx, "config/batchWrite")?;
    Ok(writes.last().context("config/batchWrite")?["params"]["edits"].clone())
}

#[test]
fn compact_starts_on_the_open_chat_and_reports_a_refusal() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "thread/resume",
        json!({"thread": recorded_thread(GREETING)?}),
    ));
    // No recorded `thread/compact/start`: the replay answers the first one with an error.
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("/compact")?;
    ensure!(rows(&fx)? == ["Compact"]);
    fx.keys("enter")?;
    fx.wait("failed compact toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|t| t == "Failed to compact chat")
        })
    })?;
    let starts = requests(&mut fx, "thread/compact/start")?;
    ensure!(starts.len() == 1 && starts[0]["params"] == json!({"threadId": GREETING}));
    ensure!(fx.composer_text()?.is_empty() && fx.composer_focused()?);

    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "thread/resume",
        json!({"thread": recorded_thread(GREETING)?}),
    ));
    lines.extend(exchange("thread/compact/start", json!({})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("/compact")?;
    fx.keys("enter")?;
    fx.wait("compact request", |fx| {
        requests(fx, "thread/compact/start").is_ok_and(|r| r.len() == 1)
    })?;
    ensure!(fx.read(|root, cx| root.toasts(cx).is_empty()));
    Ok(())
}

#[test]
fn compact_is_refused_while_a_turn_runs() -> Result<()> {
    let mut lines = script(None, running)?;
    lines.extend(exchange("thread/compact/start", json!({})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("/compact")?;
    fx.keys("enter")?;
    ensure!(fx.read(|root, cx| {
        root.toasts(cx)
            .iter()
            .any(|t| t == "Compact is disabled while a chat is in progress")
    }));
    ensure!(requests(&mut fx, "thread/compact/start")?.is_empty());
    Ok(())
}

#[test]
fn rename_opens_the_dialog_and_returns_focus_to_the_composer() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/name/set", json!({})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("/rename")?;
    fx.keys("enter")?;
    ensure!(
        fx.read(|root, _| root.dialog().is_some()),
        "Rename chat opens"
    );
    fx.keys("escape")?;
    ensure!(fx.read(|root, _| root.dialog().is_none()));
    ensure!(
        fx.composer_focused()?,
        "Cancel returns focus to the composer"
    );
    ensure!(requests(&mut fx, "thread/name/set")?.is_empty());

    fx.type_text("/rename")?;
    fx.keys("enter")?;
    fx.type_text("Greeting tweak")?;
    fx.keys("enter")?;
    fx.wait("name write", |fx| {
        requests(fx, "thread/name/set").is_ok_and(|r| r.len() == 1)
    })?;
    ensure!(
        requests(&mut fx, "thread/name/set")?[0]["params"]
            == json!({"threadId": GREETING, "name": "Greeting tweak"})
    );
    ensure!(fx.composer_focused()?, "Save returns focus to the composer");
    Ok(())
}

#[test]
fn pin_moves_the_chat_into_and_out_of_the_pinned_section() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/section/move", json!({})));
    lines.extend(exchange("thread/section/move", json!({})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    for (n, expected) in [Some(ui::screens::sidebar::PINNED_SECTION_ID), None]
        .into_iter()
        .enumerate()
    {
        fx.type_text("/pin")?;
        fx.keys("enter")?;
        fx.wait("section move", |fx| {
            requests(fx, "thread/section/move").is_ok_and(|r| r.len() == n + 1)
        })?;
        let moves = requests(&mut fx, "thread/section/move")?;
        ensure!(
            moves.last().unwrap()["params"]["sectionId"] == json!(expected),
            "pin toggles: {moves:?}"
        );
        ensure!(moves.last().unwrap()["params"]["threadId"] == GREETING);
    }
    Ok(())
}

#[test]
fn fork_submenu_forks_in_the_same_workspace_and_opens_the_fork() -> Result<()> {
    let mut forked = recorded_thread(SUBTRACT)?;
    forked["id"] = "forked-chat".into();
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/fork", json!({"thread": forked})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("/fork")?;
    fx.keys("enter")?;
    ensure!(
        fx.composer_text()? == "/fork ",
        "the token opens its submenu"
    );
    ensure!(rows(&fx)? == ["Fork chat"]);
    shot(&mut fx, "fork-submenu")?;
    ensure!(
        requests(&mut fx, "thread/fork")?.is_empty(),
        "opening is not a pick"
    );
    // Backspace over the space returns to the command list.
    fx.keys("backspace")?;
    ensure!(rows(&fx)? == ["Fork chat"] && fx.composer_text()? == "/fork");
    fx.keys("enter enter")?;
    fx.wait_thread("forked-chat")?;
    let forks = requests(&mut fx, "thread/fork")?;
    ensure!(forks.len() == 1 && forks[0]["params"] == json!({"threadId": GREETING}));
    ensure!(requests(&mut fx, "thread/rollback")?.is_empty());
    Ok(())
}

#[test]
fn fork_failure_is_reported_and_keeps_the_chat() -> Result<()> {
    let mut fx = captured(script(None, |_| {})?)?;
    open_chat(&mut fx)?;
    fx.type_text("/fork")?;
    fx.keys("enter enter")?;
    fx.wait("fork toast", |fx| {
        fx.read(|root, cx| root.toasts(cx).iter().any(|t| t == "Failed to fork chat"))
    })?;
    ensure!(fx.route() == app::Route::Thread(GREETING.into()));
    Ok(())
}

#[test]
fn model_submenu_filters_keeps_the_draft_and_writes_the_pick() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("config/batchWrite", json!({})));
    lines.extend(exchange("config/batchWrite", json!({})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("draft /model")?;
    shot(&mut fx, "model-before-command-list")?;
    fx.keys("enter")?;
    ensure!(fx.composer_text()? == "draft /model ");
    ensure!(rows(&fx)?.len() == 7, "every model is offered");
    fx.type_text("luna")?;
    ensure!(
        rows(&fx)? == ["GPT-6 Luna", "GPT-5.6 Luna"],
        "{:?}",
        rows(&fx)?
    );
    shot(&mut fx, "model-submenu-filtered")?;
    fx.keys("down")?;
    fx.keys("enter")?;
    shot(&mut fx, "model-after-pick")?;
    ensure!(fx.composer_text()? == "draft " && fx.composer_focused()?);
    fx.wait("model write", |fx| {
        requests(fx, "config/batchWrite").is_ok_and(|r| !r.is_empty())
    })?;
    let edits = config_write(&mut fx)?;
    ensure!(
        edits[0]["keyPath"] == "model" && edits[0]["value"] == "gpt-5.6-luna",
        "{edits}"
    );

    // Escape closes the submenu and leaves the typed text.
    fx.keys("cmd-a backspace")?;
    fx.type_text("/model lu")?;
    fx.keys("escape")?;
    ensure!(fx.read(|root, cx| root.composer(cx).read(cx).slash_state().is_none()));
    ensure!(fx.composer_text()? == "/model lu");
    Ok(())
}

#[test]
fn reasoning_submenu_writes_the_picked_effort() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("config/batchWrite", json!({})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("/reasoning")?;
    fx.keys("enter")?;
    shot(&mut fx, "reasoning-submenu")?;
    ensure!(
        rows(&fx)? == ["Light", "Medium", "High", "Extra High", "Ultra"],
        "{:?}",
        rows(&fx)?
    );
    fx.type_text("extra")?;
    fx.keys("enter")?;
    fx.wait("effort write", |fx| {
        requests(fx, "config/batchWrite").is_ok_and(|r| !r.is_empty())
    })?;
    let edits = config_write(&mut fx)?;
    ensure!(
        edits[1]["keyPath"] == "model_reasoning_effort" && edits[1]["value"] == "xhigh",
        "{edits}"
    );
    ensure!(fx.composer_text()?.is_empty() && fx.composer_focused()?);
    Ok(())
}

#[test]
fn project_submenu_selects_an_existing_project_for_the_next_chat() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let host = Arc::new(LocalHost::new(temp.path().join("codex")));
    let [alpha, beta] = ["alpha", "beta"].map(|name| temp.path().join(name));
    std::fs::create_dir(&alpha)?;
    std::fs::create_dir(&beta)?;
    let [_, beta] = [alpha, beta].map(|root| host.add_project(&root).unwrap());
    let mut new = recorded_thread(GREETING)?;
    new["id"] = "in-beta".into();
    new["cwd"] = json!(beta);
    new["turns"] = json!([]);
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "thread/start",
        json!({"thread": new, "model": "gpt-6.1-sol", "cwd": beta}),
    ));
    lines.extend(exchange(
        "turn/start",
        json!({"turn": {"id": "t", "items": [], "status": "completed", "error": null}}),
    ));
    let mut fx = captured(lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.wait("project rows", |fx| {
        fx.read(|root, cx| root.sidebar().read(cx).projects().len() == 2)
    })?;
    ensure!(fx.route() == app::Route::Home);
    fx.click(fx.at("home", "Do anything")?)?;
    fx.type_text("/project")?;
    fx.keys("enter")?;
    ensure!(rows(&fx)?.len() == 2, "{:?}", rows(&fx)?);
    fx.type_text("bet")?;
    ensure!(rows(&fx)? == ["beta"]);
    shot(&mut fx, "project-submenu")?;
    fx.keys("enter")?;
    ensure!(fx.composer_text()?.is_empty() && fx.composer_focused()?);
    fx.type_text("Start here")?;
    fx.keys("enter")?;
    fx.wait_thread("in-beta")?;
    let starts = requests(&mut fx, "thread/start")?;
    ensure!(
        starts.len() == 1 && starts[0]["params"]["cwd"] == json!(beta),
        "{starts:?}"
    );
    Ok(())
}
