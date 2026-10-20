mod common;

use anyhow::Result;
use common::{Fx, GREETING, SUBTRACT, exchange, proxy_child, recorded_thread, script};
use gpui::{ClipboardItem, ExternalPaths, FileDropEvent, PlatformInput, point, px};
use serde_json::{Value, json};
use std::{cell::RefCell, fs, rc::Rc, time::Duration};
use ui::composer::{Composer, Submission};

#[test]
fn proxy() {
    proxy_child();
}

fn draft_script() -> Result<Vec<Value>> {
    let mut lines = script(None, |_| {})?;
    for id in [GREETING, SUBTRACT] {
        let mut read = exchange("thread/read", json!({"thread": recorded_thread(id)?}));
        read[0]["msg"]["params"] = json!({"threadId": id, "includeTurns": true});
        lines.extend(read);
    }
    Ok(lines)
}

#[test]
fn switching_threads_keeps_each_draft_and_its_attachment() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let temp = tempfile::tempdir()?;
    let notes = temp.path().join("notes.txt");
    fs::write(&notes, "owned draft attachment\n")?;
    let mut fx = Fx::scripted(draft_script()?)?;
    fx.click(fx.at("home", "Change the greeting")?)?;
    fx.wait_thread(GREETING)?;
    fx.shot()?;
    fx.type_text("Draft A - owned composer proof")?;
    if let Some(proof) = std::env::var_os("CGX_COMPOSER_PROOF_DIR") {
        fs::create_dir_all(&proof)?;
        fx.shot()?
            .save(std::path::Path::new(&proof).join("native-draft-a.png"))?;
    }
    let position = point(px(800.), px(740.));
    fx.input(PlatformInput::FileDrop(FileDropEvent::Entered {
        position,
        paths: ExternalPaths(std::iter::once(notes.clone()).collect()),
    }))?;
    fx.input(PlatformInput::FileDrop(FileDropEvent::Pending { position }))?;
    fx.shot()?;
    fx.input(PlatformInput::FileDrop(FileDropEvent::Submit { position }))?;
    fx.input(PlatformInput::FileDrop(FileDropEvent::Ended))?;
    fx.shot()?;

    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    fx.shot()?;
    if let Some(proof) = std::env::var_os("CGX_COMPOSER_PROOF_DIR") {
        fx.hover((640., 22.))?;
        fx.cx.update(|cx| Composer::set_caret_transparent(cx, true));
        fx.shot()?
            .save(std::path::Path::new(&proof).join("native-empty-subtract.png"))?;
        for width in [767., 600.] {
            fx.resize(width, 832.)?;
            fx.shot()?
                .save(std::path::Path::new(&proof).join(format!("native-thread-{width}.png")))?;
        }
        fx.resize(1280., 832.)?;
        fx.shot()?;
        fx.cx
            .update(|cx| Composer::set_caret_transparent(cx, false));
    }
    assert_eq!(fx.composer_text()?, "");
    fx.type_text("Draft B - separate text")?;
    fx.click(fx.at("home", "Change the greeting")?)?;
    fx.wait_thread(GREETING)?;
    fx.shot()?;
    assert_eq!(fx.composer_text()?, "Draft A - owned composer proof");
    let submitted = Rc::new(RefCell::new(None::<Submission>));
    let capture = submitted.clone();
    fx.composer().update(&mut fx.cx, |composer, _| {
        composer.on_submit(move |submission, _, _| {
            *capture.borrow_mut() = Some(submission.clone());
        });
    });
    fx.keys("enter")?;
    let submission = submitted.borrow_mut().take().expect("draft submission");
    assert_eq!(submission.text, "Draft A - owned composer proof");
    assert_eq!(submission.attachments, vec![notes.clone()]);
    assert!(notes.exists());
    fx.type_text("After removal")?;
    fx.input(PlatformInput::FileDrop(FileDropEvent::Entered {
        position,
        paths: ExternalPaths(std::iter::once(notes.clone()).collect()),
    }))?;
    fx.input(PlatformInput::FileDrop(FileDropEvent::Pending { position }))?;
    fx.shot()?;
    fx.input(PlatformInput::FileDrop(FileDropEvent::Submit { position }))?;
    fx.input(PlatformInput::FileDrop(FileDropEvent::Ended))?;
    let attached = fx.shot()?;
    if let Some(proof) = std::env::var_os("CGX_COMPOSER_PROOF_DIR") {
        attached.save(std::path::Path::new(&proof).join("native-attached-a.png"))?;
    }
    fx.click((526., 708.))?;
    fx.click((1155., 790.))?;
    let removed = submitted
        .borrow_mut()
        .take()
        .expect("submission after removal");
    assert_eq!(removed.text, "After removal");
    assert!(removed.attachments.is_empty());
    assert!(notes.exists());
    fx.click(fx.at("home", "Add a subtract")?)?;
    fx.wait_thread(SUBTRACT)?;
    fx.shot()?;
    assert_eq!(fx.composer_text()?, "Draft B - separate text");
    fx.keys("cmd-z")?;
    assert_eq!(fx.composer_text()?, "");
    Ok(())
}

