mod common;

use anyhow::Result;
use app::Route;
use bridge::Event;
use common::{CWD, Fx, GREETING, SUBTRACT, exchange, proxy_child, recorded_thread, script};
use gpui::Focusable as _;
use serde_json::{Value, json};

#[test]
fn proxy() {
    proxy_child();
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(dir) = std::env::var_os("CGX_THREAD_RECOVERY_PROOF") {
        let image = fx.cx.capture_screenshot(fx.window)?;
        std::fs::create_dir_all(&dir)?;
        image.save(std::path::Path::new(&dir).join(format!("{name}.png")))?;
    }
    Ok(())
}

fn event(fx: &mut Fx, method: &str, params: Value) {
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            Event::Notification {
                method: method.into(),
                params,
            },
            cx,
        )
    });
    fx.cx.run_until_parked();
}

fn errors(fx: &Fx, turn: &str) -> Vec<String> {
    fx.read(|root, cx| {
        root.thread()
            .map(|view| view.read(cx).turn_error_messages(turn))
            .unwrap_or_default()
    })
}

#[test]
fn history_failure_retries_and_navigation_ignores_the_old_request() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut lines = script(None, |_| {})?;
    let request = lines
        .iter()
        .position(|line| {
            line["msg"]["method"] == "thread/read" && line["msg"]["params"]["threadId"] == SUBTRACT
        })
        .unwrap();
    let id = lines[request]["msg"]["id"].clone();
    let response = lines
        .iter()
        .position(|line| line["dir"] == "recv" && line["msg"]["id"] == id)
        .unwrap();
    let successful = lines[response]["msg"]["result"].clone();
    lines[response]["msg"]
        .as_object_mut()
        .unwrap()
        .remove("result");
    lines[response]["msg"]["error"] =
        json!({"code": -32000, "message": "History fixture unavailable"});
    let mut retry = exchange("thread/read", successful);
    retry[0]["msg"]["params"] = lines[request]["msg"]["params"].clone();
    lines.extend(retry);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    let prior_composer = fx.composer();
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(Event::Request {
        id: json!(503001),
        method: "item/commandExecution/requestApproval".into(),
        params: json!({"threadId": GREETING, "turnId": "prior-turn", "itemId": "prior-command",
            "command": "pwd", "cwd": CWD, "reason": "Keep this request on the prior chat"}),
    }, cx)
    });
    fx.cx.run_until_parked();
    assert!(fx.read(|_, cx| prior_composer.read(cx).approval().is_some()));
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(app::GoToChat(1)), cx);
    })?;
    assert!(fx.read(|root, cx| root.history().unwrap().read(cx).error().is_none()));
    shot(&mut fx, "history-loading")?;
    fx.wait("source history failure", |fx| {
        fx.read(|root, cx| {
            root.history().is_some_and(|view| {
                view.read(cx)
                    .error()
                    .is_some_and(|e| e.contains("History fixture unavailable"))
            })
        })
    })?;
    fx.cx.update_window(fx.window, |_, window, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            "Do not send to the prior chat".into(),
        ));
        prior_composer.focus_handle(cx).focus(window, cx);
    })?;
    fx.keys("enter cmd-v enter")?;
    let sent = fx.sent()?;
    assert!(!sent.iter().any(|request| request["reply"] == 503001
        || matches!(
            request["method"].as_str(),
            Some("turn/start" | "turn/steer" | "thread/queue/add")
        )));
    assert_eq!(fx.composer_text()?, "");
    shot(&mut fx, "history-failure")?;
    fx.click((1135., 786.))?;
    fx.wait_thread(SUBTRACT)?;
    assert!(fx.read(|root, _| root.history().is_none()));
    let reads: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == "thread/read" && r["params"]["threadId"] == SUBTRACT)
        .collect();
    assert_eq!(reads.len(), 2);
    assert!(
        reads
            .iter()
            .all(|r| r["params"]["threadId"] == SUBTRACT && r["params"]["includeTurns"] == true)
    );
    shot(&mut fx, "history-retried")?;

    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(app::GoToChat(0)), cx);
        window.dispatch_action(Box::new(ui::NewChat), cx);
    })?;
    fx.cx.run_until_parked();
    assert_eq!(fx.route(), Route::Home);
    assert!(fx.read(|root, _| root.history().is_none()));
    Ok(())
}

