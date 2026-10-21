#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::{Route, SidePanel};
use common::{EXPLAIN, Fx, GREETING, SUBTRACT, exchange, recorded_thread, script};
use serde_json::{Value, json};

fn command(fx: &mut Fx, label: &str) -> Result<()> {
    fx.keys("cmd-k")?;
    fx.type_text(label)?;
    let index = fx
        .read(|root, cx| {
            root.command_menu()
                .unwrap()
                .read(cx)
                .describe()
                .into_iter()
                .flat_map(|(_, rows)| rows)
                .position(|(text, _)| {
                    text.split(" | ").next().unwrap().replace(['[', ']'], "") == label
                })
        })
        .with_context(|| format!("visible command {label}"))?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")?;
    ensure!(
        fx.read(|root, _| root.command_menu().is_none()),
        "{label}: menu remains open"
    );
    Ok(())
}

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn command_menu_toggles_sidebar_like_the_keyboard() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-b")?;
    ensure!(!fx.sidebar_open(), "keyboard hides the sidebar");
    fx.keys("cmd-b")?;
    let route = fx.route();
    command(&mut fx, "Toggle sidebar")?;
    ensure!(
        !fx.sidebar_open(),
        "the menu must hide the sidebar like cmd-b"
    );
    ensure!(fx.route() == route && fx.composer_focused()?);
    fx.advance(std::time::Duration::from_secs(1))?;
    fx.shot()?
        .save(std::env::temp_dir().join("command-dispatch-after-sidebar.png"))?;
    command(&mut fx, "Toggle sidebar")?;
    ensure!(fx.sidebar_open());
    Ok(())
}

#[test]
fn command_navigation_preserves_chat_identity_and_returns_to_the_composer() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    for id in [GREETING, SUBTRACT, GREETING] {
        let mut read = exchange("thread/read", json!({"thread": recorded_thread(id)?}));
        read[0]["msg"]["params"] = json!({"threadId": id, "includeTurns": true});
        lines.extend(read);
    }
    let mut fx = Fx::scripted(lines)?;
    let home = tempfile::tempdir()?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(
            std::sync::Arc::new(app::host::LocalHost::new(home.path())),
            cx,
        );
    });
    command(&mut fx, "Next chat")?;
    fx.wait_thread(GREETING)?;
    command(&mut fx, "Next chat")?;
    fx.wait_thread(SUBTRACT)?;
    command(&mut fx, "Previous chat")?;
    fx.wait_thread(GREETING)?;
    command(&mut fx, "Back")?;
    fx.wait_thread(SUBTRACT)?;
    command(&mut fx, "Forward")?;
    fx.wait_thread(GREETING)?;
    for label in ["New standalone chat", "New chat"] {
        fx.type_text("Discard this draft")?;
        command(&mut fx, label)?;
        ensure!(fx.route() == Route::Home && fx.composer_focused()?);
        ensure!(fx.composer_text()?.is_empty());
    }
    command(&mut fx, "Manage scheduled tasks")?;
    ensure!(fx.route() == Route::Scheduled);
    Ok(())
}

#[test]
fn recent_commands_follow_update_order_instead_of_sidebar_order() -> Result<()> {
    let ids = [
        EXPLAIN,
        GREETING,
        SUBTRACT,
        "dispatch-four",
        "dispatch-five",
        "dispatch-six",
    ];
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        if let Some(data) = line
            .pointer_mut("/msg/result/data")
            .and_then(Value::as_array_mut)
            && data
                .first()
                .is_some_and(|thread| thread["preview"].is_string())
        {
            let template = data[0].clone();
            for id in &ids[3..] {
                let mut thread = template.clone();
                thread["id"] = (*id).into();
                data.push(thread);
            }
            for thread in data {
                let slot = ids.iter().position(|id| thread["id"] == *id).unwrap();
                thread["updatedAt"] = (1_800_000_000 - slot as i64).into();
            }
        }
    }
    for id in &ids[3..] {
        let mut thread = recorded_thread(GREETING)?;
        thread["id"] = (*id).into();
        lines.extend(exchange("thread/read", json!({"thread": thread})));
    }
    let mut fx = Fx::scripted(lines)?;
    for (slot, id) in ids.iter().enumerate() {
        command(&mut fx, &format!("Go to recent chat {}", slot + 1))?;
        fx.wait_thread(id)?;
        ensure!(fx.route() == Route::Thread((*id).into()) && fx.composer_focused()?);
    }
    Ok(())
}

#[test]
fn find_keeps_its_input_focus_after_the_command_menu_closes() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    command(&mut fx, "Find")?;
    fx.type_text("Hello")?;
    fx.shot()?
        .save(std::env::temp_dir().join("command-dispatch-after-find.png"))?;
    ensure!(fx.read(|root, cx| {
        root.thread()
            .unwrap()
            .read(cx)
            .find_state()
            .is_some_and(|(query, _, count)| query == "Hello" && count > 0)
    }));
    ensure!(fx.route() == Route::Thread(GREETING.into()));
    ensure!(
        fx.composer_text()?.is_empty(),
        "Find input reached the composer"
    );
    Ok(())
}

#[test]
fn panel_commands_keep_the_selected_chat_and_existing_tabs() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "command/exec",
        json!({"exitCode":0,"stdout":"","stderr":""}),
    ));
    lines.extend(exchange("command/exec/resize", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    command(&mut fx, "Open Changes tab")?;
    ensure!(fx.read(|root, _| root.side_panel() == Some(SidePanel::Changes)));
    command(&mut fx, "New tab")?;
    ensure!(fx.read(|root, _| root.side_panel().is_none()));
    command(&mut fx, "New tab")?;
    ensure!(fx.read(|root, _| root.side_panel() == Some(SidePanel::Changes)));
    command(&mut fx, "Open terminal")?;
    ensure!(fx.read(|root, cx| {
        root.side_panel() == Some(SidePanel::Changes)
            && root.side_tabs() == [SidePanel::Changes]
            && root.shell().read(cx).bottom.is_some()
            && root.terminal_panel().is_some()
    }));
    ensure!(fx.route() == Route::Thread(SUBTRACT.into()) && !fx.composer_focused()?);
    fx.shot()?
        .save(std::env::temp_dir().join("command-dispatch-after-terminal.png"))?;
    Ok(())
}

#[test]
fn archive_command_removes_the_selected_chat_and_returns_home() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/archive", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    command(&mut fx, "Archive chat")?;
    fx.wait("archive removes the selected chat", |fx| {
        fx.read(|root, _| root.recents() == [GREETING, EXPLAIN])
    })?;
    ensure!(fx.route() == Route::Home && fx.composer_focused()?);
    let archived: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|request| request["method"] == "thread/archive")
        .map(|request| request["params"].clone())
        .collect();
    ensure!(
        archived == [json!({"threadId": SUBTRACT})],
        "archive requests: {archived:?}"
    );
    Ok(())
}