#[test]
fn refused_send_restores_the_draft_in_its_own_thread() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    for method in ["thread/resume", "turn/start"] {
        let mut lines = draft_script()?;
        if method == "turn/start" {
            lines.extend(exchange("thread/resume", json!({})));
        }
        let mut refused = exchange(method, json!({}));
        refused[1]["msg"].as_object_mut().unwrap().remove("result");
        refused[1]["msg"]["error"] = json!({"code": -32000, "message": "owned send refusal"});
        lines.extend(refused);
        let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
        fx.click(fx.at("home", "Change the greeting")?)?;
        fx.wait_thread(GREETING)?;
        fx.shot()?;
        fx.type_text("Keep this refused draft in A")?;
        fx.keys("enter")?;
        fx.click(fx.at("home", "Add a subtract")?)?;
        fx.wait_thread(SUBTRACT)?;
        fx.wait("send refusal", |fx| {
            fx.read(|root, cx| {
                root.toasts(cx)
                    .iter()
                    .any(|toast| toast.as_ref() == "Error submitting message")
            })
        })?;
        assert_eq!(fx.composer_text()?, "", "{method} must not restore A in B");
        fx.click(fx.at("home", "Change the greeting")?)?;
        fx.wait_thread(GREETING)?;
        assert_eq!(fx.composer_text()?, "Keep this refused draft in A");
    }
    Ok(())
}

