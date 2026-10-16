mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, script};
use serde_json::Value;

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn slash_menu_keyboard_replay() -> Result<()> {
    if !cfg!(target_os = "macos") {
        eprintln!("S021 rendering requires the macOS headless renderer");
        return Ok(());
    }
    let recording: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s021.json"))?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    let before = fx.sent()?.len();
    for (index, step) in recording["steps"]
        .as_array()
        .context("steps")?
        .iter()
        .enumerate()
    {
        match step["type"].as_str() {
            Some("click") => fx.click(fx.at("home", "Do anything")?)?,
            Some("type") => fx.type_text(step["text"].as_str().context("text")?)?,
            Some("key") => fx.keys(step["portKey"].as_str().context("portKey")?)?,
            other => anyhow::bail!("unknown recorded step: {other:?}"),
        }
        let expected = &step["expect"];
        ensure!(
            fx.composer_text()? == expected["text"],
            "step {index}: editor text differs"
        );
        ensure!(
            fx.composer_focused()? == expected["focused"],
            "step {index}: editor focus differs"
        );
        let state = fx.read(|root, cx| root.composer(cx).read(cx).slash_state());
        if expected["menu"].is_null() {
            ensure!(state.is_none(), "step {index}: slash menu must close");
            continue;
        }
        let (items, selected) = state.context(format!("step {index}: slash menu did not open"))?;
        let commands = expected["menu"]["commands"]
            .as_array()
            .context("commands")?;
        let titles: Vec<_> = commands
            .iter()
            .map(|command| command["text"].as_str().unwrap())
            .collect();
        ensure!(
            items
                .iter()
                .take(titles.len())
                .map(String::as_str)
                .eq(titles),
            "step {index}: command list differs: {items:?}"
        );
        if expected["menu"]["skills"] == false {
            ensure!(
                items.len() == commands.len(),
                "step {index}: unrecorded skill rows: {items:?}"
            );
        }
        let expected_selected = expected["menu"]["selected"]["text"].as_str();
        ensure!(
            selected.map(|i| items[i].as_str()) == expected_selected,
            "step {index}: selection differs"
        );
        if expected["menu"]["empty"] == true {
            ensure!(
                items.is_empty(),
                "step {index}: unmatched query must show No commands"
            );
        }
        let image = fx.shot()?;
        if index == 1 || index == 8 {
            image.save(std::env::temp_dir().join(format!("cgx-s021-port-{index}.png")))?;
        }
        let b = &expected["menu"]["bbox"];
        let x = b["x"].as_u64().context("menu x")? as u32;
        let y = b["y"].as_u64().context("menu y")? as u32;
        let w = b["w"].as_u64().context("menu width")? as u32;
        let scale = image.width() / 1280;
        ensure!(
            image.get_pixel((x + w - 7) * scale, (y + 22) * scale).0[..3] == [45, 45, 45],
            "step {index}: recorded menu surface differs"
        );
        if let Some(b) = expected["menu"]["selected"].get("bbox") {
            let x = b["x"].as_u64().context("selected x")? as u32;
            let y = b["y"].as_u64().context("selected y")? as u32;
            let w = b["w"].as_u64().context("selected width")? as u32;
            ensure!(
                image.get_pixel((x + w - 12) * scale, (y + 14) * scale).0[..3] == [62, 62, 62],
                "step {index}: selected row color differs"
            );
        }
    }
    let sent = fx.sent()?;
    let requests: Vec<_> = sent[before..]
        .iter()
        .map(|request| {
            serde_json::json!({
                "type": "mcp-request", "method": request["method"], "params": request["params"]
            })
        })
        .collect();
    ensure!(
        requests
            == recording["result"]["requests"]
                .as_array()
                .context("requests")?
                .clone(),
        "slash activation must send only the recorded requests: {requests:?}"
    );
    ensure!(
        fx.read(|root, cx| root.composer(cx).read(cx).fast_mode()),
        "Fast must enable priority mode"
    );
    Ok(())
}

#[test]
fn slash_commands_preserve_drafts_and_filter_empty_only_actions() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = Fx::new()?;
    fx.keys("cmd-1")?;
    fx.wait_thread(common::GREETING)?;
    fx.click(fx.at("thread", "Work with Codex")?)?;
    fx.type_text("/fast")?;
    fx.keys("ctrl-n")?;
    let state = fx
        .read(|root, cx| root.composer(cx).read(cx).slash_state())
        .context("slash menu")?;
    ensure!(
        state.0[state.1.context("selection")?].starts_with("Ultrafast\n"),
        "Ctrl+N must move down"
    );
    fx.keys("ctrl-p")?;
    let state = fx
        .read(|root, cx| root.composer(cx).read(cx).slash_state())
        .context("slash menu")?;
    ensure!(
        state.0[state.1.context("selection")?].starts_with("Fast\n"),
        "Ctrl+P must move up"
    );
    fx.keys("escape cmd-a backspace")?;
    fx.type_text("draft /new")?;
    let state = fx
        .read(|root, cx| root.composer(cx).read(cx).slash_state())
        .context("slash menu")?;
    ensure!(
        !state.0.iter().any(|item| item.starts_with("New chat\n")),
        "New chat requires an empty draft"
    );
    fx.keys("escape cmd-a backspace")?;
    fx.type_text("/new")?;
    fx.keys("enter")?;
    ensure!(
        fx.route() == app::Route::Home,
        "New chat must navigate home"
    );
    ensure!(
        fx.composer_text()?.is_empty() && fx.composer_focused()?,
        "New chat must focus an empty composer"
    );
    fx.type_text("keep this /fast")?;
    fx.keys("tab")?;
    ensure!(
        fx.composer_text()? == "keep this ",
        "A command must remove only its own token"
    );
    Ok(())
}
