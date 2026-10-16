//! Replays CP-108 from the original renderer recording: a submit while a turn runs steers the
//! turn (`turn/steer`) or queues on the app-server (`thread/queue/add`), per the
//! `Follow-up behavior` setting.

mod common;

use anyhow::{Context as _, Result};
use bridge::{Event, types::TurnStatus};
use common::{Fx, ROOT, SUBTRACT, exchange, proxy_child, script};
use serde_json::{Value, json};
use std::{fs, path::Path};

#[test]
fn proxy() {
    proxy_child();
}

fn recording() -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(format!(
        "{ROOT}/refs/interactions/s023b.json"
    ))?)?)
}

/// `thread/queue/changed` and friends, as the app-server would send them.
fn notify(fx: &mut Fx, method: &str, params: Value) {
    let event = Event::Notification {
        method: method.into(),
        params,
    };
    fx.root
        .update(&mut fx.cx, |root, cx| root.on_event(event, cx));
    fx.cx.run_until_parked();
}

/// The recorded steps: open the subtract thread, hold a turn in progress, type `more`, Enter.
fn submit_while_running(fx: &mut Fx, steps: &[Value], mode: &str) -> Result<()> {
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    notify(fx, "turn/started", steps[1]["params"].clone());
    fx.wait("running turn", |fx| {
        fx.read(|root, cx| {
            root.thread()
                .unwrap()
                .read(cx)
                .turns()
                .last()
                .is_some_and(|turn| turn.status == TurnStatus::InProgress)
        })
    })?;
    assert_eq!(label(fx), "Stop");
    let [x, y, w, h] = bbox(&steps[2]["target"]["bbox"])?;
    fx.click((x + w / 2., y + h / 2.))?;
    fx.type_text(steps[3]["text"].as_str().context("typed text")?)?;
    assert_eq!(label(fx), mode);
    fx.keys("enter")?;
    Ok(())
}

fn label(fx: &mut Fx) -> &'static str {
    fx.read(|root, cx| root.composer(cx).read(cx).send_label())
}

fn queued(fx: &mut Fx) -> Vec<String> {
    fx.read(|root, cx| {
        root.composer(cx)
            .read(cx)
            .queued()
            .iter()
            .map(|s| s.to_string())
            .collect()
    })
}

fn bbox(v: &Value) -> Result<[f32; 4]> {
    let n = |k: &str| v[k].as_f64().map(|n| n as f32).context("bbox");
    Ok([n("x")?, n("y")?, n("w")?, n("h")?])
}

/// Thread id, input and the turn or message id the recorded request carries; the port leaves
/// out the original's page context and client metadata.
fn core(params: &Value) -> Value {
    let mut input = params["input"].clone();
    for part in input.as_array_mut().into_iter().flatten() {
        if let Some(text) = part["text"].as_str() {
            // The original's editor serialises the paragraph with a trailing newline.
            part["text"] = text.trim_end().into();
        }
    }
    json!({
        "threadId": params["threadId"],
        "input": input,
        "expectedTurnId": params["expectedTurnId"],
    })
}

fn sent(fx: &mut Fx, method: &str) -> Result<Vec<Value>> {
    Ok(fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == method)
        .collect())
}

#[test]
fn steer_sends_the_message_into_the_running_turn() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let rec = recording()?;
    let steps = rec["steps"].as_array().context("steps")?;
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/resume", json!({})));
    let mut steer = exchange("turn/steer", json!({}));
    steer[1]["msg"].as_object_mut().unwrap().remove("result");
    steer[1]["msg"]["error"] = json!({"code":-32000,"message":"route1: turn/steer is not allowed"});
    lines.extend(steer);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    submit_while_running(&mut fx, steps, "Steer")?;
    fx.wait("steer error toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|s| s.as_ref() == "Error submitting message")
        })
    })?;
    let want = &rec["result"]["submit"]["requests"][0];
    assert_eq!(want["method"], "turn/steer");
    let requests = sent(&mut fx, "turn/steer")?;
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(core(&requests[0]["params"]), core(&want["params"]));
    assert_eq!(label(&mut fx), rec["result"]["submit"]["button"]["aria"]);
    assert!(fx.composer_focused()?);
    // The refused message comes back to the composer (the original keeps it as a failed row).
    assert_eq!(fx.composer_text()?, "more");
    fx.shot()?
        .save(Path::new(env!("CARGO_TARGET_TMPDIR")).join("s023b-steer.png"))?;
    Ok(())
}

