#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use bridge::Event;
use common::{Fx, GREETING, GREETING_PROMPT, script};
use serde_json::{Value, json};

#[test]
fn proxy() {
    common::proxy_child();
}

fn emit(fx: &mut Fx, event: Event) {
    let root = fx.root.clone();
    fx.cx
        .update(|cx| root.update(cx, |root, cx| root.on_event(event, cx)));
    fx.cx.run_until_parked();
}

fn call(fx: &mut Fx, id: u64, arguments: Value) -> Result<Value> {
    emit(
        fx,
        Event::Request {
            id: json!(id),
            method: "item/tool/call".into(),
            params: json!({ "threadId": GREETING, "turnId": "turn-tool", "callId": format!("call-{id}"),
                "namespace": null, "tool": "read_thread_terminal", "arguments": arguments }),
        },
    );
    fx.sent()?
        .into_iter()
        .find(|message| message["reply"] == id)
        .map(|message| message["result"].clone())
        .with_context(|| format!("reply {id}"))
}

fn item(fx: &mut Fx, turn: &str, method: &str, status: &str) -> Vec<String> {
    emit(
        fx,
        Event::Notification {
            method: method.into(),
            params: json!({ "threadId": GREETING, "turnId": turn, "item": {
                "type": "dynamicToolCall", "id": "tool-item", "tool": "read_thread_terminal",
                "arguments": {}, "status": status, "contentItems": null, "success": null, "durationMs": null } }),
        },
    );
    fx.read(|root, cx| root.thread().unwrap().read(cx).activity_labels(turn, cx))
        .unwrap_or_default()
}

#[test]
fn model_reads_the_chat_terminal() -> Result<()> {
    common::install_panic_hook();
    std::fs::create_dir_all(common::CWD)?;
    let mut fx = Fx::captured(
        "dark",
        (1280., 832.),
        script(Some("sessions/greeting.jsonl"), |_| {})?,
    )?;
    fx.type_text(GREETING_PROMPT)?;
    fx.keys("enter")?;
    fx.wait_thread(GREETING)?;
    let start = fx
        .sent()?
        .into_iter()
        .find(|request| request["method"] == "thread/start")
        .context("thread/start")?;
    ensure!(
        start["params"]["dynamicTools"][0]["name"] == "read_thread_terminal",
        "{start}"
    );

    let none = call(&mut fx, 301, json!({}))?;
    ensure!(none["success"] == false, "{none}");
    ensure!(
        none["contentItems"][0]["text"]
            == "No app terminal session is attached to this thread yet."
    );

    fx.keys("cmd-j")?;
    fx.wait("opened terminal", |fx| {
        fx.read(|root, _| root.terminal_visible())
    })?;
    fx.type_text("printf 'chat-%s\\n' 'terminal-marker'")?;
    fx.keys("enter")?;
    fx.wait("terminal output", |fx| {
        fx.read(|root, cx| {
            root.terminal_contents(cx)
                .is_some_and(|text| text.contains("chat-terminal-marker"))
        })
    })?;
    let read = call(&mut fx, 302, json!({}))?;
    let text = read["contentItems"][0]["text"].as_str().unwrap_or_default();
    ensure!(read["success"] == true, "{read}");
    ensure!(
        text.starts_with("App terminal snapshot for this thread:\ncwd: "),
        "{text}"
    );
    ensure!(
        text.contains("\n```text\n") && text.ends_with("\n```"),
        "{text}"
    );
    ensure!(text.contains("chat-terminal-marker"), "{text}");
    let rejected = call(&mut fx, 303, json!({ "lines": 10 }))?;
    ensure!(rejected["contentItems"][0]["text"] == "read_thread_terminal takes no arguments.");

    let turn = fx
        .read(|root, cx| {
            root.thread()
                .unwrap()
                .read(cx)
                .turns()
                .last()
                .map(|t| t.id.clone())
        })
        .context("turn")?;
    fx.capture_evidence("CGX_TOOL_EVIDENCE", "before")?;
    let labels = item(&mut fx, &turn, "item/started", "inProgress");
    ensure!(
        labels.iter().any(|l| l == "Reading chat terminal"),
        "{labels:?}"
    );
    fx.capture_evidence("CGX_TOOL_EVIDENCE", "reading")?;
    let labels = item(&mut fx, &turn, "item/completed", "completed");
    ensure!(
        labels.iter().any(|l| l.contains("Read chat terminal")),
        "{labels:?}"
    );
    fx.capture_evidence("CGX_TOOL_EVIDENCE", "read")?;
    fx.type_text("exit")?;
    fx.keys("enter")?;
    Ok(())
}
