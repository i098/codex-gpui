use bridge::types::ThreadItem;
use serde_json::json;

#[test]
fn computer_use_results_retain_content_and_unknown_metadata() {
    let result = json!({
        "content": [{"type": "text", "text": "Saved the scratch marker"}],
        "structuredContent": {"marker": "scratch-only"},
        "_meta": {"x-codex-turn-metadata": {"session_id": "thread-a", "turn_id": "turn-a"}}
    });
    let item: ThreadItem = serde_json::from_value(json!({
        "type": "mcpToolCall", "id": "call-a", "server": "lcu", "tool": "js",
        "status": "completed", "arguments": {"title": "Write scratch marker", "code": "1+1"},
        "result": result, "durationMs": 24,
        "_meta": {"surface": "computer"}, "futureField": {"retain": true}
    })).unwrap();
    let ThreadItem::McpToolCall { arguments, result: actual, error, duration_ms, metadata, .. } = item else {
        panic!("MCP tool call was discarded");
    };
    assert_eq!(arguments["title"], "Write scratch marker");
    assert_eq!(actual, Some(result));
    assert_eq!(error, None);
    assert_eq!(duration_ms, Some(24));
    assert_eq!(metadata["_meta"]["surface"], "computer");
    assert_eq!(metadata["futureField"]["retain"], true);
}

#[test]
fn reset_failure_is_not_projected_as_a_success() {
    let item: ThreadItem = serde_json::from_value(json!({
        "type": "mcpToolCall", "id": "reset-a", "server": "lcu", "tool": "js_reset",
        "status": "failed", "error": {"message": "Reset failed", "code": "provider_unavailable"}
    })).unwrap();
    let ThreadItem::McpToolCall { tool, status, result, error, .. } = item else {
        panic!("reset item was discarded");
    };
    assert_eq!(tool, "js_reset");
    assert_eq!(status, "failed");
    assert_eq!(result, None);
    assert_eq!(error.unwrap()["code"], "provider_unavailable");
}

#[cfg(target_os = "macos")]
mod common;

#[cfg(target_os = "macos")]
#[test]
fn progress_survives_completion_without_crossing_turns() -> anyhow::Result<()> {
    use common::{Fx, SUBTRACT};
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let turn_id = fx.read(|root, cx| {
        root.thread().unwrap().read(cx).turns().last().unwrap().id.clone()
    });
    let notify = |fx: &mut Fx, method: &str, params: serde_json::Value| {
        fx.cx.update(|cx| fx.root.update(cx, |root, cx| {
            root.on_event(bridge::Event::Notification { method: method.into(), params }, cx);
        }));
    };
    let item = |status: &str| json!({
        "type": "mcpToolCall", "id": "call-a", "server": "lcu", "tool": "js",
        "arguments": {"title": "Write scratch marker"}, "status": status
    });
    let mut started = item("inProgress");
    started["_meta"] = json!({"surface": "computer"});
    notify(&mut fx, "item/started", json!({"threadId": SUBTRACT, "turnId": turn_id, "item": started}));
    notify(&mut fx, "item/mcpToolCall/progress", json!({
        "threadId": SUBTRACT, "turnId": "another-turn", "itemId": "call-a", "message": "Wrong turn"
    }));
    notify(&mut fx, "item/mcpToolCall/progress", json!({
        "threadId": SUBTRACT, "turnId": turn_id, "itemId": "call-a", "message": "Writing marker",
        "_meta": {"sequence": 1}
    }));
    let mut completed = item("completed");
    completed["result"] = json!({"content": [{"type": "text", "text": "Marker saved"}]});
    notify(&mut fx, "item/completed", json!({"threadId": SUBTRACT, "turnId": turn_id, "item": completed}));
    fx.read(|root, cx| {
        let view = root.thread().unwrap().read(cx);
        let items = &view.turns().last().unwrap().items;
        let item = items.iter().find(|item| matches!(item, ThreadItem::McpToolCall { id, .. } if id == "call-a")).unwrap();
        let ThreadItem::McpToolCall { progress, metadata, result, status, .. } = item else { unreachable!() };
        assert_eq!(status, "completed");
        assert_eq!(progress.len(), 1);
        assert_eq!(progress[0]["message"], "Writing marker");
        assert_eq!(progress[0]["_meta"]["sequence"], 1);
        assert_eq!(metadata["_meta"]["surface"], "computer");
        assert_eq!(result.as_ref().unwrap()["content"][0]["text"], "Marker saved");
    });
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn proxy() {
    common::proxy_child();
}

#[cfg(target_os = "macos")]
#[test]
fn stop_sends_one_interrupt_per_running_turn() -> anyhow::Result<()> {
    use common::{Fx, SUBTRACT, script};
    let mut lines = script(None, |_| {})?;
    for (id, turn) in [(201, "cu-stop"), (202, "cu-next")] {
        lines.extend([
            json!({"dir": "send", "msg": {"id": id, "method": "turn/interrupt", "params": {"threadId": SUBTRACT, "turnId": turn}}}),
            json!({"dir": "recv", "msg": {"id": id, "result": {}}}),
        ]);
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let turn = |fx: &mut Fx, id: &str, status: &str| {
        fx.cx.update(|cx| fx.root.update(cx, |root, cx| {
            root.on_event(bridge::Event::Notification {
                method: if status == "inProgress" { "turn/started" } else { "turn/completed" }.into(),
                params: json!({"threadId": SUBTRACT, "turn": {"id": id, "items": [], "status": status}}),
            }, cx);
        }));
        fx.cx.run_until_parked();
    };
    let stop = |fx: &mut Fx| -> anyhow::Result<()> {
        fx.cx.update_window(fx.window, |_, window, cx| {
            window.dispatch_action(Box::new(ui::composer::Stop), cx);
        })?;
        fx.cx.run_until_parked();
        Ok(())
    };
    turn(&mut fx, "cu-stop", "inProgress");
    stop(&mut fx)?;
    fx.wait("the first interrupt", |fx| {
        fx.sent().unwrap().iter().filter(|request| request["method"] == "turn/interrupt").count() == 1
    })?;
    stop(&mut fx)?;
    let requests = fx.sent()?;
    assert_eq!(requests.iter().filter(|request| request["method"] == "turn/interrupt").count(), 1);
    turn(&mut fx, "cu-stop", "interrupted");
    turn(&mut fx, "cu-next", "inProgress");
    stop(&mut fx)?;
    fx.wait("the next turn interrupt", |fx| {
        fx.sent().unwrap().iter().filter(|request| request["method"] == "turn/interrupt").count() == 2
    })?;
    let requests = fx.sent()?;
    assert!(!requests.iter().any(|request| request["method"] == "mcpServer/tool/call"));
    assert_eq!(requests.iter().filter(|request| request["method"] == "turn/interrupt").map(|request| request["params"]["turnId"].as_str().unwrap()).collect::<Vec<_>>(), vec!["cu-stop", "cu-next"]);
    Ok(())
}
