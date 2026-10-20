#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use bridge::Event;
use common::{EXPLAIN, Fx, SUBTRACT, script};
use gpui::Focusable as _;
use serde_json::{Value, json};

fn exit_on_panic() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
}

#[test]
fn proxy() {
    exit_on_panic();
    common::proxy_child();
}

fn emit(fx: &mut Fx, event: Event) {
    let root = fx.root.clone();
    fx.cx
        .update(|cx| root.update(cx, |root, cx| root.on_event(event, cx)));
    fx.cx.run_until_parked();
}

fn last_turn(thread_id: &str) -> Result<String> {
    let thread = common::recorded_thread(thread_id)?;
    thread["turns"]
        .as_array()
        .and_then(|turns| turns.last())
        .and_then(|turn| turn["id"].as_str())
        .context("recorded request turn")
        .map(str::to_owned)
}

fn permission(id: u64, thread: &str, turn: &str) -> Event {
    Event::Request {
        id: json!(id),
        method: "item/permissions/requestApproval".into(),
        params: json!({ "threadId": thread, "turnId": turn, "itemId": format!("permission-{id}"),
            "environmentId": null, "startedAtMs": 1_792_083_600_000_u64, "cwd": common::CWD,
            "reason": format!("request {id}"), "permissions": { "network": { "enabled": true }, "fileSystem": null } }),
    }
}

#[test]
fn withdrawn_answer_cannot_settle_the_next_request_or_another_thread() -> Result<()> {
    exit_on_panic();
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let subtract_turn = last_turn(SUBTRACT)?;
    let explain_turn = last_turn(EXPLAIN)?;
    emit(&mut fx, permission(101, SUBTRACT, &subtract_turn));
    emit(&mut fx, permission(102, SUBTRACT, &subtract_turn));
    emit(&mut fx, permission(104, SUBTRACT, &subtract_turn));
    emit(&mut fx, permission(103, EXPLAIN, &explain_turn));
    fx.capture_evidence("CGX_REQUEST_EVIDENCE", "permissions")?;
    fx.focus_composer()?;
    let root = fx.root.clone();
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        composer
            .focus_handle(cx)
            .dispatch_action(&ui::composer::Submit, window, cx);
        root.update(cx, |root, cx| {
            root.on_event(
                Event::Notification {
                    method: "serverRequest/resolved".into(),
                    params: json!({ "threadId": SUBTRACT, "requestId": 101 }),
                },
                cx,
            )
        });
    })?;
    fx.cx.run_until_parked();
    ensure!(
        fx.sent()?
            .iter()
            .all(|message| message.get("reply").is_none()),
        "the withdrawn card sent an answer"
    );
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| {
            root.on_event(
                Event::Notification {
                    method: "serverRequest/resolved".into(),
                    params: json!({ "threadId": SUBTRACT, "requestId": 102 }),
                },
                cx,
            )
        });
        composer
            .focus_handle(cx)
            .dispatch_action(&ui::composer::Submit, window, cx);
    })?;
    fx.cx.run_until_parked();
    ensure!(
        fx.sent()?
            .iter()
            .all(|message| message.get("reply").is_none()),
        "the stale rendered card answered its replacement"
    );
    fx.advance(std::time::Duration::ZERO)?;
    fx.keys("tab enter")?;
    fx.advance(std::time::Duration::ZERO)?;
    ensure!(
        fx.sent()?.iter().all(|message| message["reply"] != 104),
        "Opening the approval scope answered the request"
    );
    fx.keys("enter")?;
    fx.wait("permission answer", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .any(|message| message["reply"] == 104)
    })?;
    fx.keys("enter")?;
    fx.click(fx.at("home", "In two sentences")?)?;
    fx.wait_thread(EXPLAIN)?;
    fx.keys("escape")?;
    fx.wait("other thread denial", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .any(|message| message["reply"] == 103)
    })?;
    let replies: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|message| message.get("reply").is_some())
        .collect();
    ensure!(
        replies
            == vec![
                json!({ "reply": 104, "result": { "permissions": { "network": { "enabled": true } }, "scope": "session" } }),
                json!({ "reply": 103, "result": { "permissions": {}, "scope": "turn" } }),
            ],
        "wrong request answers: {replies:?}"
    );
    Ok(())
}

