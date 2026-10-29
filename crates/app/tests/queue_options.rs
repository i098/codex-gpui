//! CP-115: the `Queued message actions` menu of a queued row. `Edit message` takes the
//! submission out of the app-server's queue (`thread/queue/delete`), moves its text and images
//! into the composer and, when sent, queues it again where it stood (`thread/queue/add`, then
//! `thread/queue/reorder`). `Open in side chat` moves it into an ephemeral fork of the chat
//! (`thread/fork`) with its own tab, transcript and composer, and puts it back in place when
//! that fails. `Turn off queueing` / `Turn on queueing` writes `desktop.followUpQueueMode`,
//! which the next submit follows.

mod common;

use anyhow::Result;
use app::SidePanel;
use bridge::{Event, types::TurnStatus};
use common::{Fx, SUBTRACT, exchange, proxy_child, recorded_thread, refusal, script};
use serde_json::{Value, json};
use ui::screens::diff::panel::CloseSideTab;

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
    notify(
        fx,
        "thread/status/changed",
        json!({"threadId": SUBTRACT, "status": {"type": "active", "activeFlags": []}}),
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
    edit_and_resubmit(false)
}

#[test]
fn retry_keeps_edited_images_skills_and_queue_position() -> Result<()> {
    edit_and_resubmit(true)
}

