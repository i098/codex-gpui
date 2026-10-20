//! CP-115: the `Queued message actions` menu of a queued row. `Edit message` takes the
//! submission out of the app-server's queue (`thread/queue/delete`), moves its text and images
//! into the composer and, when sent, queues it again where it stood (`thread/queue/add`, then
//! `thread/queue/reorder`). `Turn off queueing` / `Turn on queueing` writes
//! `desktop.followUpQueueMode`, which the next submit follows.

mod common;

use anyhow::Result;
use bridge::{Event, types::TurnStatus};
use common::{Fx, SUBTRACT, exchange, proxy_child, script};
use serde_json::{Value, json};

#[test]
fn proxy() {
    proxy_child();
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

fn sent(fx: &mut Fx, method: &str) -> Result<Vec<Value>> {
    Ok(fx
        .sent()?
        .into_iter()
        .filter(|r| r["method"] == method)
        .collect())
}

fn queued(fx: &mut Fx) -> Vec<String> {
    fx.read(|root, cx| {
        let composer = root.composer(cx);
        composer
            .read(cx)
            .queued()
            .iter()
            .map(|row| row.text.to_string())
            .collect()
    })
}

/// The open menu's `(id, label)` rows.
fn menu(fx: &mut Fx) -> Vec<(String, String)> {
    fx.read(|root, cx| {
        let composer = root.composer(cx);
        composer
            .read(cx)
            .context_menu_items()
            .map(|(id, label, _)| (id.to_owned(), label.to_owned()))
            .collect()
    })
}

fn submission(id: &str, input: Value) -> Value {
    json!({ "id": id, "clientUserMessageId": format!("{id}-client"), "input": input })
}

fn text(text: &str) -> Value {
    json!({ "type": "text", "text": text, "text_elements": [] })
}

/// The `thread/queue/list` answer to the next list request of the thread.
fn list(rows: &[&Value]) -> [Value; 2] {
    let mut list = exchange(
        "thread/queue/list",
        json!({ "data": rows, "nextCursor": null }),
    );
    list[0]["msg"]["params"] = json!({ "threadId": SUBTRACT, "cursor": null });
    list
}

fn queue_mode(line: &mut Value) {
    if let Some(desktop) = line.pointer_mut("/msg/result/config/desktop") {
        desktop["followUpQueueMode"] = "queue".into();
    }
}

/// Opens the subtract thread with a turn in progress and the queue the server reports.
fn open_running(fx: &mut Fx) -> Result<()> {
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    notify(
        fx,
        "turn/started",
        json!({"threadId": SUBTRACT, "turn": {"id": "queue-options-turn", "items": [], "status": "inProgress", "error": null}}),
    );
    fx.wait("running turn", |fx| {
        fx.read(|root, cx| {
            root.thread()
                .unwrap()
                .read(cx)
                .turns()
                .last()
                .is_some_and(|turn| turn.status == TurnStatus::InProgress)
        })
    })
}

/// The `…` of row `index` of `rows`: the tray grows upward from the card, its last row at the
/// recorded single row (`s023b`: y 682, 32 px high, right edge 1163), 33 px per row.
fn actions_button(rows: usize, index: usize) -> (f32, f32) {
    (
        1163. - 1. - 10. - 12.,
        698. - 33. * (rows - 1 - index) as f32,
    )
}

fn save(fx: &mut Fx, name: &str) -> Result<()> {
    fx.shot()?.save(
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("queue-options-{name}.png")),
    )?;
    Ok(())
}

#[test]
fn edit_moves_the_exact_draft_back_and_queues_it_in_place() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let image = json!({ "type": "localImage", "path": "/tmp/cgx-mac/proj/shot.png" });
    let skill = json!({ "type": "skill", "name": "review", "path": "/tmp/cgx-mac/proj/SKILL.md" });
    let draft = "second draft ";
    let first = submission("a", json!([text("first")]));
    let second = submission("b", json!([text(draft), image, skill]));
    let third = submission("c", json!([text("third")]));
    let again = submission("b2", json!([text(draft), image, skill]));

    let mut lines = script(None, queue_mode)?;
    lines.extend(exchange("thread/resume", json!({})));
    lines.extend(list(&[&first, &second, &third]));
    lines.extend(exchange("thread/queue/delete", json!({ "deleted": true })));
    lines.extend(list(&[&first, &third]));
    lines.extend(exchange(
        "thread/queue/add",
        json!({ "queuedSubmission": again }),
    ));
    lines.extend(list(&[&first, &third, &again]));
    lines.extend(exchange("thread/queue/reorder", json!({})));
    lines.extend(list(&[&first, &again, &third]));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    open_running(&mut fx)?;
    fx.wait("three rows", |fx| {
        queued(fx) == ["first", draft.trim_end(), "third"]
    })?;
    save(&mut fx, "tray")?;

    fx.click(actions_button(3, 1))?;
    save(&mut fx, "menu")?;
    assert_eq!(
        menu(&mut fx),
        [
            ("queuedEdit".to_owned(), "Edit message".to_owned()),
            ("queuedQueueing".to_owned(), "Turn off queueing".to_owned())
        ]
    );
    fx.keys("down enter")?;
    fx.wait("draft in the composer", |fx| {
        fx.composer_text().is_ok_and(|text| text == draft)
    })?;
    assert!(fx.composer_focused()?);
    fx.wait("row left the tray", |fx| queued(fx) == ["first", "third"])?;
    save(&mut fx, "edited")?;
    let deleted = sent(&mut fx, "thread/queue/delete")?;
    assert_eq!(deleted.len(), 1, "{deleted:?}");
    assert_eq!(
        deleted[0]["params"],
        json!({ "threadId": SUBTRACT, "queuedSubmissionId": "b" })
    );

    // Sent again, the draft keeps its image and skill and returns between the other rows.
    fx.keys("enter")?;
    fx.wait("queued in place", |fx| {
        queued(fx) == ["first", draft.trim_end(), "third"]
    })?;
    let added = sent(&mut fx, "thread/queue/add")?;
    assert_eq!(added.len(), 1, "{added:?}");
    assert_eq!(
        added[0]["params"]["input"],
        json!([text(draft), image, skill])
    );
    let reordered = sent(&mut fx, "thread/queue/reorder")?;
    assert_eq!(reordered.len(), 1, "{reordered:?}");
    assert_eq!(
        reordered[0]["params"],
        json!({ "threadId": SUBTRACT, "queuedSubmissionIds": ["a", "b2", "c"] })
    );
    assert_eq!(fx.composer_text()?, "");
    Ok(())
}

