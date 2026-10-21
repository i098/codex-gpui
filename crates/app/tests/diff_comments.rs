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
use ui::screens::diff::panel::comments::model_comments;

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
    let settings = app::host_settings::Settings::load(root.join(".proof-settings/host.json"))?;
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
fn gutter_comments_keep_ranges_and_support_edit_cancel_delete_and_send() -> Result<()> {
    let (_repo, mut fx) = prepare(false)?;
    fx.drag((697., 184.), (697., 292.), 6)?;
    fx.shot()?
        .save(output()?.join("comment-range-selected.png"))?;
    fx.click((671., 292.))?;
    fx.wait("range comment editor", |fx| draft(fx))?;
    fx.type_text("Check the complete range.")?;
    fx.reference_shot()?
        .save(output()?.join("comment-range-draft.png"))?;
    fx.keys("tab tab enter")?;
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
    fx.click((850., 365.))?;
    fx.wait("edit comment", |fx| draft(fx))?;
    fx.keys("cmd-a")?;
    fx.type_text("Discard this edit.")?;
    fx.keys("escape")?;
    ensure!(
        !draft(&fx) && comment_text(&fx).as_deref() == Some("Check the complete range."),
        "Cancel changed the saved comment"
    );
    fx.click((850., 365.))?;
    fx.keys("cmd-a")?;
    fx.type_text("Check overflow.")?;
    fx.keys("tab tab tab enter")?;
    fx.wait("updated comment", |fx| {
        comment_text(fx).as_deref() == Some("Check overflow.")
    })?;
    fx.reference_shot()?
        .save(output()?.join("comment-edited.png"))?;
    fx.click((850., 365.))?;
    fx.keys("tab enter")?;
    fx.wait("deleted comment", |fx| comment_text(fx).is_none())?;
    fx.reference_shot()?
        .save(output()?.join("comment-deleted.png"))?;
    fx.click((671., 205.))?;
    fx.type_text("Use checked arithmetic.")?;
    fx.keys("tab tab enter")?;
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
fn model_comment_dismissal_uses_confirmation_and_persists() -> Result<()> {
    let (repo, mut fx) = prepare(true)?;
    fx.reference_shot()?
        .save(output()?.join("model-comment.png"))?;
    fx.click((1195., 246.))?;
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
    fx.click((1195., 246.))?;
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