fn edit_and_resubmit(retry: bool) -> Result<()> {
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
    if retry {
        lines.extend(refusal("thread/queue/add", "Submission refused"));
    }
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
            ("queuedSideChat".to_owned(), "Open in side chat".to_owned()),
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
    if retry {
        fx.wait("failed edited submission", |fx| {
            queued(fx) == ["first", "third", draft.trim_end()]
        })?;
        assert_eq!(fx.composer_text()?, "");
        fx.click((1080., 698.))?;
    }
    fx.wait("queued in place", |fx| {
        queued(fx) == ["first", draft.trim_end(), "third"]
    })?;
    let added = sent(&mut fx, "thread/queue/add")?;
    assert_eq!(added.len(), if retry { 2 } else { 1 }, "{added:?}");
    assert_eq!(
        added[0]["params"]["input"],
        json!([text(draft), image, skill])
    );
    if retry {
        assert_eq!(added[0]["params"], added[1]["params"]);
    }
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
    fx.keys("down down down enter")?;
    fx.wait("steer mode written", |fx| {
        fx.read(|root, _| root.config()["desktop"]["followUpQueueMode"] == "steer")
    })?;
    let written = sent(&mut fx, "config/value/write")?;
    assert_eq!(written.len(), 1, "{written:?}");
    assert_eq!(written[0]["params"]["keyPath"], "desktop.followUpQueueMode");
    assert_eq!(written[0]["params"]["value"], "steer");

    fx.click(actions_button(1, 0))?;
    assert_eq!(menu(&mut fx)[2].1, "Turn on queueing");
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
    fx.keys("down down down enter")?;
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

/// The ephemeral fork `Open in side chat` makes of the subtract thread.
const SIDE: &str = "01a13de9-51de-7c4a-8000-51de0c4a7000";

fn fork_response(thread: Value) -> Value {
    json!({
        "thread": thread,
        "model": "gpt-6-astra",
        "reasoningEffort": "low",
        "approvalPolicy": "on-request",
        "approvalsReviewer": "user",
        "sandbox": {
            "type": "workspaceWrite", "writableRoots": [], "networkAccess": false,
            "excludeTmpdirEnvVar": false, "excludeSlashTmp": false,
        },
    })
}

/// The selected side chat tab's `(turn ids and statuses, composer label)`.
fn side_chat(fx: &mut Fx) -> Option<(Vec<(String, TurnStatus)>, &'static str)> {
    fx.read(|root, cx| {
        let Some(SidePanel::SideChat(key)) = root.side_panel() else {
            return None;
        };
        let (view, composer) = root.conversation(Some(key))?;
        let turns = view.read(cx).turns().iter();
        Some((
            turns.map(|turn| (turn.id.clone(), turn.status)).collect(),
            composer.read(cx).send_label(),
        ))
    })
}

/// The open chat's turn ids and the label of its composer's submit button.
fn primary(fx: &mut Fx) -> (Vec<String>, &'static str) {
    fx.read(|root, cx| {
        let turns = root.thread().unwrap().read(cx).turns().iter();
        (
            turns.map(|turn| turn.id.clone()).collect(),
            root.composer(cx).read(cx).send_label(),
        )
    })
}

fn turn(id: &str, status: &str, items: Value) -> Value {
    json!({ "id": id, "items": items, "status": status, "error": null })
}

/// Opens the subtract thread running with `rows` queued, types `draft` into its composer and
/// picks `Open in side chat` on row `index`.
fn open_side_chat(fx: &mut Fx, rows: &[&str], index: usize, draft: &str) -> Result<()> {
    open_running(fx)?;
    fx.wait("queued rows", |fx| queued(fx) == rows)?;
    fx.click((809., 750.))?;
    fx.type_text(draft)?;
    fx.click(actions_button(rows.len(), index))?;
    fx.keys("down down enter")
}

#[test]
fn open_in_side_chat_runs_the_queued_message_in_its_own_conversation() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let first = submission("a", json!([text("first")]));
    let second = submission("b", json!([text("What does subtract return?")]));
    let third = submission("c", json!([text("third")]));
    let later = submission("s1", json!([text("And for negatives?")]));
    let mut fork = recorded_thread(SUBTRACT)?;
    fork["id"] = SIDE.into();
    fork["turns"] = json!([]);
    let mut side_list = list(&[&later]);
    side_list[0]["msg"]["params"]["threadId"] = SIDE.into();

    let mut lines = script(None, queue_mode)?;
    lines.extend(exchange("thread/resume", json!({})));
    lines.extend(list(&[&first, &second, &third]));
    lines.extend(exchange("thread/queue/delete", json!({ "deleted": true })));
    lines.extend(exchange("thread/fork", fork_response(fork)));
    lines.extend(exchange("thread/inject_items", json!({})));
    lines.extend(exchange(
        "turn/start",
        json!({ "turn": turn("side-turn", "inProgress", json!([])) }),
    ));
    lines.extend(list(&[&first, &third]));
    lines.extend(exchange(
        "thread/queue/add",
        json!({ "queuedSubmission": later }),
    ));
    lines.extend(side_list);
    lines.extend(exchange("turn/interrupt", json!({})));
    lines.extend(exchange(
        "thread/unsubscribe",
        json!({ "status": "unsubscribed" }),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let rows = ["first", "What does subtract return?", "third"];
    open_side_chat(&mut fx, &rows, 1, "keep me")?;

    // The submission leaves the queue and starts the first turn of an ephemeral fork, shown
    // in a side chat tab whose composer takes the focus; the chat's draft stays.
    fx.wait("side chat tab", |fx| side_chat(fx).is_some())?;
    fx.wait("first side turn sent", |fx| {
        sent(fx, "turn/start").is_ok_and(|started| started.len() == 1)
    })?;
    fx.wait("row left the tray", |fx| queued(fx) == ["first", "third"])?;
    assert_eq!(
        sent(&mut fx, "thread/queue/delete")?[0]["params"],
        json!({ "threadId": SUBTRACT, "queuedSubmissionId": "b" })
    );
    let forked = sent(&mut fx, "thread/fork")?;
    assert_eq!(forked.len(), 1, "{forked:?}");
    let params = &forked[0]["params"];
    assert_eq!(
        [
            &params["threadId"],
            &params["ephemeral"],
            &params["threadSource"],
            &params["excludeTurns"]
        ],
        [&json!(SUBTRACT), &json!(true), &json!("user"), &json!(true)]
    );
    assert!(
        params["developerInstructions"]
            .as_str()
            .is_some_and(|text| {
                text.starts_with("You are in a side conversation, not the main thread.")
            })
    );
    let injected = sent(&mut fx, "thread/inject_items")?;
    assert_eq!(injected[0]["params"]["threadId"], SIDE);
    assert!(
        injected[0]["params"]["items"][0]["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("Side conversation boundary."))
    );
    assert_eq!(
        sent(&mut fx, "turn/start")?[0]["params"],
        json!({
            "threadId": SIDE,
            "input": second["input"],
            "clientUserMessageId": "b-client",
            "turnTrigger": "queued_side_chat",
        })
    );
    assert_eq!(fx.composer_text()?, "keep me");
    assert!(fx.read(|root, cx| {
        let SidePanel::SideChat(key) = root.side_panel().unwrap() else {
            return false;
        };
        let (_, composer) = root.conversation(Some(key)).unwrap();
        composer.read(cx).permission() == ui::composer::Permission::Ask
            && root.composer(cx).read(cx).permission() == ui::composer::Permission::Custom
    }));
    assert!(!fx.composer_focused()?);

    // Each conversation takes only its own thread's events.
    notify(
        &mut fx,
        "turn/started",
        json!({"threadId": SIDE, "turn": turn("side-turn", "inProgress", json!([]))}),
    );
    for item in [
        json!({"type": "userMessage", "id": "side-user", "content": second["input"]}),
        json!({"type": "agentMessage", "id": "side-answer", "text": "It returns a - b."}),
    ] {
        notify(
            &mut fx,
            "item/completed",
            json!({"threadId": SIDE, "turnId": "side-turn", "item": item}),
        );
    }
    fx.advance(std::time::Duration::from_secs(1))?;
    save(&mut fx, "side-chat-running")?;
    notify(
        &mut fx,
        "turn/completed",
        json!({"threadId": SUBTRACT, "turn": turn("queue-options-turn", "completed", json!([]))}),
    );
    save(&mut fx, "side-chat")?;
    let (side_turns, side_label) = side_chat(&mut fx).unwrap();
    assert_eq!(
        side_turns,
        [("side-turn".to_owned(), TurnStatus::InProgress)]
    );
    assert_eq!(side_label, "Stop");
    let (primary_turns, primary_label) = primary(&mut fx);
    assert!(!primary_turns.iter().any(|id| id == "side-turn"));
    assert_eq!(primary_label, "Send");

    // The side composer queues into the side chat and stops the side chat's turn.
    fx.type_text("And for negatives?")?;
    fx.keys("enter")?;
    fx.wait("side queue", |fx| {
        fx.read(|root, cx| {
            let Some(SidePanel::SideChat(key)) = root.side_panel() else {
                return false;
            };
            root.conversation(Some(key))
                .is_some_and(|(_, composer)| composer.read(cx).queued().len() == 1)
        })
    })?;
    assert_eq!(
        sent(&mut fx, "thread/queue/add")?[0]["params"]["threadId"],
        SIDE
    );
    assert_eq!(queued(&mut fx), ["first", "third"]);
    fx.keys("escape escape")?;
    fx.wait("side turn interrupted", |fx| {
        sent(fx, "turn/interrupt").is_ok_and(|stopped| stopped.len() == 1)
    })?;
    assert_eq!(
        sent(&mut fx, "turn/interrupt")?[0]["params"],
        json!({ "threadId": SIDE, "turnId": "side-turn" })
    );
    notify(
        &mut fx,
        "turn/completed",
        json!({"threadId": SIDE, "turn": turn("side-turn", "interrupted", json!([]))}),
    );

    // Closing the tab unloads the fork; the chat and its draft stay as they were.
    fx.action(Box::new(CloseSideTab(0)))?;
    fx.wait("fork unloaded", |fx| {
        sent(fx, "thread/unsubscribe").is_ok_and(|gone| gone.len() == 1)
    })?;
    assert_eq!(
        sent(&mut fx, "thread/unsubscribe")?[0]["params"],
        json!({ "threadId": SIDE })
    );
    assert!(fx.read(|root, _| root.side_tabs().is_empty()));
    assert_eq!(fx.thread_id().as_deref(), Some(SUBTRACT));
    assert_eq!(fx.composer_text()?, "keep me");
    let allowed = [
        "thread/read",
        // Opening the chat resumes it before any queue action.
        "thread/resume",
        "thread/queue/list",
        "thread/queue/delete",
        "thread/fork",
    ];
    let on_chat: Vec<Value> = fx
        .sent()?
        .into_iter()
        .filter(|m| m["params"]["threadId"] == SUBTRACT)
        .map(|m| m["method"].clone())
        .collect();
    assert!(
        on_chat
            .iter()
            .all(|m| allowed.contains(&m.as_str().unwrap_or_default())),
        "{on_chat:?}"
    );
    Ok(())
}

#[test]
fn a_side_chat_that_fails_to_open_puts_the_message_back_in_place() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let first = submission("a", json!([text("first")]));
    let second = submission("b", json!([text("second")]));
    let third = submission("c", json!([text("third")]));
    let again = submission("b2", json!([text("second")]));
    // No `thread/fork` in the script: the server refuses it.
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
    open_side_chat(&mut fx, &["first", "second", "third"], 1, "keep me")?;

    fx.wait("failure toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|t| t.as_ref() == "Failed to open side chat")
        })
    })?;
    fx.wait("back in place", |fx| {
        queued(fx) == ["first", "second", "third"]
    })?;
    save(&mut fx, "side-chat-failed")?;
    assert_eq!(sent(&mut fx, "thread/fork")?.len(), 1);
    assert_eq!(
        sent(&mut fx, "thread/queue/add")?[0]["params"],
        json!({ "threadId": SUBTRACT, "input": second["input"], "clientUserMessageId": "b-client" })
    );
    assert_eq!(
        sent(&mut fx, "thread/queue/reorder")?[0]["params"],
        json!({ "threadId": SUBTRACT, "queuedSubmissionIds": ["a", "b2", "c"] })
    );
    assert!(sent(&mut fx, "turn/start")?.is_empty());
    assert!(fx.read(|root, _| root.side_tabs().is_empty()));
    assert_eq!(fx.composer_text()?, "keep me");
    Ok(())
}