#[test]
fn stream_errors_stay_with_their_thread_and_turn_after_progress() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    let mut turn = recorded_thread(SUBTRACT)?["turns"][0].clone();
    turn["id"] = "recovery-turn".into();
    turn["status"] = "inProgress".into();
    turn["items"] = json!([{"type":"agentMessage","id":"recovery-message","text":"Still working"}]);
    event(
        &mut fx,
        "turn/started",
        json!({"threadId":SUBTRACT,"turn":turn}),
    );
    let notice = |thread: &str, retry: bool, message: &str, info: Value| {
        json!({
            "threadId":thread,"turnId":"recovery-turn","willRetry":retry,
            "error":{"message":message,"codexErrorInfo":info,"additionalDetails":"Source fixture connection closed"}
        })
    };
    event(
        &mut fx,
        "error",
        notice("another-thread", true, "Reconnecting... 1/5", Value::Null),
    );
    assert!(errors(&fx, "recovery-turn").is_empty());
    event(
        &mut fx,
        "error",
        notice(SUBTRACT, true, "Reconnecting... 1/5", Value::Null),
    );
    assert_eq!(errors(&fx, "recovery-turn"), vec!["Reconnecting 1/5"]);
    shot(&mut fx, "reconnecting")?;
    event(
        &mut fx,
        "error",
        notice(
            SUBTRACT,
            true,
            "Reconnecting... 2/5",
            json!({"responseStreamDisconnected":{"httpStatusCode":429}}),
        ),
    );
    assert_eq!(
        errors(&fx, "recovery-turn"),
        vec!["Server is busy, reconnecting 2/5"]
    );
    shot(&mut fx, "server-busy")?;
    event(
        &mut fx,
        "item/agentMessage/delta",
        json!({"threadId":SUBTRACT,"turnId":"recovery-turn","itemId":"recovery-message","delta":" again"}),
    );
    fx.advance(ui::motion::STREAMING_TEXT)?;
    assert_eq!(
        errors(&fx, "recovery-turn"),
        vec!["Server is busy, reconnecting 2/5"]
    );
    event(
        &mut fx,
        "error",
        notice(
            SUBTRACT,
            false,
            "Source quota exhausted",
            json!("usageLimitExceeded"),
        ),
    );
    assert_eq!(
        errors(&fx, "recovery-turn"),
        vec![
            "Server is busy, reconnecting 2/5",
            "You've hit your usage limit. Add credits to continue, or try again later.",
        ]
    );
    shot(&mut fx, "usage-limit")?;
    turn["status"] = "failed".into();
    event(
        &mut fx,
        "turn/completed",
        json!({"threadId":SUBTRACT,"turn":turn}),
    );
    assert_eq!(
        errors(&fx, "recovery-turn"),
        vec![
            "Server is busy, reconnecting 2/5",
            "You've hit your usage limit. Add credits to continue, or try again later.",
        ]
    );
    turn["id"] = "next-turn".into();
    turn["status"] = "inProgress".into();
    event(
        &mut fx,
        "turn/started",
        json!({"threadId":SUBTRACT,"turn":turn}),
    );
    assert!(errors(&fx, "next-turn").is_empty());
    Ok(())
}

#[test]
fn a_failed_turn_render_keeps_other_turns_and_retries_with_the_latest_data() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    let turn_ids = |fx: &Fx| {
        fx.read(|root, cx| {
            root.thread()
                .unwrap()
                .read(cx)
                .turns()
                .iter()
                .map(|turn| turn.id.clone())
                .collect::<Vec<_>>()
        })
    };
    let mut expected_turns = turn_ids(&fx);
    expected_turns.push("bad-render".to_owned());
    let mut broken = recorded_thread(SUBTRACT)?["turns"][0].clone();
    broken["id"] = "bad-render".into();
    broken["startedAt"] = i64::MIN.into();
    event(
        &mut fx,
        "turn/started",
        json!({"threadId":SUBTRACT,"turn":broken}),
    );
    fx.advance(std::time::Duration::ZERO)?;
    shot(&mut fx, "render-failure")?;
    assert!(fx.read(|root, cx| {
        root.thread()
            .unwrap()
            .read(cx)
            .render_error("bad-render")
            .is_some()
    }));
    assert_eq!(turn_ids(&fx), expected_turns);
    broken["startedAt"] = 1_792_040_071_i64.into();
    event(
        &mut fx,
        "turn/completed",
        json!({"threadId":SUBTRACT,"turn":broken}),
    );
    fx.click((500., 520.))?;
    shot(&mut fx, "render-retried")?;
    assert!(fx.read(|root, cx| {
        root.thread()
            .unwrap()
            .read(cx)
            .render_error("bad-render")
            .is_none()
    }));
    assert_eq!(turn_ids(&fx), expected_turns);
    Ok(())
}
