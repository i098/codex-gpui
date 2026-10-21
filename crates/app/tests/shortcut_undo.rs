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