#[test]
fn failed_follow_up_can_open_a_side_chat_without_deleting_a_server_row() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    for fail_fork in [false, true] {
        let mut fork = recorded_thread(SUBTRACT)?;
        fork["id"] = SIDE.into();
        fork["turns"] = json!([]);
        let mut lines = script(None, queue_mode)?;
        lines.extend(exchange("thread/resume", json!({})));
        lines.extend(refusal("thread/queue/add", "Submission refused"));
        lines.extend(if fail_fork {
            refusal("thread/fork", "Fork refused")
        } else {
            exchange("thread/fork", fork_response(fork))
        });
        lines.extend(exchange("thread/inject_items", json!({})));
        lines.extend(exchange("turn/start", json!({})));
        let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
        open_running(&mut fx)?;
        fx.type_text("Retry in a side chat")?;
        fx.keys("enter")?;
        fx.wait("failed follow-up", |fx| {
            queued(fx) == ["Retry in a side chat"]
        })?;
        let original = sent(&mut fx, "thread/queue/add")?[0]["params"].clone();
        fx.type_text("Keep my draft")?;
        fx.click(actions_button(1, 0))?;
        fx.keys("down down enter")?;
        if fail_fork {
            fx.wait("fork failure", |fx| {
                fx.read(|root, cx| {
                    root.toasts(cx)
                        .iter()
                        .any(|toast| toast.as_ref() == "Failed to open side chat")
                })
            })?;
            assert_eq!(queued(&mut fx), ["Retry in a side chat"]);
        } else {
            fx.wait("side chat accepted the failed message", |fx| {
                side_chat(fx).is_some() && queued(fx).is_empty()
            })?;
            let started = sent(&mut fx, "turn/start")?;
            assert_eq!(started[0]["params"]["input"], original["input"]);
            assert_eq!(
                started[0]["params"]["clientUserMessageId"],
                original["clientUserMessageId"]
            );
        }
        assert_eq!(fx.composer_text()?, "Keep my draft");
        assert!(sent(&mut fx, "thread/queue/delete")?.is_empty());
        assert_eq!(sent(&mut fx, "thread/queue/add")?.len(), 1);
    }
    Ok(())
}
