//! `Code review` (`/review-mode`, the CLI's `/review`): the submenu lists
//! `Review uncommitted changes`, the chat's branches and its commits (host `git-review-targets`
//! on a real repository), and each pick sends `review/start` with the CLI's target on the open
//! chat. Escape sends nothing; a refused request shows `Failed to start code review`.

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, GREETING, exchange, recorded_thread, script};
use serde_json::{Value, json};
use std::{path::Path, process::Command};

#[test]
fn proxy() {
    common::proxy_child();
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()?;
    ensure!(output.status.success(), "git {args:?} failed");
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

/// `main` with `add calc`, then the checked-out `feature` with `break add` and an untracked file.
fn repository() -> Result<tempfile::TempDir> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    git(root, &["init", "-q", "-b", "main"])?;
    std::fs::write(root.join("calc.py"), "def add(a, b):\n    return a + b\n")?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-qm", "add calc"])?;
    git(root, &["checkout", "-qb", "feature"])?;
    std::fs::write(root.join("calc.py"), "def add(a, b):\n    return a - b\n")?;
    git(root, &["commit", "-qam", "break add"])?;
    std::fs::write(root.join("div.py"), "def div(a, b):\n    return a / b\n")?;
    Ok(temp)
}

/// The recorded chat, moved into `root`, plus `starts` answered `review/start` requests and the
/// replies in `extra`.
fn chat(root: &Path, starts: usize, extra: &[Value]) -> Result<Fx> {
    let mut thread = recorded_thread(GREETING)?;
    let recorded = thread["cwd"].as_str().context("recorded cwd")?.to_owned();
    thread["cwd"] = json!(root);
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/resume", json!({"thread": thread})));
    for _ in 0..starts {
        lines.extend(exchange(
            "review/start",
            json!({"turn": {"id": "review", "status": "inProgress", "error": null, "items": [
                    {"type": "userMessage", "id": "review",
                        "content": [{"type": "text", "text": "current changes"}]}]},
                "reviewThreadId": GREETING}),
        ));
    }
    lines.extend_from_slice(extra);
    fn moved(value: &mut Value, from: &str, to: &Value) {
        match value {
            Value::String(text) if text == from => *value = to.clone(),
            Value::Array(items) => items.iter_mut().for_each(|v| moved(v, from, to)),
            Value::Object(map) => map.values_mut().for_each(|v| moved(v, from, to)),
            _ => {}
        }
    }
    lines
        .iter_mut()
        .for_each(|line| moved(line, &recorded, &json!(root)));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.click(fx.at("thread", "Work with Codex")?)?;
    ensure!(fx.composer_focused()?, "the composer must have focus");
    Ok(fx)
}

fn rows(fx: &Fx) -> Result<Vec<String>> {
    let (rows, _) = fx
        .read(|root, cx| root.composer(cx).read(cx).slash_state())
        .context("slash menu must be open")?;
    Ok(rows
        .into_iter()
        .map(|row| row.split('\n').next().unwrap().to_owned())
        .collect())
}

fn starts(fx: &mut Fx) -> Result<Vec<Value>> {
    Ok(fx
        .sent()?
        .into_iter()
        .filter(|request| request["method"] == "review/start")
        .map(|request| request["params"].clone())
        .collect())
}

/// Opens the submenu and waits for the repository's targets.
fn open_review(fx: &mut Fx) -> Result<()> {
    fx.type_text("/review-mode")?;
    ensure!(rows(fx)? == ["Code review"], "{:?}", rows(fx)?);
    fx.keys("enter")?;
    ensure!(fx.composer_text()? == "/review-mode ");
    fx.wait("review targets", |fx| {
        rows(fx).is_ok_and(|rows| rows.iter().any(|row| row == "main"))
    })
}

