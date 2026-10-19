//! A refused `thread/start` from Home shows `Error starting chat` and gives the draft back:
//! its exact text, its attachment and the chosen project. Sending again starts the chat in
//! that project with the attachment.

mod common;

use anyhow::Result;
use app::{Root, Route, host::LocalHost};
use common::{Fx, GREETING, exchange, proxy_child, recorded_thread, script};
use serde_json::{Value, json};
use std::{fs, sync::Arc};

#[test]
fn proxy() {
    proxy_child();
}

fn requests(fx: &mut Fx, method: &str) -> Vec<Value> {
    fx.sent()
        .unwrap_or_default()
        .into_iter()
        .filter(|request| request["method"] == method)
        .collect()
}

#[test]
fn refused_thread_start_keeps_the_draft_attachment_and_project() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let temp = tempfile::tempdir()?;
    let alpha = temp.path().join("alpha");
    fs::create_dir(&alpha)?;
    let alpha = alpha.canonicalize()?;
    let notes = temp.path().join("notes.txt");
    fs::write(&notes, "attached\n")?;

    let mut lines = script(None, |_| {})?;
    let mut refused = exchange("thread/start", json!({}));
    refused[1]["msg"].as_object_mut().unwrap().remove("result");
    refused[1]["msg"]["error"] = json!({"code": -32000, "message": "route1: thread/start refused"});
    lines.extend(refused);
    let mut started = recorded_thread(GREETING)?;
    started["id"] = "start-retry".into();
    started["cwd"] = json!(alpha);
    started["turns"] = json!([]);
    lines.extend(exchange(
        "thread/start",
        json!({"thread": started, "model": "gpt-6.1-sol", "cwd": alpha}),
    ));
    lines.extend(exchange(
        "turn/start",
        json!({"turn": {"id": "retry-turn", "items": [], "status": "completed", "error": null}}),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let host = Arc::new(LocalHost::new(temp.path().join("codex")));
    fx.root
        .update(&mut fx.cx, |root: &mut Root, cx| root.set_host(host, cx));
    fx.cx.run_until_parked();

    fx.click((515., 695.))?;
    fx.cx
        .simulate_path_prompt_response(|_| Some(vec![alpha.clone()]));
    fx.wait("project selected", |fx| {
        fx.read(|root, cx| !root.sidebar().read(cx).projects().is_empty())
    })?;
    fx.keys("cmd-n")?;
    let composer = fx.composer();
    composer.update(&mut fx.cx, |c, cx| {
        c.drop_files(std::slice::from_ref(&notes), cx)
    });
    fx.type_text("Start this chat in alpha.")?;
    fx.keys("enter")?;

    fx.wait("start error toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|t| t.as_ref() == "Error starting chat")
        })
    })?;
    assert_eq!(fx.route(), Route::Home);
    assert_eq!(fx.composer_text()?, "Start this chat in alpha.");

    fx.keys("enter")?;
    fx.wait_thread("start-retry")?;
    let mut turns = Vec::new();
    fx.wait("retried turn", |fx| {
        turns = requests(fx, "turn/start");
        !turns.is_empty()
    })?;
    for start in requests(&mut fx, "thread/start") {
        assert_eq!(start["params"]["cwd"], json!(alpha));
    }
    assert_eq!(requests(&mut fx, "thread/start").len(), 2);
    assert_eq!(
        turns[0]["params"]["input"][0]["text"],
        format!("Start this chat in alpha.\n{}", notes.display())
    );
    Ok(())
}