#[test]
fn pending_paste_keeps_its_submission_and_failure_in_the_source_thread() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    for outcome in ["saved", "failed", "cancelled"] {
        let temp = tempfile::tempdir()?;
        let release = temp.path().join("release");
        let refuse = |method: &str, message: &str| {
            let mut response = exchange(method, json!({}));
            response[1]["msg"].as_object_mut().unwrap().remove("result");
            response[1]["msg"]["error"] = json!({"code":-32000,"message":message});
            response
        };
        let mut lines = draft_script()?;
        lines.extend(refuse("fs/readFile", "ENOENT: owned registry is absent"));
        if outcome == "failed" {
            lines.extend(refuse(
                "fs/createDirectory",
                "ENOTDIR: owned attachment root is a file",
            ));
        } else {
            lines.extend(exchange("fs/createDirectory", json!({})));
        }
        for _ in 0..2 {
            lines.extend(exchange("fs/writeFile", json!({})));
        }
        let mut read = exchange("thread/read", json!({"thread":recorded_thread(SUBTRACT)?}));
        read[0]["msg"]["params"] = json!({"threadId":SUBTRACT,"includeTurns":true});
        lines.extend(read);
        lines.extend(exchange("thread/resume", json!({})));
        lines.extend(refuse("turn/start", "owned submission refusal"));
        lines.insert(
            0,
            json!({"dir":"meta","msg":{},"hold":{"method":"fs/createDirectory","release":release}}),
        );
        let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
        fx.click(fx.at("home", "Add a subtract")?)?;
        fx.wait_thread(SUBTRACT)?;
        fx.shot()?;
        fx.type_text("Owned boundary draft")?;
        if std::env::var_os("CGX_COMPOSER_PROOF_DIR").is_some() {
            fx.cx.update(|cx| Composer::set_caret_transparent(cx, true));
        }
        fx.cx.update(|cx| {
            cx.write_to_clipboard(ClipboardItem::new_string("owned paste proof\n".repeat(300)))
        });
        fx.keys("cmd-v")?;
        fx.wait("owned pending filesystem request", |fx| {
            fx.sent().is_ok_and(|requests| {
                requests
                    .iter()
                    .any(|request| request["method"] == "fs/createDirectory")
            })
        })?;
        fx.hover((1155., 790.))?;
        fx.advance(Duration::from_millis(350))?;
        let pending = fx.shot()?;
        assert!(fx.read(|_, cx| ui::tooltip::shown(cx).is_none()));
        fx.keys("enter")?;
        assert!(fx.read(|root, _| root.dialog().is_none()));
        assert_eq!(fx.composer_text()?, "Owned boundary draft");
        assert!(
            !fx.sent()?
                .iter()
                .any(|request| request["method"] == "turn/start")
        );
        if let Some(proof) = std::env::var_os("CGX_COMPOSER_PROOF_DIR") {
            fs::create_dir_all(&proof)?;
            pending.save(
                std::path::Path::new(&proof).join(format!("native-paste-pending-{outcome}.png")),
            )?;
        }
        if outcome == "cancelled" {
            fx.click((664., 678.))?;
            fx.keys("enter")?;
        }
        fx.click(fx.at("home", "Change the greeting")?)?;
        fx.wait_thread(GREETING)?;
        fx.shot()?;
        fx.type_text("Other draft stays")?;
        fs::write(&release, "release")?;
        let expected = if outcome == "failed" {
            "Large pasted text could not be attached"
        } else {
            "Error submitting message"
        };
        fx.wait(&format!("{outcome} source-thread outcome"), |fx| {
            fx.read(|root, cx| {
                root.toasts(cx)
                    .iter()
                    .any(|toast| toast.as_ref() == expected)
            })
        })?;
        assert_eq!(fx.composer_text()?, "Other draft stays");
        let turns: Vec<_> = fx
            .sent()?
            .into_iter()
            .filter(|request| request["method"] == "turn/start")
            .collect();
        if outcome == "failed" {
            assert!(turns.is_empty());
        } else {
            assert_eq!(turns.len(), 1);
            assert_eq!(turns[0]["params"]["threadId"], SUBTRACT);
            let sent = turns[0]["params"]["input"][0]["text"].as_str().unwrap();
            if outcome == "saved" {
                assert!(
                    sent.starts_with("Owned boundary draft\n")
                        && sent.ends_with("/Pasted text.txt")
                );
            } else {
                assert_eq!(sent, "Owned boundary draft");
            }
        }
        fx.click(fx.at("home", "Add a subtract")?)?;
        fx.wait_thread(SUBTRACT)?;
        assert_eq!(fx.composer_text()?, "Owned boundary draft");
        fx.hover((1155., 790.))?;
        fx.advance(Duration::from_millis(350))?;
        let result = fx.shot()?;
        if let Some(proof) = std::env::var_os("CGX_COMPOSER_PROOF_DIR") {
            result
                .save(std::path::Path::new(&proof).join(format!("native-paste-{outcome}.png")))?;
        }
    }
    Ok(())
}

