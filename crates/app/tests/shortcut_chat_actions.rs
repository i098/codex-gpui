mod common;

use anyhow::{Result, ensure};
use common::{Fx, GREETING, exchange, script};
use serde_json::json;
use ui::dialog::Kind;

#[test]
fn pin_rename_and_find_shortcuts_act_on_the_open_chat() -> Result<()> {
    let mut recording = script(None, |_| {})?;
    recording.extend(exchange("thread/section/move", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), recording)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;

    fx.keys("cmd-f")?;
    ensure!(fx.read(|root, cx| root.thread().unwrap().read(cx).find_state().is_some()));
    fx.keys("escape")?;

    fx.keys("alt-cmd-p")?;
    fx.wait("pin request", |fx| {
        fx.sent()
            .map(|requests| {
                requests.iter().any(|r| {
                    r["method"] == "thread/section/move" && r["params"]["threadId"] == GREETING
                })
            })
            .unwrap_or(false)
    })?;

    fx.keys("alt-cmd-r")?;
    ensure!(fx.read(|root, cx| {
        root.dialog()
            .is_some_and(|d| d.read(cx).kind() == Kind::Rename)
    }));
    Ok(())
}

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn next_tab_takes_its_keys_only_with_focus_in_a_side_tab() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(common::SUBTRACT)?;
    fx.keys("alt-cmd-b")?;
    for mut event in ui::screens::terminal::running_command_replay() {
        if let bridge::Event::Notification { params, .. } = &mut event
            && params["turn"].is_object()
        {
            params["turn"]["rootTurnId"] = params["turn"]["id"].clone();
        }
        fx.root
            .update(&mut fx.cx, |root, cx| root.on_event(event, cx));
        fx.cx.run_until_parked();
    }
    fx.action(Box::new(ui::OpenBackgroundTerminal))?;
    let tabs = fx.read(|root, _| root.side_tabs().to_vec());
    ensure!(tabs.len() == 2, "tabs {tabs:?}");
    fx.keys("alt-cmd-b")?;
    ensure!(
        fx.read(|root, _| root.side_panel() == Some(app::SidePanel::Changes)),
        "changes"
    );
    fx.focus_composer()?;
    fx.keys("ctrl-tab")?;
    ensure!(
        fx.read(|root, _| root.side_panel() == Some(app::SidePanel::Changes)),
        "outside a side tab ctrl-tab stays with the chats"
    );
    fx.click((1000., 400.))?;
    fx.keys("ctrl-tab")?;
    ensure!(
        fx.read(|root, _| root.side_panel() == Some(app::SidePanel::Terminal)),
        "panel focus: next tab"
    );
    fx.keys("alt-cmd-b")?;
    fx.click((1000., 400.))?;
    fx.keys("ctrl-shift-tab")?;
    ensure!(
        fx.read(|root, _| root.side_panel() == Some(app::SidePanel::Terminal)),
        "previous tab wraps"
    );
    fx.keys("alt-cmd-b")?;
    fx.click((1000., 400.))?;
    fx.keys("alt-cmd-w")?;
    ensure!(
        fx.read(|root, _| root.side_tabs() == [app::SidePanel::Changes]),
        "close other tabs keeps the active tab"
    );
    fx.keys("cmd-w")?;
    ensure!(
        fx.read(|root, _| root.side_tabs().is_empty()),
        "close tab closes the active side tab"
    );
    Ok(())
}

#[test]
fn copy_deeplink_puts_the_open_chats_link_on_the_clipboard() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.keys("alt-cmd-l")?;
    let copied = fx
        .cx
        .update(|cx| cx.read_from_clipboard())
        .and_then(|item| item.text());
    ensure!(
        copied == Some(format!("codex://threads/{GREETING}")),
        "{copied:?}"
    );
    Ok(())
}

#[test]
fn cmd_w_closes_the_side_tab_first_and_a_custom_close_window_key_closes_the_window() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.keys("alt-cmd-b")?;
    fx.keys("cmd-w")?;
    ensure!(fx.read(|root, _| root.side_tabs().is_empty() && root.dialog().is_none()));
    fx.keys("alt-cmd-b")?;
    fx.cx.update(|cx| {
        let dispatch = cx
            .global::<ui::screens::settings::keyboard_shortcuts::KeyboardState>()
            .dispatch
            .clone()
            .unwrap();
        dispatch(
            ui::screens::settings::keyboard_shortcuts::Change::Bindings {
                command: "closeWindow".into(),
                keys: vec!["CmdOrCtrl+Shift+W".into()],
            },
            cx,
        );
    });
    fx.keys("cmd-shift-w")?;
    ensure!(
        fx.read(|root, _| root.dialog().is_some() && root.side_tabs().len() == 1),
        "a closeWindow-only accelerator closes the window, not the tab"
    );
    Ok(())
}

fn started(fx: &mut Fx) -> Result<Vec<String>> {
    Ok(fx
        .sent()?
        .iter()
        .filter_map(|request| request["method"].as_str())
        .filter(|method| matches!(*method, "thread/start" | "turn/start"))
        .map(str::to_owned)
        .collect())
}

#[test]
fn submit_in_background_starts_the_chat_and_keeps_home_and_enter_still_opens_it() -> Result<()> {
    let session = || script(Some("sessions/greeting.jsonl"), |_| {});
    let mut fx = Fx::captured("dark", (1280., 832.), session()?)?;
    fx.type_text(common::GREETING_PROMPT)?;
    fx.keys("cmd-enter")?;
    fx.wait("background start", |fx| {
        started(fx).is_ok_and(|methods| methods == ["thread/start", "turn/start"])
    })?;
    ensure!(
        fx.route() == app::Route::Home,
        "background must not open the chat"
    );
    ensure!(
        fx.composer_text()?.is_empty(),
        "the draft clears as in a foreground send"
    );
    ensure!(fx.composer_focused()?, "focus stays in the Home composer");

    let mut fx = Fx::captured("dark", (1280., 832.), session()?)?;
    fx.type_text(common::GREETING_PROMPT)?;
    fx.keys("enter")?;
    fx.wait_thread(GREETING)?;
    Ok(())
}

#[test]
fn submit_in_background_is_not_available_in_an_open_chat() -> Result<()> {
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.type_text("draft")?;
    let before = fx.sent()?.len();
    fx.keys("cmd-enter")?;
    fx.advance(std::time::Duration::from_millis(300))?;
    ensure!(fx.sent()?.len() == before && fx.composer_text()? == "draft");
    fx.focus_composer()?;
    Ok(())
}

#[test]
fn the_dialog_lists_send_in_background_on_home_only() -> Result<()> {
    let mut fx = Fx::new()?;
    let rows = |fx: &mut Fx| {
        fx.keys("cmd-/")?;
        let titles = fx
            .cx
            .update(|cx| ui::screens::settings::keyboard_shortcuts::active_shortcuts(cx));
        fx.keys("cmd-/")?;
        anyhow::Ok(
            titles
                .iter()
                .any(|row| row.1.id == "composer.submitInBackground"),
        )
    };
    ensure!(rows(&mut fx)?, "Home lists it");
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    ensure!(!rows(&mut fx)?, "an open chat does not");
    Ok(())
}
