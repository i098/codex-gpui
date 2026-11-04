//! PANEL-22: a thread's `collabAgentToolCall` items list its subagents in Thread summary,
//! and the `Subagents` row opens the `Subagents` side panel tab, as the original's
//! `local-conversation-subagents-panel-tab`. The items follow the original bundle's schema;
//! no recording has them.
mod common;

use anyhow::{Context as _, Result, ensure};
use app::SidePanel;
use common::*;
use serde_json::json;
use std::path::Path;

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn summary_subagents_open_the_subagents_tab() -> Result<()> {
    if !cfg!(target_os = "macos") {
        println!("subagents_panel: the Metal replay requires macOS");
        return Ok(());
    }
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let turn = fx
        .read(|r, cx| Some(r.thread()?.read(cx).turns().last()?.id.clone()))
        .context("subtract turn")?;
    let items = [
        json!({"type": "collabAgentToolCall", "id": "collab-1", "tool": "spawnAgent",
            "status": "completed", "senderThreadId": SUBTRACT,
            "receiverThreadIds": ["agent-a", "agent-b"], "model": "gpt-5.1-codex",
            "prompt": "Add unit tests for subtract in tests/test_math.py",
            "agentsStates": {}}),
        json!({"type": "collabAgentToolCall", "id": "collab-2", "tool": "wait",
        "status": "completed", "senderThreadId": SUBTRACT,
        "receiverThreadIds": ["agent-a", "agent-b"],
        "agentsStates": {
            "agent-a": {"status": "completed",
                "message": "Added 3 tests; pytest passes."},
            "agent-b": {"status": "running", "message": null}
        }}),
    ];
    for item in items {
        fx.root.update(&mut fx.cx, |r, cx| {
            r.on_event(
                bridge::Event::Notification {
                    method: "item/completed".into(),
                    params: json!({"threadId": SUBTRACT, "turnId": turn, "item": item}),
                },
                cx,
            )
        });
    }
    fx.cx.run_until_parked();
    let proof = Path::new(env!("CARGO_TARGET_TMPDIR"));
    // `Toggle summary`, 63 px from the right edge.
    fx.click((fx.size.0 - 63., 22.))?;
    fx.shot()?.save(proof.join("subagents-summary.png"))?;
    fx.action(Box::new(ui::OpenSubagentsPanel))?;
    ensure!(
        fx.read(|r, _| r.side_panel()) == Some(SidePanel::Subagents),
        "side panel {:?}",
        fx.read(|r, _| r.side_panel())
    );
    fx.shot()?.save(proof.join("subagents-tab.png"))?;
    // The `Subagent 1` row opens its detail: prompt, model and last message.
    fx.click((743., 176.))?;
    fx.shot()?.save(proof.join("subagents-detail.png"))?;
    println!("subagents_panel: shots in {}", proof.display());
    Ok(())
}
