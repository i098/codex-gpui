#![cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, SUBTRACT};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use ui::composer::dictation::{
    Command as DictationCommand, Dispatcher, Event as DictationEvent, Session,
};
use ui::screens::diff::panel::comments::model_comments;

type DictationControls = (
    std::sync::mpsc::Receiver<DictationCommand>,
    async_channel::Sender<DictationEvent>,
);

fn install_controlled_dictation(
    fx: &mut Fx,
) -> Result<std::sync::mpsc::Receiver<DictationControls>> {
    let (control, sessions) = std::sync::mpsc::channel();
    let dispatcher = Dispatcher::new(move || {
        let (commands, received) = std::sync::mpsc::channel();
        let (events, delivered) = async_channel::unbounded();
        control
            .send((received, events))
            .map_err(|error| error.to_string())?;
        Ok(Session {
            commands,
            events: delivered,
        })
    });
    fx.cx
        .update_window(fx.window, |_, _, cx| dispatcher.install(cx))?;
    Ok(sessions)
}

#[test]
fn proxy() {
    common::proxy_child();
}

fn output() -> Result<PathBuf> {
    let path = std::env::var_os("CGX_DIFF_REVIEW_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("cgx-diff-comments"));
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let result = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        result.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

fn prepare(model: bool) -> Result<(tempfile::TempDir, Fx)> {
    common::install_panic_hook();
    let repo = tempfile::tempdir_in(output()?)?;
    let root = repo.path();
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.name", "Comment proof"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    let content = (1..=10)
        .map(|line| format!("let value_{line} = {line};\n"))
        .collect::<String>();
    fs::write(root.join("file.rs"), &content)?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-qm", "before"])?;
    fs::write(
        root.join("file.rs"),
        content.replace("value_5 = 5", "value_5 = 50"),
    )?;
    let patch = git(root, &["diff", "--full-index", "--"])?;
    let mut recording = common::script(Some("sessions/subtract.jsonl"), |_| {})?;
    let oracle: Value = serde_json::from_str(include_str!("../../../tools/route1/oracle.json"))?;
    let resume = oracle
        .as_object()
        .context("Original oracle")?
        .iter()
        .find(|(key, value)| {
            key.starts_with("mcp local thread/resume ")
                && value["message"]["result"]["thread"]["id"] == SUBTRACT
        })
        .context("Original subtract resume")?
        .1["message"]["result"]
        .clone();
    recording.extend(common::exchange("thread/resume", resume));
    for row in &mut recording {
        if let Some(thread) = row.pointer_mut("/msg/result/thread") {
            thread["cwd"] = root.to_string_lossy().into_owned().into();
        }
    }
    let mut settings = app::host_settings::Settings::load(root.join(".proof-settings/host.json"))?;
    for (key, value) in common::recorded_host_settings()?.read_all().iter() {
        settings.write(key, value.clone())?;
    }
    if model {
        let comments = model_comments(
            "::code-comment{title=\"Check bounds\" body=\"The index can exceed the array.\" file=\"file.rs\" start=4 end=4 priority=1}",
            &root.to_string_lossy(),
            &[],
        );
        settings.write("diff_comments_from_model", json!({(SUBTRACT): comments}))?;
    }
    let mut fx = Fx::captured_with_host_settings("dark", (1280., 832.), recording, settings)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("alt-cmd-b")?;
    let turn = fx.read(|root, cx| {
        root.thread()
            .unwrap()
            .read(cx)
            .turns()
            .last()
            .unwrap()
            .id
            .clone()
    });
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            bridge::Event::Notification {
                method: "turn/diff/updated".into(),
                params: json!({"threadId":SUBTRACT,"turnId":turn,"diff":patch}),
            },
            cx,
        )
    });
    fx.reference_shot()?;
    Ok((repo, fx))
}

fn draft(fx: &Fx) -> bool {
    fx.read(|root, cx| {
        root.diff_panel()
            .is_some_and(|panel| panel.read(cx).comment_draft().is_some())
    })
}

fn comment_text(fx: &Fx) -> Option<String> {
    fx.read(|root, cx| {
        root.composer(cx)
            .read(cx)
            .diff_comments()
            .first()
            .map(|comment| comment.text())
    })
}

#[test]
fn gutter_button_restores_its_fill_after_hover() -> Result<()> {
    let (_repo, mut fx) = prepare(false)?;
    fx.click((704., 215.4))?;
    fx.wait("comment draft", |fx| draft(fx))?;
    fx.hover((800., 215.4))?;
    let normal = fx.shot()?.get_pixel(1390, 425).0;
    fx.hover((704., 215.4))?;
    let hovered = fx.shot()?.get_pixel(1390, 425).0;
    fx.hover((800., 215.4))?;
    let restored = fx.shot()?.get_pixel(1390, 425).0;

    assert_eq!(normal, [255, 255, 255, 255]);
    assert_eq!(hovered, [227, 227, 227, 255]);
    assert_eq!(restored, normal);
    Ok(())
}

