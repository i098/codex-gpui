//! Queue submission and row actions through the real Root, bridge, and native input.
//! The S023b recording supplies the original running-turn and queue protocol contract.

mod common;

use anyhow::{Context as _, Result};
use bridge::{Event, types::TurnStatus};
use common::{Fx, ROOT, SUBTRACT, exchange, proxy_child, script};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead as _, Write as _},
    path::Path,
    sync::Arc,
};

#[test]
fn proxy() {
    if let (Ok(path), Ok(log)) = (
        std::env::var("CGX_PROXY_SCRIPT"),
        std::env::var("CGX_PROXY_LOG"),
    ) {
        let first: Value =
            serde_json::from_str(fs::read_to_string(&path).unwrap().lines().next().unwrap())
                .unwrap();
        if let Some(options) = first.get("queueActions") {
            queue_proxy(Path::new(&path), Path::new(&log), options.clone()).unwrap();
            std::process::exit(0);
        }
    }
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
            .map(|row| row.text.to_string())
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
    assert!(fx.composer_focused()?);
    // The refused message comes back to the composer, so the button names the mode again
    // (the original keeps the message as a failed row with Retry and shows Stop).
    assert_eq!(fx.composer_text()?, "more");
    assert_eq!(label(&mut fx), "Steer");
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
    // readback.jsonl holds an empty `thread/queue/list` per thread; these match the port's
    // params exactly and so win over the recording-order fallback.
    for data in [json!([]), json!([submission]), json!([])] {
        let mut list = exchange(
            "thread/queue/list",
            json!({"data": data, "nextCursor": null}),
        );
        list[0]["msg"]["params"] = json!({"threadId": SUBTRACT, "cursor": null});
        lines.extend(list);
    }
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
    Ok(())
}