fn queue_config(line: &mut Value) {
    if let Some(desktop) = line.pointer_mut("/msg/result/config/desktop") {
        desktop["followUpQueueMode"] = "queue".into();
    }
}

#[test]
fn queue_adds_a_row_until_the_server_starts_it() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let rec = recording()?;
    let queue = &rec["queue"];
    let steps = queue["steps"].as_array().context("queue steps")?;
    let want = &queue["result"]["submit"]["requests"];
    let submission = json!({"id": "s023b-queued-1", "clientUserMessageId": "c", "input": want[0]["params"]["input"]});
    let mut lines = script(None, queue_config)?;
    lines.extend(exchange("thread/resume", json!({})));
    lines.extend(exchange(
        "thread/queue/add",
        json!({"queuedSubmission": submission}),
    ));
    lines.extend(exchange(
        "thread/queue/list",
        json!({"data": [submission], "nextCursor": null}),
    ));
    lines.extend(exchange(
        "thread/queue/list",
        json!({"data": [], "nextCursor": null}),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let rest = fx.shot()?;
    submit_while_running(&mut fx, steps, "Queue")?;
    fx.wait("queued row", |fx| queued(fx) == ["more"])?;
    assert_eq!(fx.composer_text()?, "");
    assert!(fx.composer_focused()?);
    assert_eq!(label(&mut fx), "Stop");
    let add = sent(&mut fx, "thread/queue/add")?;
    assert_eq!(add.len(), 1, "{add:?}");
    assert_eq!(core(&add[0]["params"]), core(&want[0]["params"]));
    assert_eq!(
        add[0]["params"]["clientUserMessageId"]
            .as_str()
            .map(str::len),
        Some(36)
    );
    let list = sent(&mut fx, "thread/queue/list")?;
    assert_eq!(list[0]["params"], want[1]["params"]);
    // The tray paints `bg-surface-elevated-secondary` where the original's does.
    let frame = fx.shot()?;
    let tray = &queue["result"]["submit"]["tray"]["container"];
    let [x, y, w, h] = bbox(&tray["bbox"])?;
    let changed = common::changed(&rest, &frame, [x, y, w, h]);
    assert!(changed > (w * h) as usize, "tray pixels changed: {changed}");
    let [tx, ty, _, th] = bbox(&queue["result"]["submit"]["tray"]["textBbox"])?;
    let px = frame
        .get_pixel(((x + 6.) * 2.) as u32, ((ty + th / 2.) * 2.) as u32)
        .0;
    assert_eq!(
        [px[0], px[1], px[2]],
        [45, 45, 45],
        "tray background at x+6"
    );
    assert!(tx > x, "text sits inside the tray");
    frame.save(Path::new(env!("CARGO_TARGET_TMPDIR")).join("s023b-queued.png"))?;
    // The turn ends: the client sends nothing (the app-server drains its queue).
    let before = fx.sent()?.len();
    notify(
        &mut fx,
        "turn/completed",
        json!({"threadId": SUBTRACT, "turn": {"id": "s023b-turn", "items": [], "status": "completed", "error": null}}),
    );
    assert_eq!(queued(&mut fx), ["more"]);
    let after: Vec<_> = fx.sent()?.split_off(before);
    assert!(
        after
            .iter()
            .all(|r| r["method"] != "turn/start" && r["method"] != "thread/queue/start"),
        "{after:?}"
    );
    // The server starts the queued submission: the queue changes and the row goes.
    notify(
        &mut fx,
        "thread/queue/changed",
        json!({"threadId": SUBTRACT}),
    );
    notify(
        &mut fx,
        "turn/started",
        json!({"threadId": SUBTRACT, "turn": {"id": "s023b-queued-turn", "items": [], "status": "inProgress", "error": null}}),
    );
    fx.wait("row gone", |fx| queued(fx).is_empty())?;
    assert_eq!(sent(&mut fx, "thread/queue/list")?.len(), 2);
    Ok(())
}