#[test]
fn review_overlapping_refusals_restore_each_submissions_files() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let temp = tempfile::tempdir()?;
    let files = [
        temp.path().join("first.txt"),
        temp.path().join("second.txt"),
    ];
    for file in &files {
        fs::write(file, "owned overlap attachment")?;
    }
    let release = temp.path().join("release");
    let mut lines = draft_script()?;
    lines.extend(exchange("thread/resume", json!({})));
    for _ in 0..2 {
        let mut refused = exchange("turn/start", json!({}));
        refused[1]["msg"].as_object_mut().unwrap().remove("result");
        refused[1]["msg"]["error"] = json!({"code":-32000,"message":"owned overlap refusal"});
        lines.extend(refused);
    }
    lines.insert(
        0,
        json!({"dir":"meta","msg":{},"hold":{"method":"turn/start","release":release}}),
    );
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.click(fx.at("home", "Change the greeting")?)?;
    fx.wait_thread(GREETING)?;
    fx.shot()?;
    for (index, file) in files.iter().enumerate() {
        fx.type_text(if index == 0 {
            "first request"
        } else {
            "second request"
        })?;
        let position = point(px(800.), px(740.));
        fx.input(PlatformInput::FileDrop(FileDropEvent::Entered {
            position,
            paths: ExternalPaths(std::iter::once(file.clone()).collect()),
        }))?;
        fx.input(PlatformInput::FileDrop(FileDropEvent::Pending { position }))?;
        fx.shot()?;
        fx.input(PlatformInput::FileDrop(FileDropEvent::Submit { position }))?;
        fx.input(PlatformInput::FileDrop(FileDropEvent::Ended))?;
        fx.shot()?;
        fx.keys("enter")?;
        fx.wait("overlapping request", |fx| {
            fx.sent().is_ok_and(|requests| {
                requests
                    .iter()
                    .filter(|request| request["method"] == "turn/start")
                    .count()
                    == index + 1
            })
        })?;
    }
    fs::write(release, "release")?;
    fx.wait("both drafts restored", |fx| {
        fx.composer_text()
            .is_ok_and(|text| text.contains("first request") && text.contains("second request"))
    })?;
    let retried = Rc::new(RefCell::new(None::<Submission>));
    let capture = retried.clone();
    fx.composer().update(&mut fx.cx, |composer, _| {
        composer
            .on_submit(move |submission, _, _| *capture.borrow_mut() = Some(submission.clone()));
    });
    fx.keys("enter")?;
    let mut got = retried.borrow_mut().take().expect("retry").attachments;
    got.sort();
    assert_eq!(got.as_slice(), files.as_slice());
    assert!(files.iter().all(|file| file.exists()));
    Ok(())
}