#[test]
fn native_forms_send_typed_answers_and_cancel_without_accepting() -> Result<()> {
    exit_on_panic();
    let mut lines = script(None, |_| {})?;
    for thread in [SUBTRACT, EXPLAIN] {
        let request = lines
            .iter()
            .find(|line| {
                line["dir"] == "send"
                    && line["msg"]["method"] == "thread/read"
                    && line["msg"]["params"]["threadId"] == thread
            })
            .cloned()
            .context("recorded thread read")?;
        let response = lines
            .iter()
            .find(|line| {
                line["dir"] == "recv"
                    && line["msg"]["id"] == request["msg"]["id"]
                    && line["msg"].get("result").is_some()
            })
            .cloned()
            .context("recorded thread read result")?;
        lines.extend([request, response]);
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let turn = last_turn(SUBTRACT)?;
    emit(
        &mut fx,
        Event::Request {
            id: json!(201),
            method: "item/tool/requestUserInput".into(),
            params: json!({
                "threadId": SUBTRACT, "turnId": turn, "itemId": "question-201", "isBlocking": true, "autoResolutionMs": null,
                "questions": [{ "id": "destination", "header": "Destination", "question": "Where should the report go?",
                    "isOther": false, "isSecret": false, "options": null }]
            }),
        },
    );
    fx.capture_evidence("CGX_REQUEST_EVIDENCE", "user-input")?;
    fx.focus_composer()?;
    fx.keys("tab tab")?;
    fx.type_text("Review folder")?;
    fx.click(fx.at("home", "In two sentences")?)?;
    fx.wait_thread(EXPLAIN)?;
    emit(
        &mut fx,
        Event::Request {
            id: json!(206),
            method: "item/tool/requestUserInput".into(),
            params: json!({ "threadId": EXPLAIN, "turnId": last_turn(EXPLAIN)?, "itemId": "other-206", "isBlocking": true, "autoResolutionMs": null,
            "questions": [{ "id": "other-chat", "header": "Other chat", "question": "What should this chat use?",
                "isOther": false, "isSecret": false, "options": null }] }),
        },
    );
    fx.focus_composer()?;
    fx.keys("tab tab")?;
    fx.type_text("Other answer")?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    fx.advance(std::time::Duration::ZERO)?;
    fx.focus_composer()?;
    fx.keys("enter")?;
    fx.wait("user input answer", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .any(|message| message["reply"] == 201)
    })?;
    ensure!(fx.sent()?.contains(&json!({ "reply": 201, "result": { "answers": { "destination": { "answers": ["Review folder"] } } } })), "the text answer lost its question id");

    emit(
        &mut fx,
        Event::Request {
            id: json!(202),
            method: "mcpServer/elicitation/request".into(),
            params: json!({
                "threadId": SUBTRACT, "turnId": null, "serverName": "release-fixture", "mode": "form", "_meta": null, "message": "Confirm the report settings",
                "requestedSchema": { "type": "object", "required": ["copies", "publish"], "properties": {
                    "copies": { "type": "integer", "title": "Copies", "minimum": 1, "maximum": 5, "default": 2 },
                    "publish": { "type": "boolean", "title": "Publish", "default": false }
                } }
            }),
        },
    );
    fx.capture_evidence("CGX_REQUEST_EVIDENCE", "elicitation")?;
    for keys in [
        "up up up up up up up up up",
        "down down down down down down down down down",
        "up up up",
    ] {
        fx.keys(keys)?;
    }
    fx.click((1143., 724.))?;
    fx.keys("down down up")?;
    fx.focus_composer()?;
    fx.keys("cmd-enter")?;
    ensure!(
        fx.sent()?.iter().all(|message| message["reply"] != 202),
        "Next answered before the final field"
    );
    fx.capture_evidence("CGX_REQUEST_EVIDENCE", "elicitation-final")?;
    fx.keys("cmd-enter")?;
    fx.wait("typed elicitation answer", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .any(|message| message["reply"] == 202)
    })?;
    let replies = fx.sent()?;
    ensure!(replies.contains(&json!({ "reply": 202, "result": { "action": "accept", "content": { "copies": 4, "publish": false }, "_meta": null } })), "number steps did not preserve types or honor both bounds: {replies:?}");

    emit(
        &mut fx,
        Event::Request {
            id: json!(207),
            method: "mcpServer/elicitation/request".into(),
            params: json!({
                "threadId": SUBTRACT, "turnId": null, "serverName": "release-fixture", "mode": "form",
                "message": "Choose report options", "requestedSchema": {
                    "type": "object", "required": ["name"], "properties": {
                        "name": { "type": "string", "default": "Report" },
                        "zColour": { "type": "string", "enum": ["Red", "Blue"] }
                    }
                }
            }),
        },
    );
    fx.focus_composer()?;
    fx.keys("cmd-enter")?;
    fx.keys("tab tab tab tab tab enter")?;
    fx.wait("optional final choice skipped", |fx| {
        fx.sent().unwrap().contains(&json!({
            "reply": 207, "result": { "action": "accept", "content": { "name": "Report" }, "_meta": null }
        }))
    })?;

    emit(
        &mut fx,
        Event::Request {
            id: json!(203),
            method: "item/tool/call".into(),
            params: json!({
                "threadId": SUBTRACT, "turnId": turn, "callId": "picker-203", "namespace": null,
                "tool": "request_option_picker", "arguments": { "question": "Choose a report format", "options": [{ "label": "Markdown" }, { "label": "Plain text" }], "allowMultiple": false }
            }),
        },
    );
    fx.capture_evidence("CGX_REQUEST_EVIDENCE", "tool-picker")?;
    fx.focus_composer()?;
    fx.keys("escape")?;
    fx.wait("dynamic tool dismissal", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .any(|message| message["reply"] == 203)
    })?;
    let answer = fx
        .sent()?
        .into_iter()
        .find(|message| message["reply"] == 203)
        .unwrap();
    let content: Value = serde_json::from_str(
        answer["result"]["contentItems"][0]["text"]
            .as_str()
            .unwrap(),
    )?;
    ensure!(
        content == json!({ "action": "dismiss", "selectedOptions": [], "freeformAnswer": null }),
        "the dynamic tool accepted on Escape"
    );
    emit(
        &mut fx,
        Event::Request {
            id: json!(204),
            method: "item/tool/requestUserInput".into(),
            params: json!({ "threadId": SUBTRACT, "turnId": turn, "itemId": "secret-204", "isBlocking": true, "autoResolutionMs": null,
            "questions": [{ "id": "token", "header": "Token", "question": "Enter the fixture token",
                "isOther": false, "isSecret": true, "options": null }] }),
        },
    );
    fx.focus_composer()?;
    fx.keys("tab tab")?;
    fx.capture_evidence("CGX_REQUEST_EVIDENCE", "secret-input-empty")?;
    fx.type_text("fixture-secret")?;
    fx.cx.update(|cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("clipboard-marker".into()))
    });
    fx.keys("cmd-a cmd-c")?;
    ensure!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
            == Some("clipboard-marker".into()),
        "secret copy changed the clipboard"
    );
    fx.capture_evidence("CGX_REQUEST_EVIDENCE", "secret-input")?;
    fx.keys("cmd-x")?;
    ensure!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
            == Some("clipboard-marker".into()),
        "secret cut changed the clipboard"
    );
    fx.type_text("replacement-token")?;
    fx.keys("enter")?;
    fx.wait("secret answer", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .any(|message| message["reply"] == 204)
    })?;
    ensure!(fx.sent()?.contains(&json!({ "reply": 204, "result": { "answers": { "token": { "answers": ["replacement-token"] } } } })), "the secret edit lost its value");
    emit(
        &mut fx,
        Event::Request {
            id: json!(205),
            method: "item/tool/requestUserInput".into(),
            params: json!({ "threadId": SUBTRACT, "turnId": turn, "itemId": "skip-205", "isBlocking": true, "autoResolutionMs": null,
            "questions": [{ "id": "optional-answer", "header": "Answer", "question": "Provide an answer or skip",
                "isOther": false, "isSecret": false, "options": null }] }),
        },
    );
    fx.focus_composer()?;
    fx.keys("enter")?;
    fx.wait("skipped question answer", |fx| {
        fx.sent()
            .unwrap()
            .contains(&json!({ "reply": 205, "result": { "answers": {} } }))
    })?;
    ensure!(
        fx.sent()?
            .iter()
            .all(|message| message["method"] != "turn/interrupt"),
        "Skip interrupted the turn"
    );
    fx.click(fx.at("home", "In two sentences")?)?;
    fx.wait_thread(EXPLAIN)?;
    fx.advance(std::time::Duration::ZERO)?;
    fx.focus_composer()?;
    fx.keys("enter")?;
    fx.wait("other thread draft answer", |fx| {
        fx.sent().unwrap().contains(&json!({
            "reply": 206, "result": { "answers": { "other-chat": { "answers": ["Other answer"] } } }
        }))
    })?;
    let answers = fx.sent()?;
    for id in 201..=206 {
        ensure!(
            answers
                .iter()
                .filter(|message| message["reply"] == id)
                .count()
                == 1,
            "request {id} was not answered exactly once"
        );
    }
    Ok(())
}

