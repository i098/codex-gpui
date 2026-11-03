mod common;

use anyhow::Result;
use app::{Root, Route, State};
use bridge::{Bridge, types::ThreadItem};
use common::{Fx, GREETING, script};
use gpui::{AppContext as _, Focusable as _, px, size};
use serde_json::{Value, json};
use std::sync::Arc;

#[test]
fn remote_thread_reads_writes_and_events_stay_on_its_connection() -> Result<()> {
    let mut local = Fx::new()?;
    local.cx.update_window(local.window, |_, window, cx| {
        local.root.update(cx, |root, cx| {
            let link = app::deep_link::parse(&format!("codex://threads/{GREETING}")).unwrap();
            root.open_deep_link(&link, window, cx);
        });
    })?;
    local.wait_thread(GREETING)?;
    let local_turns =
        local.read(|root, cx| format!("{:?}", root.thread().unwrap().read(cx).turns()));

    let mut lines = script(None, |_| {})?;
    let pages: Vec<Value> = lines
        .iter()
        .filter(|line| line["msg"]["method"] == "thread/turns/list")
        .map(|line| line["msg"]["id"].clone())
        .collect();
    for line in &mut lines {
        if line["msg"]["result"]["thread"]["id"] == GREETING {
            line["msg"]["result"]["thread"]["turns"] = json!([]);
        }
        // The paged read (`thread/turns/list`) carries the turns too.
        if line["dir"] == "recv" && pages.contains(&line["msg"]["id"]) {
            line["msg"]["result"]["data"] = json!([]);
        }
    }
    let turn = json!({"id": "remote-turn", "items": [], "status": "inProgress", "error": null});
    let item = json!({"id": "remote-answer", "type": "agentMessage", "text": "Remote answer"});
    lines.extend([
        json!({"dir": "send", "msg": {"id": 900001, "method": "turn/start", "params": {}}}),
        json!({"dir": "recv", "msg": {"id": 900001, "result": {"turn": turn}}}),
        json!({"dir": "recv", "msg": {"method": "item/started", "params": {"threadId": GREETING, "turnId": "remote-turn", "item": item}}}),
        json!({"dir": "recv", "msg": {"method": "item/completed", "params": {"threadId": GREETING, "turnId": "remote-turn", "item": item}}}),
        json!({"dir": "recv", "msg": {"method": "turn/completed", "params": {"threadId": GREETING, "turn": {"id": "remote-turn", "items": [item], "status": "completed", "error": null}}}}),
    ]);
    let recording = tempfile::NamedTempFile::new()?;
    std::fs::write(
        recording.path(),
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )?;
    let (bridge, _) = Bridge::replay(recording.path())?;
    let bridge = Arc::new(bridge);
    let remote_window = local
        .cx
        .open_window(size(px(1280.), px(832.)), |window, cx| {
            cx.new(|cx| {
                Root::on_host(
                    State {
                        route: Route::Thread(GREETING.into()),
                        ..State::default()
                    },
                    "ssh:test-host".into(),
                    bridge.clone(),
                    "/remote/project".into(),
                    window,
                    cx,
                )
                .unwrap()
            })
        })?;
    let remote = local.cx.read_window(&remote_window, |root, _| root)?;
    local.wait("remote thread", |fx| {
        remote.read_with(&fx.cx, |root, cx| {
            root.ready()
                && root
                    .thread()
                    .is_some_and(|view| view.read(cx).id() == GREETING)
        })
    })?;
    assert_eq!(
        remote.read_with(&local.cx, |root, _| root.host_id().to_owned()),
        "ssh:test-host"
    );
    assert!(remote.read_with(&local.cx, |root, cx| {
        root.thread().unwrap().read(cx).turns().is_empty()
    }));
    local.cx.update_window(*remote_window, |_, window, cx| {
        remote.update(cx, |root, cx| {
            let composer = root.composer(cx);
            composer.update(cx, |composer, cx| {
                composer.set_text("Reply on the remote host", cx)
            });
            composer.focus_handle(cx).focus(window, cx);
        });
        window.dispatch_action(Box::new(ui::composer::Submit), cx);
    })?;
    local.wait("remote response", |fx| {
        remote.read_with(&fx.cx, |root, cx| {
            root.thread().is_some_and(|view| {
                view.read(cx).turns().iter().any(|turn| {
                    turn.items.iter().any(|item| {
                        matches!(item, ThreadItem::AgentMessage { text, .. } if text == "Remote answer")
                    })
                })
            })
        })
    })?;
    assert_eq!(
        local.read(|root, cx| format!("{:?}", root.thread().unwrap().read(cx).turns())),
        local_turns
    );
    let windows = local.cx.update(|cx| cx.windows().len());
    local.cx.update(|cx| {
        app::deep_link::open(
            &format!("codex://threads/{GREETING}?hostId=ssh%3Atest-host"),
            cx,
        );
        app::deep_link::open("codex://settings", cx);
    });
    local.wait("local settings link", |fx| fx.route() == Route::Settings)?;
    assert_eq!(local.cx.update(|cx| cx.windows().len()), windows);
    assert_eq!(
        remote.read_with(&local.cx, |root, _| root.route().clone()),
        Route::Thread(GREETING.into())
    );
    Ok(())
}

#[test]
fn remote_thread_rejects_empty_hosts_and_disconnected_bridges() -> Result<()> {
    let mut fx = Fx::new()?;
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "exit 0"]);
    let (bridge, _) = Bridge::spawn(command)?;
    let bridge = Arc::new(bridge);
    fx.wait("closed bridge", |_| bridge.is_closed())?;
    for (host_id, message) in [
        ("", "missing thread host"),
        ("ssh:closed", "thread host is disconnected"),
    ] {
        let error = fx.cx.update_window(fx.window, |_, window, cx| {
            fx.root.update(cx, |_, cx| {
                Root::on_host(
                    State::default(),
                    host_id.into(),
                    bridge.clone(),
                    "/remote/project".into(),
                    window,
                    cx,
                )
                .err()
                .unwrap()
                .to_string()
            })
        })?;
        assert_eq!(error, message);
    }
    Ok(())
}
