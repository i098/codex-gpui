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
    fx.keys("ctrl-`")?;
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
