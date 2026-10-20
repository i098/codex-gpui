#![cfg(target_os = "macos")]

mod common;

use anyhow::{Result, ensure};
use common::{CWD, Fx, GREETING, SUBTRACT, exchange, recorded_thread, script};
use gpui::{px, size};
use serde_json::json;
use ui::screens::sidebar::SelectProject;

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn standalone_chat_discards_the_selected_project_and_draft() -> Result<()> {
    let mut fx = Fx::captured(
        "dark",
        (1280., 832.),
        script(Some("sessions/greeting.jsonl"), |_| {})?,
    )?;
    let project = tempfile::tempdir()?;
    fx.action(Box::new(SelectProject(
        project.path().to_str().unwrap().into(),
    )))?;
    fx.type_text("Discard this project draft")?;
    fx.keys("cmd-alt-o")?;
    ensure!(fx.route() == app::Route::Home && fx.composer_focused()?);
    ensure!(fx.composer_text()?.is_empty());
    fx.type_text("Start outside the selected project")?;
    fx.keys("enter")?;
    fx.wait_thread(GREETING)?;
    let requests = fx.sent()?;
    let start = requests
        .iter()
        .find(|m| m["method"] == "thread/start")
        .unwrap();
    ensure!(
        start["params"]["cwd"] == CWD,
        "standalone chat reused the selected project"
    );
    Ok(())
}

#[test]
fn zoom_preserves_draft_focus_and_scaled_pointer_targets() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    let mut read = exchange("thread/read", json!({"thread": recorded_thread(SUBTRACT)?}));
    read[0]["msg"]["params"] = json!({"threadId": SUBTRACT, "includeTurns": true});
    lines.extend(read);
    let mut fx = Fx::scripted(lines)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.type_text("Keep this draft")?;
    fx.keys("cmd-=")?;
    ensure!(fx.composer_focused()? && fx.composer_text()? == "Keep this draft");
    fx.cx.update_window(fx.window, |_, window, _| {
        assert_eq!(window.content_zoom(), 1.1);
        assert_eq!(window.viewport_size(), size(px(1280.), px(832.)) / 1.1);
        assert_eq!(window.bounds().size, size(px(1280.), px(832.)));
    })?;
    fx.keys("cmd-a")?;
    fx.keys("backspace")?;
    if let Some(directory) = std::env::var_os("CGX_SHELL_PROOF") {
        std::fs::create_dir_all(&directory)?;
        fx.reference_shot()?
            .save(std::path::Path::new(&directory).join("native-zoom.png"))?;
    }
    fx.action(Box::new(ui::OpenScheduled))?;
    fx.click((26. * 1.1, 70. * 1.1))?;
    ensure!(fx.route() == app::Route::Home && fx.composer_focused()?);
    ensure!(fx.composer_text()?.is_empty());
    fx.keys("cmd--")?;
    fx.cx.update_window(fx.window, |_, window, _| {
        assert_eq!(window.content_zoom(), 1.);
    })?;
    fx.keys("cmd-=")?;
    fx.keys("cmd-0")?;
    fx.cx
        .simulate_window_resize(fx.window, size(px(1000.), px(700.)));
    fx.cx.update_window(fx.window, |_, window, _| {
        assert_eq!(window.content_zoom(), 1.);
        assert_eq!(window.viewport_size(), size(px(1000.), px(700.)));
    })?;
    Ok(())
}
