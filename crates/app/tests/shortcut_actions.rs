mod common;

use anyhow::{Result, ensure};
use app::GoToChat;
use common::{EXPLAIN, Fx, GREETING, SUBTRACT, exchange, script};
use serde_json::{Value, json};
use ui::screens::sidebar::{ArchiveThread, RenameThread};

fn rejected(method: &str) -> [Value; 2] {
    let [request, mut reply] = exchange(method, Value::Null);
    reply["msg"].as_object_mut().unwrap().remove("result");
    reply["msg"]["error"] = json!({"code": -32603, "message": "write refused"});
    [request, reply]
}

#[test]
fn failed_undo_keeps_the_edit_available_and_redo_reapplies_it() -> Result<()> {
    let mut recording = script(None, |_| {})?;
    recording.extend(exchange("thread/name/set", json!({})));
    recording.extend(rejected("thread/name/set"));
    recording.extend(exchange("thread/name/set", json!({})));
    recording.extend(exchange("thread/name/set", json!({})));
    let mut fx = Fx::scripted(recording)?;
    fx.action(Box::new(GoToChat(0)))?;
    fx.wait_thread(GREETING)?;
    let before = fx.read(|root, cx| root.thread().unwrap().read(cx).header_title());
    fx.action(Box::new(RenameThread(GREETING.into())))?;
    fx.type_text("Renamed fixture")?;
    fx.keys("enter")?;
    fx.wait("renamed header", |fx| {
        fx.read(|root, cx| root.thread().unwrap().read(cx).header_title() == "Renamed fixture")
    })?;
    fx.keys("cmd-, cmd-z")?;
    fx.wait("failed undo", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|toast| toast.as_ref() == "Could not undo action")
        })
    })?;
    ensure!(
        fx.read(|root, cx| root.thread().unwrap().read(cx).header_title() == "Renamed fixture")
    );
    fx.keys("cmd-z")?;
    fx.wait("retried undo", |fx| {
        fx.read(|root, cx| root.thread().unwrap().read(cx).header_title() == before)
    })?;
    fx.keys("cmd-shift-z")?;
    fx.wait("redone rename", |fx| {
        fx.read(|root, cx| root.thread().unwrap().read(cx).header_title() == "Renamed fixture")
    })?;
    Ok(())
}

#[test]
fn archive_undo_restores_the_exact_row_and_redo_removes_it() -> Result<()> {
    let mut recording = script(None, |_| {})?;
    recording.extend(exchange("thread/archive", json!({})));
    recording.extend(exchange(
        "thread/unarchive",
        json!({"thread": common::recorded_thread(GREETING)?}),
    ));
    recording.extend(exchange("thread/archive", json!({})));
    let mut fx = Fx::scripted(recording)?;
    fx.action(Box::new(ArchiveThread(GREETING.into())))?;
    fx.wait("archived row", |fx| {
        fx.read(|root, _| root.recents() == [SUBTRACT, EXPLAIN])
    })?;
    fx.keys("cmd-, cmd-z")?;
    fx.wait("restored row", |fx| {
        fx.read(|root, _| root.recents() == [GREETING, SUBTRACT, EXPLAIN])
    })?;
    fx.keys("cmd-shift-z")?;
    fx.wait("redone archive", |fx| {
        fx.read(|root, _| root.recents() == [SUBTRACT, EXPLAIN])
    })?;
    Ok(())
}

fn clear_command(fx: &mut Fx) -> Result<()> {
    fx.keys("cmd-k")?;
    fx.type_text("clear all unreads")?;
    fx.keys("enter")
}

#[test]
fn clear_unreads_requires_confirmation_and_remembers_the_opt_out() -> Result<()> {
    let mut fx = Fx::new()?;
    let sidebar = fx.read(|root, _| root.sidebar().clone());
    sidebar.update(&mut fx.cx, |sidebar, cx| sidebar.set_unread(GREETING, cx));
    clear_command(&mut fx)?;
    fx.keys("escape")?;
    ensure!(fx.read(|root, cx| root.sidebar().read(cx).unread == [GREETING]));
    clear_command(&mut fx)?;
    fx.keys("tab tab space shift-tab enter")?;
    fx.wait("confirmed unread clearing", |fx| {
        fx.read(|root, cx| root.sidebar().read(cx).unread.is_empty())
    })?;
    sidebar.update(&mut fx.cx, |sidebar, cx| sidebar.set_unread(SUBTRACT, cx));
    clear_command(&mut fx)?;
    fx.wait("saved confirmation preference", |fx| {
        fx.read(|root, cx| root.sidebar().read(cx).unread.is_empty())
    })?;
    Ok(())
}

#[test]
fn attention_shortcut_skips_the_open_unread_chat() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    let sidebar = fx.read(|root, _| root.sidebar().clone());
    sidebar.update(&mut fx.cx, |sidebar, cx| {
        sidebar.set_unread(GREETING, cx);
        sidebar.set_unread(SUBTRACT, cx);
    });
    fx.keys("alt-cmd-a")?;
    fx.wait_thread(SUBTRACT)?;
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn terminal_output_survives_bottom_panel_visibility_changes() -> Result<()> {
    common::install_panic_hook();
    let directory = tempfile::tempdir()?;
    let mut fx = Fx::new()?;
    fx.action(Box::new(ui::screens::sidebar::SelectProject(
        directory.path().to_string_lossy().into_owned(),
    )))?;
    fx.keys("cmd-j")?;
    fx.wait("opened terminal", |fx| {
        fx.read(|root, _| root.terminal_visible())
    })?;
    fx.type_text("printf 'shortcuts-%s\\n' 'pty-ready'")?;
    fx.keys("enter")?;
    let has_output = |fx: &Fx| {
        fx.read(|root, cx| {
            root.terminal_contents(cx)
                .is_some_and(|text| text.contains("shortcuts-pty-ready"))
        })
    };
    fx.wait("terminal process output", |fx| has_output(fx))?;
    fx.keys("cmd-j")?;
    ensure!(!fx.read(|root, _| root.terminal_visible()));
    fx.keys("ctrl-`")?;
    fx.wait("restored terminal output", |fx| {
        fx.read(|root, _| root.terminal_visible()) && has_output(fx)
    })?;
    fx.type_text("exit")?;
    fx.keys("enter")?;
    Ok(())
}
