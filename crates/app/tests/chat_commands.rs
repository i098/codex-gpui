#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use app::{Route, host};
use common::{CWD, EXPLAIN, Fx, GREETING, SUBTRACT, exchange, script};
use serde_json::{Value, json};
use std::sync::Arc;
use ui::dialog::Kind;

#[test]
fn proxy() {
    common::proxy_child();
}

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
        .context("command is visible")?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")?;
    ensure!(fx.read(|root, _| root.command_menu().is_none()));
    Ok(())
}

fn unread(fx: &Fx) -> Vec<String> {
    fx.read(|root, cx| root.sidebar().read(cx).unread.clone())
}

fn set_unread(fx: &mut Fx, ids: &[&str]) {
    fx.root.update(&mut fx.cx, |root, cx| {
        root.sidebar().update(cx, |sidebar, cx| {
            for id in ids {
                sidebar.set_unread(id, cx);
            }
        });
    });
}

fn with_reopened_chat(id: &str) -> Result<Fx> {
    let mut lines = script(None, |_| {})?;
    let mut read = exchange(
        "thread/read",
        json!({"thread": common::recorded_thread(id)?}),
    );
    read[0]["msg"]["params"] = json!({"threadId": id, "includeTurns": true});
    lines.extend(read);
    Fx::scripted(lines)
}

#[test]
fn unread_command_is_idempotent_and_opening_the_chat_reads_it() -> Result<()> {
    let mut fx = with_reopened_chat(SUBTRACT)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    command(&mut fx, "Mark as unread")?;
    fx.keys("cmd-shift-u")?;
    ensure!(unread(&fx) == [SUBTRACT]);
    ensure!(fx.route() == Route::Thread(SUBTRACT.into()) && fx.composer_focused()?);
    fx.proof_shot("accelerator-unread.png")?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.keys("alt-cmd-a")?;
    fx.wait_thread(SUBTRACT)?;
    ensure!(unread(&fx).is_empty());
    fx.proof_shot("reader-read.png")?;
    fx.keys("cmd-k")?;
    fx.proof_shot("menu-groups-thread.png")?;
    fx.keys("escape")?;
    fx.keys("cmd-n cmd-shift-u alt-cmd-r alt-cmd-p")?;
    ensure!(fx.route() == Route::Home && fx.read(|root, _| root.dialog().is_none()));
    ensure!(unread(&fx).is_empty());
    Ok(())
}

#[test]
fn attention_prefers_waiting_then_recency_and_excludes_the_open_chat() -> Result<()> {
    let mut fx = with_reopened_chat(EXPLAIN)?;
    set_unread(&mut fx, &[GREETING, SUBTRACT]);
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(bridge::Event::Request {
            id: json!(987),
            method: "item/commandExecution/requestApproval".into(),
            params: json!({"threadId": EXPLAIN, "turnId":"waiting-turn", "itemId":"waiting-item", "command":"echo fixture", "cwd": CWD}),
        }, cx);
    });
    command(&mut fx, "Next chat needing attention")?;
    fx.wait_thread(EXPLAIN)?;
    fx.keys("alt-cmd-a")?;
    fx.wait_thread(GREETING)?;
    fx.keys("alt-cmd-a")?;
    fx.wait_thread(EXPLAIN)?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            bridge::Event::Notification {
                method: "serverRequest/resolved".into(),
                params: json!({"threadId":EXPLAIN,"requestId":987}),
            },
            cx,
        );
    });
    fx.keys("alt-cmd-a")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("alt-cmd-a")?;
    ensure!(fx.route() == Route::Thread(SUBTRACT.into()));
    Ok(())
}

