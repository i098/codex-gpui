//! Pending approval requests: each waits under its own request id and thread, the open
//! thread's oldest shows, a decision settles exactly that request once, and requests the
//! server withdraws (`serverRequest/resolved`, `turn/completed`) leave unanswered.
#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{CWD, EXPLAIN, Fx, SUBTRACT, recorded_thread, script};
use serde_json::{Value, json};

#[test]
fn proxy() {
    common::proxy_child();
}

/// A request crafted from `CommandApprovalRequest` / `FileChangeApprovalRequest`: the
/// recorded sessions ran with `approvalPolicy: never`. `reason` names the request.
fn request(id: u64, kind: &str, thread: &str, turn: &str) -> Value {
    let reason = format!("request {id}");
    let (method, params) = match kind {
        "command" => (
            "item/commandExecution/requestApproval",
            json!({ "threadId": thread, "turnId": turn, "itemId": format!("call-{id}"),
                    "command": "python3 hello.py", "cwd": CWD, "reason": reason }),
        ),
        _ => (
            "item/fileChange/requestApproval",
            json!({ "threadId": thread, "turnId": turn, "itemId": format!("patch-{id}"),
                    "reason": reason }),
        ),
    };
    json!({ "dir": "recv", "msg": { "id": id, "method": method, "params": params } })
}

fn reply(id: u64, decision: &str) -> Value {
    json!({ "reply": id, "result": { "decision": decision } })
}

fn shown(fx: &Fx) -> Option<String> {
    let composer = fx.composer();
    fx.read(|_, cx| composer.read(cx).approval().map(|a| format!("{a:?}")))
}

fn wait_shown(fx: &mut Fx, id: u64) -> Result<()> {
    let name = format!("\"request {id}\"");
    fx.wait(&format!("the card of request {id}"), |fx| {
        shown(fx).is_some_and(|card| card.contains(&name))
    })
}

fn wait_sent(fx: &mut Fx, answer: &Value) -> Result<()> {
    fx.wait(&format!("the answer {answer}"), |fx| {
        fx.sent().unwrap().contains(answer)
    })
}

#[test]
fn approvals_queue_per_request_and_thread() -> Result<()> {
    let subtract = recorded_thread(SUBTRACT)?;
    let last = subtract["turns"]
        .as_array()
        .and_then(|turns| turns.last())
        .context("recorded turn")?;
    let turn = last["id"].as_str().context("turn id")?;
    let mut lines = script(None, |_| {})?;
    let read = lines
        .iter()
        .position(|v| v["dir"] == "recv" && v["msg"]["result"]["thread"]["id"] == SUBTRACT)
        .context("no thread/read reply")?;
    // Four requests on the open chat (1 and 2 of an earlier turn, 3 and 4 of its last turn)
    // and one on another chat, all pending at once.
    let pending = [
        request(1, "command", SUBTRACT, "turn-earlier"),
        request(2, "command", SUBTRACT, "turn-earlier"),
        request(5, "command", EXPLAIN, "turn-other"),
        request(3, "fileChange", SUBTRACT, turn),
        request(4, "command", SUBTRACT, turn),
    ];
    lines.splice(read + 1..read + 1, pending);
    // The server withdraws request 2 after the answer to 1, and ends the last turn (with
    // request 4 pending) after the answer to 3.
    let resolved = json!({ "threadId": SUBTRACT, "requestId": 2 });
    let completed = json!({ "threadId": SUBTRACT, "turn": last });
    lines.extend([
        json!({ "dir": "send", "msg": { "id": 1, "result": { "decision": "decline" } } }),
        json!({ "dir": "recv", "msg": { "method": "serverRequest/resolved", "params": resolved } }),
        json!({ "dir": "send", "msg": { "id": 3, "result": { "decision": "accept" } } }),
        json!({ "dir": "recv", "msg": { "method": "turn/completed", "params": completed } }),
        json!({ "dir": "send", "msg": { "id": 5, "result": { "decision": "decline" } } }),
    ]);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;

    wait_shown(&mut fx, 1)?;
    fx.keys("escape")?;
    wait_sent(&mut fx, &reply(1, "decline"))?;
    // The server withdrew request 2: the next card is 3, not the other chat's 5.
    wait_shown(&mut fx, 3)?;
    fx.keys("enter")?;
    wait_sent(&mut fx, &reply(3, "accept"))?;
    // The turn of request 4 ended; request 5 belongs to the other chat.
    fx.wait("no card on the chat", |fx| shown(fx).is_none())?;

    fx.click(fx.at("home", "In two sentences")?)?;
    fx.wait_thread(EXPLAIN)?;
    wait_shown(&mut fx, 5)?;
    fx.keys("escape")?;
    wait_sent(&mut fx, &reply(5, "decline"))?;
    ensure!(shown(&fx).is_none(), "a card stayed after the last answer");

    let answers: Vec<Value> = fx
        .sent()?
        .into_iter()
        .filter(|m| m.get("reply").is_some())
        .collect();
    let want = [reply(1, "decline"), reply(3, "accept"), reply(5, "decline")];
    ensure!(answers == want, "answers {answers:?}, want {want:?}");
    Ok(())
}
