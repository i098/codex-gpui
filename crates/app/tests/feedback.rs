#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use common::{Fx, GREETING, exchange, recorded_thread, script};
use serde_json::{Value, json};

#[test]
fn proxy() {
    common::proxy_child();
}

fn open_chat(lines: Vec<Value>) -> Result<Fx> {
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    keys(&mut fx, "cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.click(fx.at("thread", "Work with Codex")?)?;
    Ok(fx)
}

fn requests(fx: &mut Fx, method: &str) -> Result<Vec<Value>> {
    Ok(fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == method)
        .collect())
}

fn keys(fx: &mut Fx, sequence: &str) -> Result<()> {
    for key in sequence.split_whitespace() {
        fx.keys(key)?;
        fx.input(gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse(key)?,
        }))?;
    }
    Ok(())
}

#[test]
fn feedback_requires_confirmation_and_explicit_log_consent() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "feedback/upload",
        json!({"threadId": GREETING, "promptHash": null}),
    ));
    let mut fx = open_chat(lines)?;
    fx.type_text("/feedback")?;
    keys(&mut fx, "enter")?;
    ensure!(requests(&mut fx, "feedback/upload")?.is_empty());
    fx.type_text("The feedback button did not work")?;
    keys(&mut fx, "tab tab tab tab tab enter")?;
    fx.wait("feedback sent", |fx| {
        fx.read(|root, cx| root.toasts(cx).iter().any(|t| t == "Feedback sent"))
    })?;
    let sent = requests(&mut fx, "feedback/upload")?;
    ensure!(sent.len() == 1);
    ensure!(
        sent[0]["params"]
            == json!({"classification":"bug", "threadId": GREETING,
        "reason":"The feedback button did not work", "includeLogs":false})
    );
    ensure!(fx.composer_focused()?);
    Ok(())
}

#[test]
fn feedback_save_keeps_input_after_invalid_paths() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "feedback/save",
        json!({"savedPath":"/tmp/feedback.zip", "threadId": GREETING, "promptHash": null}),
    ));
    let mut fx = open_chat(lines)?;
    fx.type_text("/feedback")?;
    keys(&mut fx, "enter")?;
    fx.type_text("Keep my report")?;
    keys(&mut fx, "tab")?;
    fx.type_text("relative.zip")?;
    keys(&mut fx, "tab space tab tab enter")?;
    ensure!(requests(&mut fx, "feedback/save")?.is_empty());
    keys(&mut fx, "shift-tab shift-tab shift-tab cmd-a")?;
    fx.type_text("/tmp/feedback.zip")?;
    keys(&mut fx, "tab tab tab enter")?;
    fx.wait("feedback saved", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|t| t == "Feedback saved on app-server host: /tmp/feedback.zip")
        })
    })?;
    let sent = requests(&mut fx, "feedback/save")?;
    ensure!(sent.len() == 1);
    ensure!(
        sent[0]["params"]
            == json!({"classification":"bug", "threadId": GREETING,
        "reason":"Keep my report", "includeLogs":true, "savePath":"/tmp/feedback.zip"})
    );
    Ok(())
}

#[test]
fn recap_keeps_the_existing_conversation_and_draft() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "thread/resume",
        json!({"thread": recorded_thread(GREETING)?}),
    ));
    lines.extend(exchange(
        "turn/start",
        json!({"turn":{"id":"recap", "items":[], "status":"inProgress", "error":null}}),
    ));
    let mut fx = open_chat(lines)?;
    fx.type_text("draft /recap")?;
    keys(&mut fx, "enter")?;
    fx.wait("recap turn", |fx| {
        requests(fx, "turn/start").is_ok_and(|r| !r.is_empty())
    })?;
    let sent = requests(&mut fx, "turn/start")?;
    ensure!(sent[0]["params"]["threadId"] == GREETING);
    ensure!(
        sent[0]["params"]["input"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Summarize this conversation")
    );
    ensure!(requests(&mut fx, "thread/compact/start")?.is_empty());
    ensure!(fx.composer_text()? == "draft ");
    ensure!(fx.route() == app::Route::Thread(GREETING.into()));
    Ok(())
}

#[test]
fn recap_refuses_to_interrupt_a_running_turn() -> Result<()> {
    let lines = script(None, |line| {
        if line.pointer("/msg/result/thread/id") == Some(&json!(GREETING))
            && let Some(status) = line.pointer_mut("/msg/result/thread/turns/0/status")
        {
            *status = json!("inProgress");
        }
    })?;
    let mut fx = open_chat(lines)?;
    fx.type_text("/recap")?;
    keys(&mut fx, "enter")?;
    ensure!(fx.read(|root, cx| {
        root.toasts(cx)
            .iter()
            .any(|message| message == "Wait for the current turn before requesting a recap")
    }));
    ensure!(requests(&mut fx, "turn/start")?.is_empty());
    Ok(())
}

#[test]
fn export_reads_the_whole_thread_and_refuses_to_overwrite_a_file() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    for _ in 0..2 {
        lines.extend(exchange(
            "thread/read",
            json!({"thread":recorded_thread(GREETING)?}),
        ));
    }
    let mut fx = open_chat(lines)?;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("conversation.md");
    let mut original = None;
    for expected in [
        "Transcript exported",
        "Failed to export transcript; choose an unused file path",
    ] {
        fx.type_text("/export")?;
        keys(&mut fx, "enter")?;
        fx.cx.simulate_new_path_selection(|_| Some(path.clone()));
        fx.wait("export result", |fx| {
            fx.read(|root, cx| root.toasts(cx).iter().any(|t| t == expected))
        })?;
        let markdown = std::fs::read_to_string(&path)?;
        ensure!(markdown.contains("## User\n\n") && markdown.contains("## Assistant\n\n"));
        if let Some(original) = &original {
            ensure!(&markdown == original);
        } else {
            original = Some(markdown);
        }
    }
    let reads = requests(&mut fx, "thread/read")?;
    ensure!(
        reads
            .iter()
            .any(|r| r["params"] == json!({"threadId":GREETING, "includeTurns":true}))
    );
    Ok(())
}
