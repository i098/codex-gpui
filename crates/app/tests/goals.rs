mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, exchange, script};
use serde_json::json;

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn goal_commands_preserve_drafts_and_show_server_state() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut lines = script(None, |line| {
        if let Some(goal) = line.pointer_mut("/msg/result/goal") {
            *goal = json!({"threadId": common::GREETING, "objective": "Ship the native app", "status": "paused"});
        }
    })?;
    let goal = json!({"threadId": common::GREETING, "objective": "Ship the native app", "status": "paused"});
    lines.extend(exchange("thread/goal/get", json!({"goal": goal})));
    let mut active = goal.clone();
    active["status"] = "active".into();
    lines.extend(exchange("thread/goal/set", json!({"goal": active})));
    lines.extend(exchange("thread/goal/set", json!({"goal": goal})));
    lines.extend(exchange("thread/goal/clear", json!({"cleared": true})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(common::GREETING)?;
    fx.click(fx.at("thread", "Work with Codex")?)?;
    fx.type_text("draft /goal")?;
    fx.keys("enter")?;
    fx.shot()?
        .save(std::env::temp_dir().join("cgx-r4-goal-open.png"))?;
    fx.wait("stored goal in submenu", |fx| {
        fx.read(|root, cx| root.composer(cx).read(cx).slash_state())
            .is_some_and(|(items, _)| items.iter().any(|row| row.contains("Ship the native app")))
    })?;
    ensure!(
        fx.read(|root, cx| root.toasts(cx).is_empty()),
        "Opening the goal menu must not create a status toast"
    );
    fx.shot()?
        .save(std::env::temp_dir().join("cgx-r4-goal-ready.png"))?;
    fx.keys("escape")?;
    ensure!(
        fx.composer_text()? == "draft /goal ",
        "Cancel must preserve the goal command and draft"
    );
    fx.keys("cmd-a backspace")?;
    fx.type_text("draft /goal")?;
    fx.keys("enter")?;
    fx.type_text("Ship the native app")?;
    fx.keys("enter")?;
    ensure!(
        fx.composer_text()? == "draft ",
        "Setting a goal must preserve the surrounding draft"
    );
    fx.wait("goal set", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|title| title.contains("Goal (active)"))
        })
    })?;
    let sent = fx.sent()?;
    let set = sent
        .iter()
        .find(|request| request["method"] == "thread/goal/set")
        .context("set request")?;
    ensure!(
        set["params"]["objective"] == "Ship the native app" && set["params"]["origin"] == "user",
        "The goal must use the typed objective and explicit user origin"
    );
    fx.keys("cmd-a backspace")?;
    fx.type_text("/goal")?;
    fx.keys("enter")?;
    fx.type_text("Keep this draft")?;
    fx.keys("down down enter")?;
    ensure!(
        fx.composer_text()? == "Keep this draft",
        "A goal status action must preserve the typed draft"
    );
    fx.wait("goal paused", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|title| title.contains("Goal (paused)"))
        })
    })?;
    fx.keys("cmd-a backspace")?;
    fx.type_text("/goal")?;
    fx.keys("enter down down down down enter")?;
    fx.wait("goal cleared", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|title| title.as_ref() == "No goal for this chat")
        })
    })?;
    Ok(())
}

#[test]
fn thread_memory_commands_do_not_change_global_memory_settings() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/memoryMode/set", json!({})));
    lines.extend(exchange("thread/memoryMode/set", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(common::GREETING)?;
    fx.click(fx.at("thread", "Work with Codex")?)?;
    let before = fx.sent()?.len();
    for (keys, mode) in [("enter down enter", "disabled"), ("enter enter", "enabled")] {
        fx.type_text("/memory")?;
        fx.keys(keys)?;
        fx.wait("memory mode saved", |fx| {
            fx.read(|root, cx| {
                root.toasts(cx)
                    .iter()
                    .any(|title| title.as_ref() == format!("Thread memory {mode}"))
            })
        })?;
    }
    let sent = fx.sent()?;
    let changes: Vec<_> = sent[before..]
        .iter()
        .filter(|request| request["method"] == "thread/memoryMode/set")
        .collect();
    ensure!(changes[0]["params"] == json!({"threadId": common::GREETING, "mode": "disabled"}));
    ensure!(changes[1]["params"] == json!({"threadId": common::GREETING, "mode": "enabled"}));
    ensure!(
        !sent[before..]
            .iter()
            .any(|request| request["method"] == "config/batchWrite"),
        "Thread memory must not change global configuration"
    );
    Ok(())
}