#[test]
fn cancelling_input_retains_the_request_when_interrupt_fails() -> Result<()> {
    exit_on_panic();
    let mut lines = script(None, |_| {})?;
    let turn = last_turn(SUBTRACT)?;
    lines.extend([
        json!({ "dir": "send", "msg": { "id": 900, "method": "turn/interrupt", "params": { "threadId": SUBTRACT, "turnId": turn } } }),
        json!({ "dir": "recv", "msg": { "id": 900, "result": {} } }),
    ]);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    for id in [301, 302] {
        emit(
            &mut fx,
            Event::Request {
                id: json!(id),
                method: "item/tool/requestUserInput".into(),
                params: json!({ "threadId": SUBTRACT, "turnId": turn, "itemId": format!("question-{id}"), "isBlocking": true, "autoResolutionMs": null,
                "questions": [{ "id": "answer", "header": "Answer", "question": "Provide an answer",
                    "isOther": false, "isSecret": false, "options": null }] }),
            },
        );
    }
    fx.focus_composer()?;
    fx.keys("escape")?;
    fx.wait("successful interruption reveals the next request", |fx| {
        let composer = fx.composer();
        fx.read(|_, cx| composer.read(cx).approval().is_some())
    })?;
    fx.advance(std::time::Duration::ZERO)?;
    fx.focus_composer()?;
    fx.keys("tab tab")?;
    fx.type_text("Retain this answer")?;
    fx.keys("escape")?;
    fx.wait("failed interruption restores the card", |fx| {
        let composer = fx.composer();
        fx.read(|_, cx| composer.read(cx).approval().is_some())
    })?;
    let sent = fx.sent()?;
    ensure!(
        sent.iter()
            .filter(|message| message["method"] == "turn/interrupt")
            .count()
            == 2,
        "the interruption was not sent once per request"
    );
    ensure!(
        sent.iter().all(|message| message.get("reply").is_none()),
        "cancellation answered a question"
    );
    fx.advance(std::time::Duration::ZERO)?;
    fx.keys("enter")?;
    fx.wait("restored draft answer", |fx| {
        fx.sent().unwrap().contains(&json!({
        "reply": 302, "result": { "answers": { "answer": { "answers": ["Retain this answer"] } } }
    }))
    })?;
    emit(
        &mut fx,
        Event::Notification {
            method: "serverRequest/resolved".into(),
            params: json!({ "threadId": SUBTRACT, "requestId": 302 }),
        },
    );
    Ok(())
}

