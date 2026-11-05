#[path = "common/mod.rs"]
mod common;

use anyhow::{Result, ensure};
use common::{CWD, Fx, GREETING, ROOT, SUBTRACT, exchange, recorded_thread, script};
use serde_json::{Value, json};
use ui::screens::thread::view::EditMessage;

#[test]
fn proxy() {
    common::proxy_child();
}

const IMAGE: &str = "fixtures/media/guide-page-1-pdfjs.png";

/// The GREETING chat forked from SUBTRACT, with a cited answer and a second turn that sent an
/// image and a file mention and then searched the web, spawned an agent and made an image.
fn thread() -> Result<Value> {
    let image = format!("{ROOT}/{IMAGE}");
    let mut thread = recorded_thread(GREETING)?;
    thread["forkedFromId"] = SUBTRACT.into();
    let answer = thread["turns"][0]["items"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap();
    answer["memoryCitation"] = json!({"entries": [
        {"path": "memories/greeting.md", "lineStart": 3, "lineEnd": 5, "note": "Prefers Hello, Codex"},
    ], "threadIds": []});
    let mut turn = thread["turns"][0].clone();
    turn["id"] = "turn-items".into();
    turn["items"] = json!([
        {"type": "userMessage", "id": "u2", "content": [
            {"type": "text", "text": "Check this page", "text_elements": []},
            {"type": "localImage", "path": image},
            {"type": "mention", "name": "hello.py", "path": format!("{CWD}/hello.py")},
        ]},
        {"type": "webSearch", "id": "w1", "query": "gpui image element"},
        {"type": "collabAgentToolCall", "id": "c1", "tool": "spawnAgent", "status": "completed",
            "senderThreadId": GREETING, "receiverThreadIds": [SUBTRACT], "prompt": "Review hello.py",
            "agentsStates": {SUBTRACT: {"status": "running", "message": null}}},
        {"type": "imageGeneration", "id": "g1", "status": "completed", "result": "",
            "savedPath": image, "revisedPrompt": "A page of a guide"},
        {"type": "imageView", "id": "v1", "path": image},
    ]);
    thread["turns"].as_array_mut().unwrap().push(turn);
    Ok(thread)
}

/// Replaces every recorded read of GREETING with [`thread`].
fn inject(line: &mut Value) {
    if let Some(recorded) = line.pointer_mut("/msg/result/thread")
        && recorded["id"] == GREETING
        && recorded["turns"].is_array()
    {
        *recorded = thread().unwrap();
    }
}

fn open(extra: Vec<[Value; 2]>) -> Result<Fx> {
    let mut lines = script(None, inject)?;
    lines.extend(extra.into_iter().flatten());
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    Ok(fx)
}

#[test]
fn new_items_render_and_edit_resends_attachments() -> Result<()> {
    let mut fx = open(vec![
        exchange(
            "thread/revert",
            json!({"thread": recorded_thread(GREETING)?}),
        ),
        exchange(
            "turn/start",
            json!({"turn": {"id": "t3", "status": "inProgress", "items": []}}),
        ),
    ])?;
    if let Some(dir) = std::env::var_os("CGX_THREAD_ITEMS_SHOTS") {
        std::fs::create_dir_all(&dir)?;
        let dir = std::path::Path::new(&dir);
        for (name, dy) in [("turn", 0.), ("items", -4000.)] {
            fx.scroll((640., 400.), dy)?;
            fx.advance(std::time::Duration::from_secs(1))?;
            fx.shot()?.save(dir.join(format!("{name}.png")))?;
        }
    }
    let editable = fx.read(|r, cx| r.thread().unwrap().read(cx).can_edit("turn-items"));
    ensure!(
        editable,
        "a message with an image and a mention is not editable"
    );
    let editor = fx.composer();
    let image = format!("{ROOT}/{IMAGE}");
    let mention = json!({"type": "mention", "name": "hello.py", "path": format!("{CWD}/hello.py")});
    fx.action(Box::new(EditMessage {
        thread_id: GREETING.into(),
        turn_id: "turn-items".into(),
        message: "Check this page again".into(),
        attachments: vec![image.clone().into()],
        extra: vec![mention.clone()],
        editor,
    }))?;
    fx.wait("turn/start", |fx| {
        fx.sent()
            .is_ok_and(|s| s.iter().any(|r| r["method"] == "turn/start"))
    })?;
    let start = fx
        .sent()?
        .into_iter()
        .find(|r| r["method"] == "turn/start")
        .unwrap();
    ensure!(
        start["params"]["input"]
            == json!([
                {"type": "text", "text": "Check this page again", "text_elements": []},
                {"type": "localImage", "path": image},
                mention,
            ]),
        "edit input lost attachments: {start}"
    );
    Ok(())
}
