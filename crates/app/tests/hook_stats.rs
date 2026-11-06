mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, SUBTRACT};
use serde_json::{Value, json};

/// `hook/started` and `hook/completed` rows show inside the turn: the start row becomes the
/// completion row, its output shows under it, and another thread's hooks are ignored.
#[test]
fn hook_runs_show_in_turn_activity() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let turn = fx
        .read(|r, cx| Some(r.thread()?.read(cx).turns().last()?.id.clone()))
        .context("subtract turn")?;
    let notify = |fx: &mut Fx, method: &str, thread: &str, run: &Value| {
        let params = json!({"threadId": thread, "turnId": turn, "run": run});
        fx.root.update(&mut fx.cx, |r, cx| {
            r.on_event(
                bridge::Event::Notification {
                    method: method.into(),
                    params,
                },
                cx,
            )
        });
        fx.cx.run_until_parked();
    };
    let labels = |fx: &Fx| -> Vec<String> {
        fx.read(|r, cx| r.thread()?.read(cx).activity_labels(&turn, cx))
            .unwrap_or_default()
    };
    let shots = std::env::var("CGX_HOOK_SHOTS").ok();
    if let Some(out) = &shots {
        std::fs::create_dir_all(out)?;
        fx.click(fx.at("thread", "Worked for")?)?;
        fx.shot()?.save(format!("{out}/hook-rows-before.png"))?;
    }
    let mut started = run(
        "hook-1",
        "preToolUse",
        "project",
        "running",
        Some("Checking shell command"),
        json!([]),
    );
    started["completedAt"] = Value::Null;
    notify(&mut fx, "hook/started", SUBTRACT, &started);
    ensure!(
        labels(&fx).contains(&"Running PreToolUse hook".to_owned()),
        "{:?}",
        labels(&fx)
    );
    if let Some(out) = &shots {
        fx.shot()?.save(format!("{out}/hook-rows-running.png"))?;
    }
    let done = run(
        "hook-1",
        "preToolUse",
        "project",
        "completed",
        Some("Checked shell command"),
        json!([{"kind": "feedback", "text": "Command allowed."}]),
    );
    notify(&mut fx, "hook/completed", SUBTRACT, &done);
    let rows = labels(&fx);
    ensure!(rows.contains(&"Ran PreToolUse hook".to_owned()), "{rows:?}");
    ensure!(
        !rows.iter().any(|row| row.starts_with("Running PreToolUse")),
        "the start row stayed: {rows:?}"
    );
    ensure!(
        rows.contains(&"Checked shell command".to_owned()),
        "{rows:?}"
    );
    ensure!(rows.contains(&"Command allowed.".to_owned()), "{rows:?}");
    if let Some(out) = &shots {
        fx.shot()?.save(format!("{out}/hook-rows-done.png"))?;
    }
    // Another thread's hook run is ignored.
    let other = run(
        "hook-2",
        "postToolUse",
        "user",
        "completed",
        None,
        json!([]),
    );
    let before = rows;
    notify(&mut fx, "hook/completed", "thread-other", &other);
    ensure!(labels(&fx) == before, "{:?}", labels(&fx));
    Ok(())
}

#[test]
fn proxy() {
    common::proxy_child();
}

fn run(
    id: &str,
    event: &str,
    source: &str,
    status: &str,
    message: Option<&str>,
    entries: Value,
) -> Value {
    json!({"id": id, "eventName": event, "handlerType": "command", "executionMode": "sync",
        "scope": "turn", "sourcePath": "/Users/me/.codex/hooks.json", "source": source,
        "displayOrder": 0, "status": status, "statusMessage": message,
        "startedAt": 1761900000, "completedAt": 1761900001, "durationMs": 120, "entries": entries})
}

/// `hook/completed` runs of a finished turn add `Hook stats` to its actions; the dialog counts
/// them, and a blocked or failed run opens to its status message and output.
#[test]
fn hook_runs_open_hook_stats_dialog() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    let turn = fx
        .read(|r, cx| Some(r.thread()?.read(cx).turns().last()?.id.clone()))
        .context("subtract turn")?;
    let runs = [
        run(
            "hook-1",
            "userPromptSubmit",
            "user",
            "completed",
            None,
            json!([]),
        ),
        run(
            "hook-2",
            "preToolUse",
            "project",
            "blocked",
            Some("Checking shell command"),
            json!([{"kind": "feedback", "text": "rm -rf is not allowed in this repository."}]),
        ),
        run(
            "hook-3",
            "postToolUse",
            "user",
            "failed",
            None,
            json!([{"kind": "error", "text": "hook exited with code 1: ./hooks/lint.sh: line 3: ruff: command not found"}]),
        ),
        run("hook-4", "stop", "admin", "stopped", None, json!([])),
        run("hook-5", "stop", "plugin", "completed", None, json!([])),
    ];
    // A running run is not counted; its completion replaces it.
    let notify = |fx: &mut Fx, method: &str, run: &Value| {
        fx.root.update(&mut fx.cx, |r, cx| {
            r.on_event(
                bridge::Event::Notification {
                    method: method.into(),
                    params: json!({"threadId": SUBTRACT, "turnId": turn, "run": run}),
                },
                cx,
            )
        });
        fx.cx.run_until_parked();
    };
    let mut running = runs[0].clone();
    running["status"] = "running".into();
    notify(&mut fx, "hook/started", &running);
    for run in &runs {
        notify(&mut fx, "hook/completed", run);
    }
    let shots = std::env::var("CGX_HOOK_SHOTS").ok();
    if let Some(out) = &shots {
        std::fs::create_dir_all(out)?;
        fx.shot()?.save(format!("{out}/1-thread.png"))?;
    }
    let button = fx
        .cx
        .debug_bounds(fx.window, "hook-stats")?
        .context("Hook stats button")?
        .center();
    fx.click((button.x.into(), button.y.into()))?;
    let kind = fx.read(|r, cx| Some(r.dialog()?.read(cx).kind()));
    ensure!(kind == Some(ui::dialog::Kind::HookStats), "{kind:?}");
    ensure!(
        fx.read(|r, cx| r.dialog()?.read(cx).hook_runs_open()) == Some(vec![false; 5]),
        "runs not listed"
    );
    if let Some(out) = &shots {
        fx.hover((1270., 6.))?;
        fx.shot()?.save(format!("{out}/2-dialog.png"))?;
    }
    // `Tab` walks from the list to each run's summary; `Enter` opens the blocked and failed runs.
    fx.keys("tab tab enter tab enter")?;
    ensure!(
        fx.read(|r, cx| r.dialog()?.read(cx).hook_runs_open())
            == Some(vec![false, true, true, false, false]),
        "{:?}",
        fx.read(|r, cx| r.dialog()?.read(cx).hook_runs_open())
    );
    if let Some(out) = &shots {
        fx.shot()?.save(format!("{out}/3-open.png"))?;
    }
    fx.keys("escape")?;
    ensure!(fx.read(|r, _| r.dialog().is_none()), "dialog still open");
    Ok(())
}
