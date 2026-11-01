//! A queue left by an interrupted turn shows `Queue paused because you interrupted` with
//! `Resume`, and a send asks whether to clear it first, as the original's queued-message list
//! and `pausedQueueSubmit` dialog do. The app-server holds the queue after `turn/interrupt`
//! until `thread/queue/start` or a new turn (checked against codex-cli 0.158.0).

mod common;

use anyhow::{Context as _, Result, ensure};
use bridge::types::{QueuedSubmission, TurnStatus};
use common::{Fx, GREETING, ROOT, exchange, proxy_child, script};
use serde_json::{Value, json};
use std::fs;
use ui::{composer::QueueAction, screens::settings::AppServer};

#[test]
fn proxy() {
    proxy_child();
}

fn notify(fx: &mut Fx, method: &'static str) {
    fx.cx.update(|cx| {
        let bridge = cx.global::<AppServer>().0.clone();
        cx.spawn(async move |_| {
            bridge.request::<_, Value>(method, json!({})).await.unwrap();
        })
        .detach();
    });
}

/// Opens the greeting chat with an interrupted last turn and one queued row.
fn paused() -> Result<Fx> {
    let recording: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s023.json"))?)?;
    let mut lines = script(None, |_| {})?;
    let list_id = &lines
        .iter()
        .find(|line| line["msg"]["method"] == "thread/list")
        .context("recorded thread list request")?["msg"]["id"];
    let list = lines
        .iter()
        .find(|line| line["dir"] == "recv" && &line["msg"]["id"] == list_id)
        .context("recorded thread list response")?["msg"]["result"]
        .clone();
    lines.extend(exchange("thread/list", list));
    for (method, step) in [("s023/start", 1), ("s023/complete", 3)] {
        let notification = &recording["steps"][step]["message"];
        lines.extend(exchange(method, json!({})));
        lines.push(json!({"dir":"recv", "msg":{
            "method":notification["method"], "params":notification["params"]
        }}));
    }
    lines.extend(exchange("thread/queue/start", json!({})));
    lines.extend(exchange("thread/queue/delete", json!({"deleted": true})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.click(fx.at("home", "Change the greeting")?)?;
    fx.wait_thread(GREETING)?;
    notify(&mut fx, "s023/start");
    fx.wait("running turn", |fx| {
        fx.read(|root, cx| root.thread().unwrap().read(cx).is_running())
    })?;
    notify(&mut fx, "s023/complete");
    fx.wait("interrupted turn", |fx| {
        fx.read(|root, cx| {
            root.thread()
                .unwrap()
                .read(cx)
                .turns()
                .last()
                .unwrap()
                .status
                == TurnStatus::Interrupted
        })
    })?;
    let row: QueuedSubmission = serde_json::from_value(json!({
        "id": "queued-1", "clientUserMessageId": "client-1",
        "input": [{"type": "text", "text": "Run the tests", "text_elements": []}],
    }))?;
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, _, cx| {
        composer.update(cx, |c, cx| c.set_queued(vec![row], cx));
    })?;
    fx.wait("paused queue", |fx| {
        fx.read(|root, cx| root.composer(cx).read(cx).queue_paused())
    })?;
    Ok(fx)
}

fn sent(fx: &mut Fx, method: &str) -> Result<Vec<Value>> {
    Ok(fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == method)
        .map(|r| r["params"].clone())
        .collect())
}

#[test]
fn resume_starts_the_paused_queue() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = paused()?;
    fx.capture_evidence("CGX_QUEUE_PAUSED_EVIDENCE", "queue-paused")?;
    // The banner's `Resume` button emits this action.
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, _, cx| {
        composer.update(cx, |_, cx| cx.emit(QueueAction::Resume));
    })?;
    fx.wait("queue start", |fx| {
        sent(fx, "thread/queue/start").is_ok_and(|s| !s.is_empty())
    })?;
    let thread = fx.read(|root, cx| root.thread().unwrap().read(cx).id().to_owned());
    ensure!(sent(&mut fx, "thread/queue/start")? == vec![json!({"threadId": thread})]);
    Ok(())
}

#[test]
fn send_while_paused_asks_to_clear_the_queue() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = paused()?;
    fx.type_text("New request")?;
    fx.keys("enter")?;
    fx.capture_evidence("CGX_QUEUE_PAUSED_EVIDENCE", "send-dialog")?;
    ensure!(
        sent(&mut fx, "turn/start")?.is_empty(),
        "the send did not wait for the paused-queue dialog"
    );
    fx.keys("shift-tab enter")?;
    fx.wait("queue delete", |fx| {
        sent(fx, "thread/queue/delete").is_ok_and(|s| !s.is_empty())
    })?;
    ensure!(sent(&mut fx, "thread/queue/delete")?[0]["queuedSubmissionId"] == "queued-1");
    fx.wait("new turn", |fx| {
        sent(fx, "turn/start").is_ok_and(|s| !s.is_empty())
    })?;
    Ok(())
}