#[test]
fn review_queue_edits_keep_hidden_input_and_position_per_thread() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let temp = tempfile::tempdir()?;
    let text = |value: &str| json!({"type":"text","text":value,"text_elements":[]});
    let row = |id: &str, input: Value| json!({"id":id,"clientUserMessageId":format!("{id}-client"),"input":input});
    let input_a = json!([text("queued A"), {"type":"skill","name":"skill-a","path":temp.path().join("a.md")}]);
    let input_b = json!([text("queued B"), {"type":"skill","name":"skill-b","path":temp.path().join("b.md")}]);
    let edit_a = row("edit-a", input_a.clone());
    let edit_b = row("edit-b", input_b.clone());
    let tail_a = row("tail-a", json!([text("tail A")]));
    let tail_b = row("tail-b", json!([text("tail B")]));
    let again_a = row("again-a", input_a.clone());
    let again_b = row("again-b", input_b.clone());
    let list = |id: &str, rows: Vec<Value>| {
        let mut result = exchange("thread/queue/list", json!({"data":rows,"nextCursor":null}));
        result[0]["msg"]["params"] = json!({"threadId":id,"cursor":null});
        result
    };
    let mut lines = draft_script()?;
    for line in &mut lines {
        if let Some(desktop) = line.pointer_mut("/msg/result/config/desktop") {
            desktop["followUpQueueMode"] = "queue".into();
        }
        if let Some(thread) = line.pointer_mut("/msg/result/thread")
            && thread["id"]
                .as_str()
                .is_some_and(|id| id == GREETING || id == SUBTRACT)
            && let Some(turn) = thread["turns"]
                .as_array_mut()
                .and_then(|turns| turns.last_mut())
        {
            turn["status"] = "inProgress".into();
        }
    }
    lines.extend(list(GREETING, vec![edit_a, tail_a.clone()]));
    lines.extend(list(GREETING, vec![tail_a.clone()]));
    lines.extend(list(GREETING, vec![tail_a.clone()]));
    lines.extend(list(GREETING, vec![tail_a.clone(), again_a.clone()]));
    lines.extend(list(GREETING, vec![again_a.clone(), tail_a]));
    lines.extend(list(SUBTRACT, vec![edit_b, tail_b.clone()]));
    lines.extend(list(SUBTRACT, vec![tail_b.clone()]));
    lines.extend(list(SUBTRACT, vec![tail_b.clone()]));
    lines.extend(list(SUBTRACT, vec![tail_b.clone(), again_b.clone()]));
    lines.extend(list(SUBTRACT, vec![again_b.clone(), tail_b]));
    for queued in [again_a, again_b] {
        lines.extend(exchange("thread/resume", json!({})));
        lines.extend(exchange("thread/queue/delete", json!({"deleted":true})));
        lines.extend(exchange(
            "thread/queue/add",
            json!({"queuedSubmission":queued}),
        ));
        lines.extend(exchange("thread/queue/reorder", json!({})));
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    for (title, id, draft) in [
        ("Change the greeting", GREETING, "queued A"),
        ("Add a subtract", SUBTRACT, "queued B"),
    ] {
        fx.click(fx.at("home", title)?)?;
        fx.wait_thread(id)?;
        fx.wait("editable queued row", |fx| {
            fx.composer()
                .read_with(&fx.cx, |composer, _| composer.queued().len() == 2)
        })?;
        fx.shot()?;
        fx.click((1140., 665.))?;
        fx.shot()?;
        fx.keys("down enter")?;
        fx.wait("queued draft restored", |fx| {
            fx.composer_text().is_ok_and(|text| text == draft)
        })?;
        fx.wait("edited row removed", |fx| {
            fx.composer()
                .read_with(&fx.cx, |composer, _| composer.queued().len() == 1)
        })?;
    }
    for (index, (title, id, expected_input, expected_order)) in [
        (
            "Change the greeting",
            GREETING,
            input_a,
            json!(["again-a", "tail-a"]),
        ),
        (
            "Add a subtract",
            SUBTRACT,
            input_b,
            json!(["again-b", "tail-b"]),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        fx.click(fx.at("home", title)?)?;
        fx.wait_thread(id)?;
        fx.shot()?;
        fx.keys("enter")?;
        fx.wait("edited draft queued again", |fx| {
            fx.sent().is_ok_and(|requests| {
                requests
                    .iter()
                    .filter(|request| request["method"] == "thread/queue/add")
                    .count()
                    == index + 1
            })
        })?;
        let added: Vec<_> = fx
            .sent()?
            .into_iter()
            .filter(|request| request["method"] == "thread/queue/add")
            .collect();
        assert_eq!(added[index]["params"]["threadId"], id);
        assert_eq!(added[index]["params"]["input"], expected_input);
        fx.wait("original queue position restored", |fx| {
            fx.sent().is_ok_and(|requests| {
                requests
                    .iter()
                    .filter(|request| request["method"] == "thread/queue/reorder")
                    .count()
                    == index + 1
            })
        })?;
        let reordered: Vec<_> = fx
            .sent()?
            .into_iter()
            .filter(|request| request["method"] == "thread/queue/reorder")
            .collect();
        assert_eq!(reordered[index]["params"]["threadId"], id);
        assert_eq!(
            reordered[index]["params"]["queuedSubmissionIds"],
            expected_order
        );
    }
    Ok(())
}