fn queue_proxy(path: &Path, log: &Path, mut options: Value) -> Result<()> {
    let (bridge, events) = bridge::Bridge::replay(path)?;
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let emit = |out: &Mutex<std::io::Stdout>, msg: Value| {
        let mut out = out.lock();
        writeln!(out, "{msg}").unwrap();
        out.flush().unwrap();
    };
    let event_out = out.clone();
    std::thread::spawn(move || {
        while let Ok(Event::Notification { method, params }) = events.recv_blocking() {
            let mut out = event_out.lock();
            writeln!(out, "{}", json!({"method": method, "params": params})).unwrap();
            out.flush().unwrap();
        }
    });
    let mut log = fs::OpenOptions::new().append(true).open(log)?;
    let mut queues = std::collections::HashMap::<String, Vec<Value>>::new();
    let mut sequence = 0;
    let mut deferred = None;
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let method = request["method"].as_str().context("request method")?;
        let params = &request["params"];
        if method != "proxy/barrier" {
            writeln!(log, "{}", json!({"method": method, "params": params}))?;
        }
        let Some(id) = request.get("id") else {
            bridge.notify(method, params)?;
            continue;
        };
        let refusal = match method {
            "turn/steer" => "steerFailures",
            "thread/queue/delete" => "deleteFailures",
            "thread/queue/reorder" => "reorderFailures",
            _ => "",
        };
        let failures = options[refusal].as_u64().unwrap_or(0);
        if failures > 0 {
            options[refusal] = (failures - 1).into();
            emit(
                &out,
                json!({"id": id, "error": {"code": -32000, "message": "isolated queue refusal"}}),
            );
            continue;
        }
        let queue = queues
            .entry(params["threadId"].as_str().unwrap_or_default().to_owned())
            .or_default();
        let result = match method {
            "proxy/barrier" => json!({}),
            "thread/queue/list" => {
                let start = params["cursor"]
                    .as_str()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                let end =
                    (start + options["pageSize"].as_u64().unwrap_or(100) as usize).min(queue.len());
                json!({"data": queue[start..end], "nextCursor": (end < queue.len()).then(|| end.to_string())})
            }
            "thread/queue/add" => {
                let row = queue
                    .iter()
                    .find(|row| row["clientUserMessageId"] == params["clientUserMessageId"])
                    .cloned();
                let row = row.unwrap_or_else(|| {
                    sequence += 1;
                    let row = json!({"id": format!("queue-{sequence}"), "input": params["input"], "clientUserMessageId": params["clientUserMessageId"]});
                    queue.push(row.clone());
                    row
                });
                json!({"queuedSubmission": row})
            }
            "thread/queue/delete" => {
                let at = queue
                    .iter()
                    .position(|row| row["id"] == params["queuedSubmissionId"]);
                if let Some(at) = at {
                    queue.remove(at);
                }
                json!({"deleted": at.is_some()})
            }
            "thread/queue/reorder" => {
                let ordered: Vec<_> = params["queuedSubmissionIds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| queue.iter().find(|row| &row["id"] == id).unwrap().clone())
                    .collect();
                assert_eq!(ordered.len(), queue.len());
                *queue = ordered;
                json!({})
            }
            "turn/steer" => {
                assert!(queue.iter().any(|row| row["clientUserMessageId"]
                    == params["clientUserMessageId"]
                    && row["input"] == params["input"]));
                assert_eq!(params["expectedTurnId"], "s023b-turn");
                json!({"turnId": "s023b-turn"})
            }
            "thread/queue/start" => {
                let at = queue
                    .iter()
                    .position(|row| row["id"] == params["queuedSubmissionId"])
                    .unwrap();
                queue.remove(at);
                json!({"turn": recording()?["steps"][1]["params"]["turn"]})
            }
            _ => match pollster::block_on(bridge.request::<_, Value>(method, params)) {
                Ok(result) => result,
                Err(error) => {
                    emit(
                        &out,
                        json!({"id": id, "error": {"code": -32000, "message": error.to_string()}}),
                    );
                    continue;
                }
            },
        };
        if method == "turn/steer" && options["holdSteer"] == true {
            deferred = Some(json!({"id": id, "result": result}));
            continue;
        }
        if method == "proxy/barrier"
            && let Some(reply) = deferred.take()
        {
            emit(&out, reply);
        }
        emit(&out, json!({"id": id, "result": result}));
        if method.starts_with("thread/queue/") && method != "thread/queue/list" {
            emit(
                &out,
                json!({"method": "thread/queue/changed", "params": {"threadId": params["threadId"]}}),
            );
        }
    }
    Ok(())
}

const MESSAGES: [&str; 3] = ["first queued", "middle 🦀 queued", "last queued"];

fn queue_fixture(options: Value) -> Result<Fx> {
    let mut lines = script(None, queue_config)?;
    lines[0]["queueActions"] = options;
    let oracle: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/tools/route1/oracle.json"))?)?;
    let resume = oracle
        .as_object()
        .unwrap()
        .iter()
        .find(|(key, _)| key.starts_with("mcp local thread/resume "))
        .context("original resume")?;
    lines.extend(exchange(
        "thread/resume",
        resume.1["message"]["result"].clone(),
    ));
    let list_id = lines
        .iter()
        .find(|line| line["msg"]["method"] == "thread/list")
        .unwrap()["msg"]["id"]
        .clone();
    let list = lines
        .iter()
        .find(|line| line["dir"] == "recv" && line["msg"]["id"] == list_id)
        .unwrap()["msg"]["result"]
        .clone();
    lines.extend(exchange("thread/list", list));
    let read = lines
        .iter()
        .find(|line| {
            line["msg"]["method"] == "thread/read" && line["msg"]["params"]["threadId"] == SUBTRACT
        })
        .unwrap();
    let read_id = read["msg"]["id"].clone();
    let params = read["msg"]["params"].clone();
    let result = lines
        .iter()
        .find(|line| line["dir"] == "recv" && line["msg"]["id"] == read_id)
        .unwrap()["msg"]["result"]
        .clone();
    let mut reopen = exchange("thread/read", result);
    reopen[0]["msg"]["params"] = params;
    lines.extend(reopen);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.shot()?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    notify(
        &mut fx,
        "turn/started",
        recording()?["steps"][1]["params"].clone(),
    );
    for (index, text) in MESSAGES.iter().enumerate() {
        fx.type_text(text)?;
        fx.keys("enter")?;
        fx.wait("server-owned queued row", |fx| {
            queued(fx) == MESSAGES[..=index]
        })?;
    }
    Ok(fx)
}