#[test]
fn gutter_comments_keep_ranges_and_support_edit_cancel_delete_and_send() -> Result<()> {
    let (_repo, mut fx) = prepare(false)?;
    fx.drag((686., 194.4), (686., 302.4), 6)?;
    fx.shot()?
        .save(output()?.join("comment-range-selected.png"))?;
    fx.click((704., 302.4))?;
    fx.wait("range comment editor", |fx| draft(fx))?;
    fx.type_text("Check the complete range.")?;
    fx.reference_shot()?
        .save(output()?.join("comment-range-draft.png"))?;
    fx.keys("tab tab tab enter")?;
    fx.wait("saved comment", |fx| {
        comment_text(fx).as_deref() == Some("Check the complete range.")
    })?;
    fx.read(|root, cx| {
        let composer = root.composer(cx);
        let comment = &composer.read(cx).diff_comments()[0];
        assert_eq!(comment.position.start_line.unwrap().get(), 3);
        assert_eq!(comment.position.line.get(), 7);
    });
    fx.reference_shot()?
        .save(output()?.join("comment-saved.png"))?;
    fx.click((850., 375.4))?;
    fx.wait("edit comment", |fx| draft(fx))?;
    fx.keys("cmd-a")?;
    fx.type_text("Discard this edit.")?;
    fx.keys("escape")?;
    ensure!(
        !draft(&fx) && comment_text(&fx).as_deref() == Some("Check the complete range."),
        "Cancel changed the saved comment"
    );
    fx.click((850., 375.4))?;
    fx.keys("cmd-a")?;
    fx.type_text("Check overflow.")?;
    fx.keys("tab tab tab tab enter")?;
    fx.wait("updated comment", |fx| {
        comment_text(fx).as_deref() == Some("Check overflow.")
    })?;
    fx.reference_shot()?
        .save(output()?.join("comment-edited.png"))?;
    fx.click((850., 375.4))?;
    fx.keys("tab enter")?;
    fx.wait("deleted comment", |fx| comment_text(fx).is_none())?;
    fx.reference_shot()?
        .save(output()?.join("comment-deleted.png"))?;
    fx.click((704., 215.4))?;
    fx.type_text("Use checked arithmetic.")?;
    fx.keys("tab tab tab enter")?;
    fx.wait("new comment", |fx| comment_text(fx).is_some())?;
    fx.focus_composer()?;
    fx.type_text("Apply this review.")?;
    fx.keys("enter")?;
    fx.wait("comment request", |fx| {
        fx.sent()
            .is_ok_and(|sent| sent.iter().any(|request| request["method"] == "turn/start"))
    })?;
    let requests = fx.sent()?;
    let request = requests
        .iter()
        .find(|request| request["method"] == "turn/start")
        .unwrap();
    let text = request["params"]["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|input| input["text"].as_str())
        .collect::<String>();
    ensure!(
        text.contains("# Diff comments:")
            && text.contains("Side: R")
            && text.contains("Use checked arithmetic.")
            && text.ends_with("Apply this review."),
        "The turn request lost comment context: {text}"
    );
    Ok(())
}

#[test]
fn comment_attachment_opens_the_file_and_removes_persisted_comments() -> Result<()> {
    let (repo, mut fx) = prepare(false)?;
    fx.drag((686., 194.4), (686., 302.4), 6)?;
    fx.click((704., 302.4))?;
    fx.wait("comment editor", |fx| draft(fx))?;
    fx.type_text("Check this range.")?;
    fx.keys("tab tab tab enter")?;
    fx.wait("saved attachment", |fx| {
        comment_text(fx).as_deref() == Some("Check this range.")
    })?;
    let settings =
        app::host_settings::Settings::load(repo.path().join(".proof-settings/host.json"))?;
    ensure!(
        settings.read_all()["diff_comments"][SUBTRACT][0]["content"][0]["text"]
            == "Check this range."
    );
    fx.click((420., 701.7))?;
    fx.hover((800., 12.))?;
    fx.advance(std::time::Duration::from_millis(150))?;
    let composer = fx.composer();
    let (open, _, trigger_focused) = fx.cx.update_window(fx.window, |_, window, cx| {
        composer.read(cx).menu_state(window)
    })?;
    ensure!(
        !open && trigger_focused,
        "Closing the preview lost keyboard focus"
    );
    fx.click((420., 701.7))?;
    fx.keys("down enter")?;
    let expected = fs::read_to_string(repo.path().join("file.rs"))?;
    fx.wait("comment source file", |fx| {
        fx.read(|root, cx| {
            root.file_view().is_some_and(|file| {
                let file = file.read(cx);
                file.path.ends_with("/file.rs")
                    && file.line == Some(7)
                    && file
                        .contents
                        .as_ref()
                        .is_some_and(|contents| contents.as_deref() == Ok(expected.as_str()))
            })
        })
    })?;
    fx.action(Box::new(ui::screens::diff::panel::CloseSideTab(1)))?;
    fx.click((466., 701.7))?;
    fx.wait("removed attachment", |fx| comment_text(fx).is_none())?;
    let settings =
        app::host_settings::Settings::load(repo.path().join(".proof-settings/host.json"))?;
    ensure!(
        settings
            .read_all()
            .get("diff_comments")
            .and_then(|comments| comments.get(SUBTRACT))
            .is_none(),
        "Removed comments returned after reloading their storage"
    );
    Ok(())
}

#[test]
fn model_comment_dismissal_uses_confirmation_and_persists() -> Result<()> {
    let (repo, mut fx) = prepare(true)?;
    fx.reference_shot()?
        .save(output()?.join("model-comment.png"))?;
    fx.click((1195., 256.4))?;
    fx.wait("dismiss confirmation", |fx| {
        fx.read(|root, _| root.diff_comment_confirmation().is_some())
    })?;
    fx.reference_shot()?
        .save(output()?.join("model-comment-confirmation.png"))?;
    fx.keys("escape")?;
    ensure!(
        fx.read(|root, _| root.diff_comment_confirmation().is_none()),
        "Escape did not cancel dismissal"
    );
    fx.click((1195., 256.4))?;
    fx.wait("dismiss confirmation again", |fx| {
        fx.read(|root, _| root.diff_comment_confirmation().is_some())
    })?;
    fx.keys("tab enter")?;
    fx.wait("persisted dismissal", |fx| {
        fx.read(|_, cx| {
            cx.global::<app::host_settings::Settings>()
                .read_all()
                .iter()
                .any(|(key, value)| {
                    key.starts_with(&format!("dismissed-model-diff-comments-v3:{SUBTRACT}:"))
                        && value == true
                })
        })
    })?;
    ensure!(
        fx.read(|root, _| root.diff_comment_confirmation().is_none()),
        "The confirmation stayed open"
    );
    let persisted =
        app::host_settings::Settings::load(repo.path().join(".proof-settings/host.json"))?;
    ensure!(
        persisted.read_all().iter().any(|(key, value)| {
            key.starts_with(&format!("dismissed-model-diff-comments-v3:{SUBTRACT}:"))
                && value.as_bool() == Some(true)
        }),
        "The dismissal did not survive reloading the settings file"
    );
    fx.reference_shot()?
        .save(output()?.join("model-comment-dismissed.png"))?;
    Ok(())
}

#[test]
fn controlled_dictation_targets_the_comment_and_cancels_on_close() -> Result<()> {
    common::install_panic_hook();
    let mut fx = Fx::new()?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.focus_composer()?;
    fx.type_text("Keep this draft.")?;
    fx.keys("alt-cmd-b")?;
    fx.hover((814., 239.6))?;
    fx.click((704., 239.6))?;
    fx.wait("comment editor", |fx| draft(fx))?;
    fx.type_text("old")?;
    fx.keys("cmd-a")?;
    let editor = fx.read(|root, cx| {
        root.diff_panel()
            .unwrap()
            .read(cx)
            .comment_draft()
            .unwrap()
            .clone()
    });
    let sessions = install_controlled_dictation(&mut fx)?;
    fx.click((1099., 357.))?;
    let (commands, events) = sessions.recv_timeout(std::time::Duration::from_secs(2))?;
    events.send_blocking(DictationEvent::Recording)?;
    fx.wait("controlled recording", |fx| {
        fx.read(|_, cx| editor.read(cx).dictation_label() == "Stop dictation")
    })?;
    fx.click((1099., 357.))?;
    ensure!(matches!(
        commands.recv_timeout(std::time::Duration::from_secs(2)),
        Ok(DictationCommand::Finish)
    ));
    events.send_blocking(DictationEvent::Finished(Ok("replacement".into())))?;
    fx.wait("comment transcript", |fx| {
        fx.read(|_, cx| editor.read(cx).field_text() == "replacement")
    })?;
    ensure!(fx.composer_text()? == "Keep this draft.");
    fx.click((1099., 357.))?;
    let (commands, events) = sessions.recv_timeout(std::time::Duration::from_secs(2))?;
    events.send_blocking(DictationEvent::Recording)?;
    fx.wait("second controlled recording", |fx| {
        fx.read(|_, cx| editor.read(cx).dictation_label() == "Stop dictation")
    })?;
    fx.click((1221., 357.))?;
    ensure!(draft(&fx), "Comment saved before the pending transcript");
    ensure!(matches!(
        commands.recv_timeout(std::time::Duration::from_secs(2)),
        Ok(DictationCommand::Finish)
    ));
    fx.click((1148., 357.))?;
    ensure!(!draft(&fx));
    ensure!(matches!(
        commands.recv_timeout(std::time::Duration::from_secs(2)),
        Ok(DictationCommand::Cancel)
    ));
    let _ = events.try_send(DictationEvent::Finished(Ok("late result".into())));
    fx.advance(std::time::Duration::from_millis(100))?;
    ensure!(fx.read(|_, cx| editor.read(cx).field_text() == "replacement"));
    ensure!(
        comment_text(&fx).is_none(),
        "Cancelled dictation saved a comment"
    );
    ensure!(fx.composer_text()? == "Keep this draft.");
    println!(
        "Controlled comment dictation passed without microphone capture or a permission request."
    );
    Ok(())
}

#[test]
fn comment_save_waits_for_dictation_and_keeps_failures_editable() -> Result<()> {
    let (repo, mut fx) = prepare(false)?;
    fx.drag((686., 194.4), (686., 302.4), 6)?;
    fx.click((704., 302.4))?;
    fx.wait("comment editor", |fx| draft(fx))?;
    fx.type_text("keep ")?;
    let editor = fx.read(|root, cx| {
        root.diff_panel()
            .unwrap()
            .read(cx)
            .comment_draft()
            .unwrap()
            .clone()
    });
    let sessions = install_controlled_dictation(&mut fx)?;
    fx.keys("tab enter")?;
    let (commands, events) = sessions.recv_timeout(std::time::Duration::from_secs(2))?;
    events.send_blocking(DictationEvent::Recording)?;
    fx.wait("controlled recording", |fx| {
        fx.read(|_, cx| editor.read(cx).dictation_label() == "Stop dictation")
    })?;
    fx.keys("tab tab tab enter")?;
    ensure!(draft(&fx), "Comment saved before the transcript arrived");
    ensure!(comment_text(&fx).is_none());
    ensure!(matches!(
        commands.recv_timeout(std::time::Duration::from_secs(2)),
        Ok(DictationCommand::Finish)
    ));
    events.send_blocking(DictationEvent::Finished(Ok("dictated".into())))?;
    fx.wait("saved transcribed comment", |fx| {
        !draft(fx) && comment_text(fx).as_deref() == Some("keep dictated")
    })?;
    let settings =
        app::host_settings::Settings::load(repo.path().join(".proof-settings/host.json"))?;
    ensure!(
        settings.read_all()["diff_comments"][SUBTRACT][0]["content"][0]["text"] == "keep dictated"
    );

    fx.click((850., 375.4))?;
    fx.wait("editing saved comment", |fx| draft(fx))?;
    let editor = fx.read(|root, cx| {
        root.diff_panel()
            .unwrap()
            .read(cx)
            .comment_draft()
            .unwrap()
            .clone()
    });
    fx.keys("cmd-a")?;
    fx.type_text("retry ")?;
    fx.keys("tab tab enter")?;
    let (commands, events) = sessions.recv_timeout(std::time::Duration::from_secs(2))?;
    events.send_blocking(DictationEvent::Recording)?;
    fx.wait("controlled retry recording", |fx| {
        fx.read(|_, cx| editor.read(cx).dictation_label() == "Stop dictation")
    })?;
    fx.keys("tab tab tab tab enter")?;
    ensure!(draft(&fx), "Save closed an unfinished transcription");
    ensure!(matches!(
        commands.recv_timeout(std::time::Duration::from_secs(2)),
        Ok(DictationCommand::Finish)
    ));
    events.send_blocking(DictationEvent::Finished(Err(
        "Controlled transcription failure".into(),
    )))?;
    fx.wait("transcription failure", |fx| {
        fx.read(|_, cx| editor.read(cx).dictation_error().is_some())
    })?;
    ensure!(draft(&fx));
    ensure!(comment_text(&fx).as_deref() == Some("keep dictated"));
    fx.click((850., 375.4))?;
    fx.keys("cmd-a")?;
    fx.type_text("retry safe")?;
    fx.keys("tab tab tab tab enter")?;
    fx.wait("saved corrected text", |fx| {
        !draft(fx) && comment_text(fx).as_deref() == Some("retry safe")
    })?;
    Ok(())
}