#[test]
fn review_targets_start_review_on_the_open_chat() -> Result<()> {
    let repo = repository()?;
    let head = git(repo.path(), &["rev-parse", "HEAD"])?;
    let mut fx = chat(repo.path(), 3, &[])?;
    open_review(&mut fx)?;
    ensure!(
        rows(&fx)?
            == [
                "Review uncommitted changes",
                "main",
                "break add",
                "add calc"
            ],
        "the current branch is not a base: {:?}",
        rows(&fx)?
    );
    if let Some(dir) = std::env::var_os("CGX_REVIEW_SHOTS") {
        fx.shot()?
            .save(Path::new(&dir).join("review-submenu.png"))?;
    }
    fx.keys("escape")?;
    ensure!(starts(&mut fx)?.is_empty(), "Escape sends nothing");
    ensure!(
        fx.composer_text()? == "/review-mode ",
        "Escape leaves the text"
    );
    fx.keys(&["backspace"; 13].join(" "))?;
    ensure!(fx.composer_text()?.is_empty());

    let picks = [
        ("", json!({"type": "uncommittedChanges"})),
        ("main", json!({"type": "baseBranch", "branch": "main"})),
        (
            "break",
            json!({"type": "commit", "sha": head, "title": "break add"}),
        ),
    ];
    for (n, (query, target)) in picks.into_iter().enumerate() {
        open_review(&mut fx)?;
        fx.type_text(query)?;
        fx.keys("enter")?;
        fx.wait("review/start reply", |fx| {
            starts(fx).is_ok_and(|s| s.len() == n + 1) && running(fx) == 1
        })?;
        let sent = starts(&mut fx)?;
        ensure!(
            sent[n] == json!({"threadId": GREETING, "target": target}),
            "{sent:?}"
        );
        ensure!(fx.composer_text()?.is_empty() && fx.composer_focused()?);
        // One review runs at a time: the row hides until this one ends.
        notify(
            &mut fx,
            "turn/completed",
            json!({"threadId": GREETING,
            "turn": {"id": "review", "rootTurnId": "review", "items": [], "status": "completed"}}),
        );
    }
    ensure!(fx.read(|root, cx| root.toasts(cx).is_empty()));
    Ok(())
}

#[test]
fn refused_review_is_reported() -> Result<()> {
    let repo = repository()?;
    // No recorded `review/start`: the replay answers it with an error.
    let mut fx = chat(repo.path(), 0, &[])?;
    open_review(&mut fx)?;
    fx.keys("enter")?;
    fx.wait("failed review toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|t| t == "Failed to start code review")
        })
    })?;
    ensure!(starts(&mut fx)?.len() == 1);
    Ok(())
}

fn notify(fx: &mut Fx, method: &str, params: Value) {
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            bridge::Event::Notification {
                method: method.into(),
                params,
            },
            cx,
        )
    });
    fx.cx.run_until_parked();
}

fn running(fx: &Fx) -> usize {
    fx.read(|root, cx| {
        root.thread().map_or(0, |view| {
            view.read(cx)
                .turns()
                .iter()
                .filter(|t| t.status == bridge::types::TurnStatus::InProgress)
                .count()
        })
    })
}

/// The live server runs a review as a sub-turn: `turn/started` names it with `rootTurnId` the
/// `review/start` turn, items and `turn/completed` name the root, and `turn/interrupt` accepts
/// only the sub-turn's id.
#[test]
fn stop_interrupts_the_running_review() -> Result<()> {
    let repo = repository()?;
    let interrupt = [
        json!({"dir": "send", "msg": {"id": 301, "method": "turn/interrupt",
            "params": {"threadId": GREETING, "turnId": "review-sub"}}}),
        json!({"dir": "recv", "msg": {"id": 301, "result": {}}}),
    ];
    let mut fx = chat(repo.path(), 1, &interrupt)?;
    open_review(&mut fx)?;
    fx.keys("enter")?;
    fx.wait("review/start reply", |fx| {
        starts(fx).is_ok_and(|s| s.len() == 1) && running(fx) == 1
    })?;
    let turn = |fx: &mut Fx, method: &str, id: &str, status: &str| {
        let turn = json!({"id": id, "rootTurnId": "review", "items": [], "status": status});
        notify(fx, method, json!({"threadId": GREETING, "turn": turn}));
    };
    turn(&mut fx, "turn/started", "review-sub", "inProgress");
    ensure!(running(&fx) == 1, "the sub-turn runs as the review turn");
    notify(
        &mut fx,
        "item/started",
        json!({"threadId": GREETING, "turnId": "review", "item": {
        "type": "userMessage", "id": "prompt",
        "content": [{"type": "text", "text": "Review the current code changes"}]}}),
    );
    let items = fx.read(|root, cx| {
        root.thread()
            .map(|view| format!("{:?}", view.read(cx).turns()))
    });
    ensure!(
        items
            .as_ref()
            .is_some_and(|items| items.contains("current changes")
                && !items.contains("Review the current code changes")),
        "the reply's hint shows, the forwarded prompt does not: {items:?}"
    );
    fx.type_text("/review-mode")?;
    ensure!(
        rows(&fx).map_or(true, |rows| !rows.iter().any(|r| r == "Code review")),
        "a running turn hides Code review"
    );
    fx.keys(&["backspace"; 12].join(" "))?;
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(ui::composer::Stop), cx)
    })?;
    fx.wait("turn/interrupt", |fx| {
        fx.sent()
            .unwrap()
            .iter()
            .any(|r| r["method"] == "turn/interrupt" && r["params"]["turnId"] == "review-sub")
    })?;
    turn(&mut fx, "turn/completed", "review", "interrupted");
    ensure!(running(&fx) == 0, "the review's completion ends it");
    ensure!(fx.read(|root, cx| root.toasts(cx).is_empty()));
    Ok(())
}