fn click_queue(fx: &mut Fx, action: &'static str) -> Result<(f32, f32)> {
    fx.shot()?;
    let [x, y, width, height] =
        bbox(&recording()?["queue"]["result"]["submit"]["tray"]["row"]["bbox"])?;
    let right_offset = if action.starts_with("queued-steer") {
        60.
    } else {
        22.
    };
    let at = (
        x + width - right_offset,
        y + height / 2. - (queued(fx).len() as f32 - 2.) * 33.,
    );
    fx.click(at)?;
    Ok(at)
}

fn save_queue_frame(fx: &mut Fx, name: &str) -> Result<()> {
    fx.reference_shot()?
        .save(Path::new(env!("CARGO_TARGET_TMPDIR")).join(name))?;
    Ok(())
}

#[test]
fn queued_row_steer_sends_once_and_keeps_the_other_rows() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = queue_fixture(json!({"pageSize": 1, "holdSteer": true}))?;
    let before = fx.reference_shot()?;
    before.save(Path::new(env!("CARGO_TARGET_TMPDIR")).join("queue-actions-ready.png"))?;
    let input = sent(&mut fx, "thread/queue/add")?[1]["params"].clone();
    let at = click_queue(&mut fx, "queued-steer-queue-2")?;
    fx.click(at)?;
    fx.sent()?;
    fx.wait("only the accepted row removed", |fx| {
        queued(fx) == [MESSAGES[0], MESSAGES[2]]
    })?;
    let steer = sent(&mut fx, "turn/steer")?;
    assert_eq!(steer.len(), 1);
    assert_eq!(
        steer[0]["params"],
        json!({
            "threadId": SUBTRACT, "expectedTurnId": "s023b-turn",
            "input": input["input"], "clientUserMessageId": input["clientUserMessageId"],
        })
    );
    assert_eq!(
        sent(&mut fx, "thread/queue/delete")?[0]["params"]["queuedSubmissionId"],
        "queue-2"
    );
    assert!(sent(&mut fx, "turn/start")?.is_empty());
    let after = fx.reference_shot()?;
    let [x, y, _, height] = bbox(&recording()?["queue"]["result"]["submit"]["tray"]["textBbox"])?;
    assert_eq!(
        common::changed(&before, &after, [x, y, 120., height]),
        0,
        "the remaining queued message keeps its rendered text"
    );
    after.save(Path::new(env!("CARGO_TARGET_TMPDIR")).join("queue-actions-steered.png"))?;
    Ok(())
}