#[test]
fn standalone_requests_outlive_the_correlated_turn() -> Result<()> {
    exit_on_panic();
    let thread = common::recorded_thread(SUBTRACT)?;
    let turn = thread["turns"]
        .as_array()
        .and_then(|turns| turns.last())
        .context("recorded turn")?
        .clone();
    let turn_id = turn["id"].as_str().context("turn id")?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    emit(&mut fx, permission(401, SUBTRACT, turn_id));
    emit(
        &mut fx,
        Event::Request {
            id: json!(402),
            method: "mcpServer/elicitation/request".into(),
            params: json!({
                "threadId": SUBTRACT, "turnId": turn_id, "serverName": "fixture", "mode": "form", "_meta": null,
                "message": "Standalone request", "requestedSchema": { "type": "object", "properties": {} }
            }),
        },
    );
    emit(
        &mut fx,
        Event::Request {
            id: json!(403),
            method: "item/tool/requestUserInput".into(),
            params: json!({
                "threadId": SUBTRACT, "turnId": turn_id, "itemId": "async-403", "isBlocking": false, "autoResolutionMs": null,
                "questions": [{ "id": "answer", "header": "Answer", "question": "Answer after the turn",
                    "isOther": false, "isSecret": false, "options": null }]
            }),
        },
    );
    emit(
        &mut fx,
        Event::Notification {
            method: "turn/completed".into(),
            params: json!({ "threadId": SUBTRACT, "turn": turn }),
        },
    );
    fx.advance(std::time::Duration::ZERO)?;
    fx.focus_composer()?;
    fx.keys("escape")?;
    fx.wait("standalone elicitation cancellation", |fx| {
        fx.sent().unwrap().contains(&json!({
            "reply": 402, "result": { "action": "cancel", "content": null, "_meta": null }
        }))
    })?;
    fx.advance(std::time::Duration::ZERO)?;
    fx.focus_composer()?;
    fx.keys("escape")?;
    fx.wait("nonblocking question cancellation", |fx| {
        fx.sent().unwrap().contains(&json!({
            "reply": 403, "result": { "answers": {} }
        }))
    })?;
    let sent = fx.sent()?;
    ensure!(
        sent.iter()
            .all(|message| message["reply"] != 401 && message["method"] != "turn/interrupt"),
        "an expired permission was answered or a nonblocking request interrupted the turn"
    );
    Ok(())
}

#[test]
fn malformed_question_and_permission_requests_receive_denials() -> Result<()> {
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    for (id, method, params) in [
        (
            json!("malformed-question"),
            "item/tool/requestUserInput",
            json!({ "threadId": SUBTRACT, "questions": [] }),
        ),
        (
            json!(702),
            "item/permissions/requestApproval",
            json!({ "threadId": SUBTRACT, "permissions": null }),
        ),
    ] {
        emit(
            &mut fx,
            Event::Request {
                id,
                method: method.into(),
                params,
            },
        );
    }
    fx.wait("malformed request denials", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .filter(|message| message.get("reply").is_some())
            .count()
            == 2
    })?;
    let replies: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|message| message.get("reply").is_some())
        .collect();
    ensure!(
        replies
            == vec![
                json!({ "reply": "malformed-question", "result": { "answers": {} } }),
                json!({ "reply": 702, "result": { "permissions": {}, "scope": "turn" } }),
            ],
        "malformed request replies changed identity or granted permission: {replies:?}"
    );
    Ok(())
}
