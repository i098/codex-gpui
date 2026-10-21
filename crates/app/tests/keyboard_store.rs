mod common;

use anyhow::{Result, ensure};
use app::{GoToChat, Route};
use common::{Fx, GREETING, exchange, recorded_thread, script};
use serde_json::json;
use ui::keyboard::{Change, KeyboardState, NumberTarget, Preferences, commands};

fn panic_hook() {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
}

fn change(fx: &mut Fx, change: Change) {
    fx.cx.update(|cx| {
        let dispatch = cx.global::<KeyboardState>().dispatch.clone().unwrap();
        dispatch(change, cx);
    });
    fx.cx.run_until_parked();
}

#[test]
fn edits_reject_conflicts_and_reset_restores_the_number_mode() -> Result<()> {
    panic_hook();
    let mut preferences = Preferences::default();
    let command = commands()
        .iter()
        .find(|command| command.id == "newTask")
        .unwrap();
    preferences
        .change(&Change::Bindings {
            command: "newTask".into(),
            keys: vec!["Command+Shift+Y".into()],
        })
        .map_err(anyhow::Error::msg)?;
    let previous = preferences.clone();
    ensure!(
        preferences
            .change(&Change::Bindings {
                command: "newTask".into(),
                keys: vec!["Command+K".into()]
            })
            .is_err()
    );
    ensure!(preferences == previous);
    preferences
        .change(&Change::NumberTarget(NumberTarget::Sidebar))
        .map_err(anyhow::Error::msg)?;
    preferences
        .change(&Change::Reset)
        .map_err(anyhow::Error::msg)?;
    ensure!(preferences.primary_number_shortcut_target == NumberTarget::Sidebar);
    ensure!(
        preferences.keys(command).collect::<Vec<_>>() == vec!["CmdOrCtrl+N", "CmdOrCtrl+Shift+O"]
    );
    let chat = commands()
        .iter()
        .find(|command| command.id == "thread1")
        .unwrap();
    ensure!(preferences.keys(chat).next() == Some("Command+1"));
    preferences
        .change(&Change::NumberTarget(NumberTarget::Tabs))
        .map_err(anyhow::Error::msg)?;
    ensure!(preferences.keys(chat).next() == Some("Control+1"));
    Ok(())
}

#[test]
fn saved_bindings_replace_live_keys_and_survive_reload() -> Result<()> {
    panic_hook();
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("settings.json");
    let settings = app::host_settings::Settings::load(path.clone())?;
    let mut lines = script(None, |_| {})?;
    let reads: std::collections::HashSet<_> = lines
        .iter()
        .filter(|line| line["msg"]["method"] == "thread/read")
        .map(|line| line["msg"]["id"].to_string())
        .collect();
    lines.retain(|line| !reads.contains(&line["msg"]["id"].to_string()));
    for _ in 0..3 {
        lines.extend(exchange(
            "thread/read",
            json!({"thread": recorded_thread(GREETING)?}),
        ));
    }
    let mut fx = Fx::captured_with_host_settings("dark", (1280., 832.), lines.clone(), settings)?;
    change(
        &mut fx,
        Change::Bindings {
            command: "newTask".into(),
            keys: vec!["Command+Shift+Y".into()],
        },
    );
    ensure!(
        fx.cx
            .update(|cx| cx.global::<KeyboardState>().error.is_none())
    );
    fx.action(Box::new(GoToChat(0)))?;
    fx.wait_thread(GREETING)?;
    fx.keys("cmd-n")?;
    ensure!(
        fx.route() == Route::Thread(GREETING.into()),
        "the old key still opens Home"
    );
    fx.keys("cmd-shift-y")?;
    ensure!(fx.route() == Route::Home);
    drop(fx);
    let settings = app::host_settings::Settings::load(path)?;
    let mut fx = Fx::captured_with_host_settings("dark", (1280., 832.), lines, settings)?;
    fx.action(Box::new(GoToChat(0)))?;
    fx.wait_thread(GREETING)?;
    fx.keys("cmd-shift-y")?;
    ensure!(
        fx.route() == Route::Home,
        "the binding did not survive reload"
    );
    change(&mut fx, Change::Reset);
    fx.action(Box::new(GoToChat(0)))?;
    fx.wait_thread(GREETING)?;
    fx.keys("cmd-n")?;
    ensure!(
        fx.route() == Route::Home,
        "reset did not restore the default key"
    );
    Ok(())
}

#[test]
fn failed_preference_write_keeps_the_previous_bindings() -> Result<()> {
    panic_hook();
    let directory = tempfile::tempdir()?;
    let parent = directory.path().join("settings");
    std::fs::create_dir(&parent)?;
    let mut settings = app::host_settings::Settings::load(parent.join("host.json"))?;
    settings.write(
        "codex-command-keybindings",
        json!({"newTask":["Command+N"]}),
    )?;
    std::fs::remove_file(parent.join("host.json"))?;
    std::fs::remove_dir(&parent)?;
    std::fs::write(&parent, "not a directory")?;
    ensure!(
        settings
            .write_batch([
                ("codex-command-keybindings", json!({})),
                ("external-browser-link-shortcut", json!("unassigned"))
            ])
            .is_err()
    );
    ensure!(settings.read_all()["codex-command-keybindings"] == json!({"newTask":["Command+N"]}));
    ensure!(
        !settings
            .read_all()
            .contains_key("external-browser-link-shortcut")
    );
    Ok(())
}

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn bare_app_shortcut_does_not_intercept_composer_text() -> Result<()> {
    panic_hook();
    let mut fx = Fx::new()?;
    change(
        &mut fx,
        Change::Bindings {
            command: "toggleSidebar".into(),
            keys: vec!["X".into()],
        },
    );
    let open = fx.sidebar_open();
    fx.focus_composer()?;
    fx.type_text("x")?;
    ensure!(fx.composer_text()? == "x");
    ensure!(fx.sidebar_open() == open);
    Ok(())
}