#[test]
fn delete_undo_restores_order_and_location_before_editor_history() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = queue_fixture(json!({}))?;
    fx.type_text("draft")?;
    let before = fx.reference_shot()?;
    click_queue(&mut fx, "queued-delete-queue-2")?;
    fx.wait("deleted row", |fx| queued(fx) == [MESSAGES[0], MESSAGES[2]])?;
    save_queue_frame(&mut fx, "queue-actions-deleted.png")?;
    fx.keys("cmd-1")?;
    fx.wait_thread(common::GREETING)?;
    save_queue_frame(&mut fx, "queue-actions-away.png")?;
    fx.keys("cmd-z")?;
    fx.wait("restored row and location", |fx| {
        fx.thread_id().as_deref() == Some(SUBTRACT) && queued(fx) == MESSAGES
    })?;
    assert!(fx.read(|root, cx| {
        root.toasts(cx)
            .iter()
            .any(|text| text.as_ref() == "Queued message restored")
    }));
    assert_eq!(fx.composer_text()?, "draft");
    let adds = sent(&mut fx, "thread/queue/add")?;
    assert_eq!(adds.len(), 4);
    assert_eq!(adds[3]["params"], adds[1]["params"]);
    assert_eq!(
        sent(&mut fx, "thread/queue/reorder")?[0]["params"]["queuedSubmissionIds"],
        json!(["queue-1", "queue-4", "queue-3"])
    );
    let after = fx.reference_shot()?;
    after.save(Path::new(env!("CARGO_TARGET_TMPDIR")).join("queue-actions-restored.png"))?;
    let [x, y, _, height] = bbox(&recording()?["queue"]["result"]["submit"]["tray"]["textBbox"])?;
    let text_box = [x, y, 120., height];
    assert_eq!(
        common::changed(&before, &after, text_box),
        0,
        "the remaining queued message keeps its rendered text after undo"
    );
    fx.keys("cmd-z")?;
    assert_eq!(fx.composer_text()?, "");
    assert_eq!(queued(&mut fx), MESSAGES);
    Ok(())
}

#[test]
fn a_failed_delete_never_resubmits_an_accepted_steer() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = queue_fixture(json!({"steerFailures": 1, "deleteFailures": 1}))?;
    click_queue(&mut fx, "queued-steer-queue-2")?;
    fx.wait("refused steer", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|t| t.as_ref() == "Error submitting message")
        })
    })?;
    assert_eq!(queued(&mut fx), MESSAGES);
    click_queue(&mut fx, "queued-steer-queue-2")?;
    fx.wait("accepted steer with refused deletion", |fx| {
        sent(fx, "thread/queue/delete").is_ok_and(|requests| requests.len() == 1)
    })?;
    fx.cx.run_until_parked();
    assert_eq!(queued(&mut fx), MESSAGES);
    click_queue(&mut fx, "queued-steer-queue-2")?;
    fx.wait("accepted row removed without another steer", |fx| {
        queued(fx) == [MESSAGES[0], MESSAGES[2]]
    })?;
    assert_eq!(sent(&mut fx, "turn/steer")?.len(), 2);
    assert_eq!(sent(&mut fx, "thread/queue/delete")?.len(), 2);
    Ok(())
}

#[test]
fn undo_retries_ordering_without_adding_the_message_twice() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = queue_fixture(json!({"reorderFailures": 1}))?;
    click_queue(&mut fx, "queued-delete-queue-2")?;
    fx.wait("deleted row", |fx| queued(fx) == [MESSAGES[0], MESSAGES[2]])?;
    fx.keys("cmd-z")?;
    fx.wait("restore ordering refused", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|t| t.as_ref() == "Failed to restore queued message")
        })
    })?;
    fx.keys("cmd-z")?;
    fx.wait("restored order", |fx| queued(fx) == MESSAGES)?;
    assert_eq!(sent(&mut fx, "thread/queue/add")?.len(), 4);
    assert_eq!(sent(&mut fx, "thread/queue/reorder")?.len(), 2);
    Ok(())
}

#[test]
fn queued_row_steer_starts_the_same_submission_when_idle() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = queue_fixture(json!({}))?;
    notify(
        &mut fx,
        "turn/completed",
        json!({
            "threadId": SUBTRACT,
            "turn": {"id": "s023b-turn", "items": [], "status": "completed", "error": null},
        }),
    );
    fx.wait("idle composer", |fx| label(fx) == "Send")?;
    click_queue(&mut fx, "queued-steer-queue-2")?;
    fx.wait("started row removed", |fx| {
        queued(fx) == [MESSAGES[0], MESSAGES[2]]
    })?;
    assert_eq!(
        sent(&mut fx, "thread/queue/start")?[0]["params"],
        json!({
            "threadId": SUBTRACT, "queuedSubmissionId": "queue-2",
        })
    );
    assert!(sent(&mut fx, "turn/steer")?.is_empty());
    assert!(sent(&mut fx, "turn/start")?.is_empty());
    Ok(())
}
