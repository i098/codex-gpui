mod common;

use anyhow::{Result, ensure};
use app::{Route, SidePanel};
use common::{EXPLAIN, Fx, GREETING, SUBTRACT, exchange, recorded_thread, script};
use gpui::{
    KeyUpEvent, Keystroke, ModifiersChangedEvent, MouseButton, NavigationDirection, PlatformInput,
};
use serde_json::json;

#[test]
fn recent_chats_commit_on_control_release_and_keep_mru_order() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    for id in [SUBTRACT, EXPLAIN] {
        lines.extend(exchange(
            "thread/read",
            json!({ "thread": recorded_thread(id)? }),
        ));
    }
    let mut fx = Fx::scripted(lines)?;
    for (key, id) in [("cmd-1", GREETING), ("cmd-2", SUBTRACT), ("cmd-3", EXPLAIN)] {
        fx.keys(key)?;
        fx.wait_thread(id)?;
    }
    fx.keys("ctrl-tab")?;
    ensure!(fx.route() == Route::Thread(EXPLAIN.into()));
    let rows = fx.read(|root, cx| root.command_menu().unwrap().read(cx).describe());
    ensure!(rows.len() == 1 && rows[0].0 == "Recently viewed");
    ensure!(rows[0].1.len() == 3);
    ensure!(rows[0].1[0].0.starts_with("In two sentences"));
    ensure!(rows[0].1[1].0.starts_with("Add a subtract"));
    ensure!(!rows[0].1.iter().any(|(row, _)| row.contains('⌃')));
    fx.keys("ctrl-tab ctrl-shift-tab")?;
    ensure!(fx.route() == Route::Thread(EXPLAIN.into()));
    if let Some(out) = std::env::var_os("CGX_SHORTCUT_PROOF") {
        std::fs::create_dir_all(&out)?;
        fx.shot()?
            .save(std::path::Path::new(&out).join("native-mru.png"))?;
    }
    fx.input(PlatformInput::ModifiersChanged(
        ModifiersChangedEvent::default(),
    ))?;
    fx.wait_thread(SUBTRACT)?;
    ensure!(fx.read(|root, _| root.command_menu().is_none()));
    fx.keys("ctrl-tab")?;
    fx.input(PlatformInput::KeyUp(KeyUpEvent {
        keystroke: Keystroke {
            key: "control".into(),
            ..Default::default()
        },
    }))?;
    fx.wait_thread(EXPLAIN)?;
    fx.keys("ctrl-tab escape")?;
    ensure!(fx.route() == Route::Thread(EXPLAIN.into()));
    ensure!(fx.read(|root, _| root.command_menu().is_none()));
    Ok(())
}

#[test]
fn mouse_history_and_changes_shortcut_use_existing_navigation() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("cmd-,")?;
    fx.press(
        (100., 22.),
        MouseButton::Navigate(NavigationDirection::Back),
    )?;
    fx.wait_thread(SUBTRACT)?;
    fx.press(
        (100., 22.),
        MouseButton::Navigate(NavigationDirection::Forward),
    )?;
    ensure!(fx.route() == Route::Settings);
    fx.press(
        (100., 22.),
        MouseButton::Navigate(NavigationDirection::Back),
    )?;
    fx.keys("ctrl-shift-g")?;
    ensure!(fx.read(|root, _| root.side_panel()) == Some(SidePanel::Changes));
    Ok(())
}
