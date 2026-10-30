#![cfg(target_os = "macos")]
//! S01: plan mode. `/plan-mode` turns it on and off, the next `turn/start` names the `plan`
//! collaboration mode, a resumed chat keeps the mode `thread/resume` reports, and the plan
//! notifications (`item/plan/delta`, `turn/plan/updated`) reach the transcript.

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use bridge::{Event, types::ThreadItem};
use common::{Fx, GREETING, exchange, script};
use serde_json::{Value, json};

#[test]
fn proxy() {
    common::proxy_child();
}

fn captured(lines: Vec<Value>) -> Result<Fx> {
    Fx::captured("dark", (1280., 832.), lines)
}

fn open_chat(fx: &mut Fx) -> Result<()> {
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.click(fx.at("thread", "Work with Codex")?)?;
    ensure!(fx.composer_focused()?, "the composer must have focus");
    Ok(())
}

fn plan_mode(fx: &Fx) -> bool {
    fx.read(|root, cx| root.composer(cx).read(cx).plan_mode())
}

/// The slash menu's selected row as `title\ndescription`.
fn plan_row(fx: &Fx) -> Result<String> {
    let (rows, selected) = fx
        .read(|root, cx| root.composer(cx).read(cx).slash_state())
        .context("slash menu must be open")?;
    Ok(rows[selected.context("no selected row")?].clone())
}

/// Saves the window for the PR review (`CGX_PLAN_SHOTS` names the directory).
fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(dir) = std::env::var_os("CGX_PLAN_SHOTS") {
        fx.shot()?
            .save(std::path::Path::new(&dir).join(format!("{name}.png")))?;
    }
    Ok(())
}

fn notify(fx: &mut Fx, method: &str, params: Value) {
    let event = Event::Notification {
        method: method.into(),
        params,
    };
    fx.root
        .update(&mut fx.cx, |root, cx| root.on_event(event, cx));
    fx.cx.run_until_parked();
}

#[test]
fn plan_mode_command_sends_the_next_turn_in_plan_mode() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    let turn = json!({"id": "plan-turn", "status": "inProgress", "items": []});
    lines.extend(exchange("turn/start", json!({ "turn": turn })));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    shot(&mut fx, "plan-off")?;
    fx.type_text("/plan")?;
    ensure!(plan_row(&fx)? == "Plan mode\nTurn plan mode on");
    fx.keys("enter")?;
    ensure!(plan_mode(&fx), "Enter turns plan mode on");
    shot(&mut fx, "plan-on")?;
    ensure!(fx.composer_text()?.is_empty() && fx.composer_focused()?);

    fx.type_text("Make a plan")?;
    fx.keys("enter")?;
    fx.wait("turn/start", |fx| {
        fx.sent()
            .is_ok_and(|sent| sent.iter().any(|r| r["method"] == "turn/start"))
    })?;
    let sent = fx.sent()?;
    let start = sent
        .iter()
        .find(|r| r["method"] == "turn/start")
        .context("turn/start")?;
    let mode = &start["params"]["collaborationMode"];
    ensure!(mode["mode"] == "plan", "{mode}");
    ensure!(
        mode["settings"]["model"] == start["params"]["model"]
            && mode["settings"]["reasoning_effort"] == start["params"]["effort"]
            && mode["settings"]["developer_instructions"].is_null(),
        "{mode}"
    );
    ensure!(plan_mode(&fx), "plan mode stays on for the next turns");

    fx.type_text("/plan")?;
    ensure!(plan_row(&fx)? == "Plan mode\nTurn plan mode off");
    fx.keys("enter")?;
    ensure!(!plan_mode(&fx), "a second Enter turns plan mode off");
    Ok(())
}

fn plan_resume(line: &mut Value) {
    if line.pointer("/msg/result/thread/id") == Some(&json!(GREETING))
        && let Some(mode) = line.pointer_mut("/msg/result/collaborationMode/mode")
    {
        *mode = json!("plan");
    }
}

#[test]
fn a_resumed_chat_keeps_its_plan_mode() -> Result<()> {
    let mut fx = captured(script(None, plan_resume)?)?;
    ensure!(!plan_mode(&fx));
    open_chat(&mut fx)?;
    fx.wait("plan mode restored", |fx| plan_mode(fx))?;
    Ok(())
}

#[test]
fn plan_notifications_reach_the_transcript() -> Result<()> {
    let mut fx = captured(script(None, |_| {})?)?;
    open_chat(&mut fx)?;
    let turn = json!({"id": "plan-turn", "status": "inProgress", "items": []});
    notify(
        &mut fx,
        "turn/started",
        json!({"threadId": GREETING, "turn": turn}),
    );
    notify(
        &mut fx,
        "item/started",
        json!({"threadId": GREETING, "turnId": "plan-turn",
            "item": {"type": "plan", "id": "plan-item", "text": ""}}),
    );
    for delta in ["# Plan\n", "1. Read the code"] {
        notify(
            &mut fx,
            "item/plan/delta",
            json!({"threadId": GREETING, "turnId": "plan-turn", "itemId": "plan-item", "delta": delta}),
        );
    }
    notify(
        &mut fx,
        "turn/plan/updated",
        json!({"threadId": GREETING, "turnId": "plan-turn", "explanation": null, "plan": [
            {"step": "Read the code", "status": "completed"},
            {"step": "Write the plan", "status": "inProgress"},
            {"step": "Review", "status": "pending"},
        ]}),
    );
    let text = fx.read(|root, cx| {
        let thread = root.thread().unwrap().read(cx);
        thread.turns().last().and_then(|turn| {
            turn.items.iter().find_map(|item| match item {
                ThreadItem::Plan { text, .. } => Some(text.clone()),
                _ => None,
            })
        })
    });
    ensure!(
        text.as_deref() == Some("# Plan\n1. Read the code"),
        "{text:?}"
    );
    shot(&mut fx, "plan-events")?;
    Ok(())
}

#[test]
fn a_finished_plan_opens_in_its_own_window() -> Result<()> {
    let mut fx = captured(script(None, |_| {})?)?;
    open_chat(&mut fx)?;
    let plan = "# Plan\n\n1. Read the code\n2. Write the change";
    let item = json!({"type": "plan", "id": "plan-item", "text": plan});
    let turn = json!({"id": "plan-turn", "status": "inProgress", "items": []});
    notify(&mut fx, "turn/started", json!({"threadId": GREETING, "turn": turn}));
    notify(
        &mut fx,
        "item/completed",
        json!({"threadId": GREETING, "turnId": "plan-turn", "item": item}),
    );
    let turn = json!({"id": "plan-turn", "status": "completed", "items": [item]});
    notify(&mut fx, "turn/completed", json!({"threadId": GREETING, "turn": turn}));
    shot(&mut fx, "plan-answer")?;
    fx.action(Box::new(ui::screens::thread::view::OpenPlanSummary(plan.into())))?;
    let window = fx
        .cx
        .update(|cx| app::windows::handles(app::windows::Kind::Page, cx))
        .into_iter()
        .next()
        .context("the plan opens in a page window")?;
    let summary = window
        .downcast::<ui::screens::thread::view::PlanSummary>()
        .context("the page window shows the plan")?;
    if let Some(dir) = std::env::var_os("CGX_PLAN_SHOTS") {
        fx.cx
            .capture_screenshot(summary.into())?
            .save(std::path::Path::new(&dir).join("plan-window.png"))?;
    }
    Ok(())
}