#[test]
fn pin_shortcut_reuses_section_moves_and_restores_a_refused_move() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/section/move", json!({})));
    for _ in 0..2 {
        let mut error = exchange("thread/section/move", Value::Null);
        error[1]["msg"].as_object_mut().unwrap().remove("result");
        error[1]["msg"]["error"] = json!({"code":-32000,"message":"fixture refusal"});
        lines.extend(error);
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    command(&mut fx, "Toggle pin")?;
    fx.wait("the pin write", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .filter(|r| r["method"] == "thread/section/move")
            .count()
            == 1
    })?;
    fx.wait("the pinned row leaves Recents", |fx| {
        fx.read(|root, cx| root.sidebar().read(cx).recents_shown() == 2)
    })?;
    fx.keys("alt-cmd-p")?;
    fx.wait("the refused unpin write", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .filter(|r| r["method"] == "thread/section/move")
            .count()
            == 2
    })?;
    fx.wait("the refused unpin restores the row", |fx| {
        fx.read(|root, cx| root.sidebar().read(cx).recents_shown() == 2)
    })?;
    fx.keys("alt-cmd-p")?;
    fx.wait("the restored pin state", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .filter(|r| r["method"] == "thread/section/move")
            .count()
            == 3
    })?;
    let moves: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == "thread/section/move")
        .map(|r| r["params"].clone())
        .collect();
    ensure!(
        moves
            == [
                json!({"threadId":SUBTRACT,"sectionId":ui::screens::sidebar::PINNED_SECTION_ID,"beforeThreadId":null}),
                json!({"threadId":SUBTRACT,"sectionId":null,"beforeThreadId":null}),
                json!({"threadId":SUBTRACT,"sectionId":null,"beforeThreadId":null}),
            ]
    );
    Ok(())
}

#[test]
fn clear_unreads_confirms_cancels_and_remembers_the_checkbox() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("shift-escape")?;
    ensure!(
        fx.read(
            |root, cx| root.dialog().unwrap().read(cx).focused_control() == "Don’t ask me again"
        )
    );
    fx.proof_shot("native-clear-unreads.png")?;
    set_unread(&mut fx, &[GREETING, SUBTRACT]);
    fx.keys("tab enter")?;
    ensure!(unread(&fx) == [GREETING, SUBTRACT] && fx.composer_focused()?);
    command(&mut fx, "Clear all unreads")?;
    fx.keys("space")?;
    fx.proof_shot("native-clear-unreads-checked.png")?;
    fx.keys("tab tab enter")?;
    fx.wait("all chats are read", |fx| unread(fx).is_empty())?;
    ensure!(fx.read(|root, _| root.dialog().is_none()));
    set_unread(&mut fx, &[EXPLAIN]);
    fx.keys("shift-escape")?;
    fx.wait("the saved checkbox skips confirmation", |fx| {
        unread(fx).is_empty()
    })?;
    ensure!(fx.read(|root, _| root.dialog().is_none()));
    Ok(())
}

struct FailingHost;
impl host::Host for FailingHost {
    fn fetch(&self, _: &host::Request) -> Result<Value> {
        anyhow::bail!("fixture storage is read-only")
    }
}

#[test]
fn clear_unreads_keeps_indicators_after_storage_failure() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(Arc::new(FailingHost), cx)
    });
    set_unread(&mut fx, &[GREETING]);
    fx.keys("shift-escape tab tab enter")?;
    fx.wait("the read failure is visible", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|toast| toast == "Failed to mark chats as read")
        })
    })?;
    ensure!(unread(&fx) == [GREETING] && fx.composer_focused()?);
    Ok(())
}

#[test]
fn hidden_copy_accelerators_use_the_active_chat_and_do_not_overwrite_on_home() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    let cwd = common::recorded_thread(SUBTRACT)?["cwd"]
        .as_str()
        .unwrap()
        .to_owned();
    for keys in ["cmd-shift-c", "alt-cmd-shift-c"] {
        fx.keys(keys)?;
        ensure!(
            fx.cx
                .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
                == Some(cwd.clone())
        );
    }
    fx.keys("alt-cmd-l")?;
    let link = format!("codex://threads/{SUBTRACT}");
    ensure!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
            == Some(link.clone())
    );
    fx.keys("cmd-n cmd-shift-c alt-cmd-l alt-cmd-shift-c")?;
    ensure!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
            == Some(link)
    );
    Ok(())
}

#[test]
fn rename_accelerator_keeps_the_chat_and_restores_composer_focus() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("alt-cmd-r")?;
    ensure!(fx.read(|root, cx| root.dialog().unwrap().read(cx).kind() == Kind::Rename));
    fx.proof_shot("accelerator-rename.png")?;
    fx.keys("escape")?;
    ensure!(fx.route() == Route::Thread(SUBTRACT.into()) && fx.composer_focused()?);
    Ok(())
}