#[test]
fn edit_keeps_the_current_draft_and_a_submission_the_server_already_took() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let only = submission("a", json!([text("queued")]));
    let mut lines = script(None, queue_mode)?;
    lines.extend(exchange("thread/resume", json!({})));
    lines.extend(list(&[&only]));
    lines.extend(exchange("thread/queue/delete", json!({ "deleted": false })));
    lines.extend(list(&[]));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    open_running(&mut fx)?;
    fx.wait("one row", |fx| queued(fx) == ["queued"])?;
    fx.type_text("typed")?;

    // The server started the submission first: nothing moves into the composer.
    fx.click(actions_button(1, 0))?;
    fx.keys("down enter")?;
    fx.wait("row gone", |fx| queued(fx).is_empty())?;
    assert_eq!(fx.composer_text()?, "typed");
    assert_eq!(sent(&mut fx, "thread/queue/delete")?.len(), 1);
    Ok(())
}

#[test]
fn queueing_switch_decides_what_the_next_submit_does() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let only = submission("a", json!([text("queued")]));
    let mut lines = script(None, queue_mode)?;
    lines.extend(exchange("thread/resume", json!({})));
    lines.extend(list(&[&only]));
    lines.extend(exchange("config/value/write", json!({})));
    lines.extend(exchange("turn/steer", json!({})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    open_running(&mut fx)?;
    fx.wait("one row", |fx| queued(fx) == ["queued"])?;

    fx.click(actions_button(1, 0))?;
    fx.keys("down down enter")?;
    fx.wait("steer mode written", |fx| {
        fx.read(|root, _| root.config()["desktop"]["followUpQueueMode"] == "steer")
    })?;
    let written = sent(&mut fx, "config/value/write")?;
    assert_eq!(written.len(), 1, "{written:?}");
    assert_eq!(written[0]["params"]["keyPath"], "desktop.followUpQueueMode");
    assert_eq!(written[0]["params"]["value"], "steer");

    fx.click(actions_button(1, 0))?;
    assert_eq!(menu(&mut fx)[1].1, "Turn on queueing");
    fx.keys("escape")?;
    fx.click((809., 750.))?;
    fx.type_text("now")?;
    fx.keys("enter")?;
    fx.wait("steered", |fx| {
        sent(fx, "turn/steer").is_ok_and(|steered| steered.len() == 1)
    })?;
    assert!(sent(&mut fx, "thread/queue/add")?.is_empty());
    Ok(())
}

#[test]
fn a_refused_queueing_switch_leaves_the_behavior_that_applies() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let only = submission("a", json!([text("queued")]));
    let mut lines = script(None, queue_mode)?;
    lines.extend(exchange("thread/resume", json!({})));
    lines.extend(list(&[&only]));
    let mut write = exchange("config/value/write", json!({}));
    write[1]["msg"].as_object_mut().unwrap().remove("result");
    write[1]["msg"]["error"] = json!({"code": -32000, "message": "config is read-only"});
    lines.extend(write);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    open_running(&mut fx)?;
    fx.wait("one row", |fx| queued(fx) == ["queued"])?;

    fx.click(actions_button(1, 0))?;
    fx.keys("down down enter")?;
    fx.wait("refusal toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|t| t.as_ref() == "Couldn’t save your changes. Try again")
        })
    })?;
    assert_eq!(
        fx.read(|root, _| root.config()["desktop"]["followUpQueueMode"].clone()),
        "queue"
    );
    // The composer still names the behavior a submit follows.
    fx.type_text("x")?;
    assert_eq!(
        fx.read(|root, cx| root.composer(cx).read(cx).send_label()),
        "Queue"
    );
    Ok(())
}
