#![cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use bridge::Event;
use common::{Fx, SUBTRACT};
use serde_json::json;
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let result = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        result.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(String::from_utf8(result.stdout)?)
}

fn choose(fx: &mut Fx, label: &str) -> Result<()> {
    let index = fx
        .read(|root, cx| {
            root.diff_panel()?
                .read(cx)
                .menu_items()
                .position(|(text, enabled)| text == label && enabled)
        })
        .context("missing enabled diff option")?;
    fx.keys("home")?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")
}

#[test]
fn word_diff_toggle_and_nontext_files_use_native_controls() -> Result<()> {
    common::install_panic_hook();
    let fallback = tempfile::tempdir()?;
    let out = std::env::var_os("CGX_DIFF_REVIEW_EVIDENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| fallback.path().to_owned());
    fs::create_dir_all(&out)?;
    let repo = tempfile::tempdir_in(&out)?;
    git(repo.path(), &["init", "-q"])?;
    git(repo.path(), &["config", "user.name", "Diff proof"])?;
    git(
        repo.path(),
        &["config", "user.email", "proof@example.invalid"],
    )?;
    fs::write(
        repo.path().join("code.rs"),
        "fn value() { let first = 1; let second = 2; }\n",
    )?;
    fs::write(
        repo.path().join("old.txt"),
        "rename without content changes\n",
    )?;
    fs::write(repo.path().join("binary.dat"), [0, 1, 2, 3])?;
    git(repo.path(), &["add", "."])?;
    git(repo.path(), &["commit", "-qm", "base"])?;
    fs::write(
        repo.path().join("code.rs"),
        "fn value() { let first = 3; let second = 4; }\n",
    )?;
    fs::write(repo.path().join("binary.dat"), [0, 9, 8, 7])?;
    fs::write(repo.path().join("empty.txt"), "")?;
    git(repo.path(), &["mv", "old.txt", "new.txt"])?;
    git(repo.path(), &["add", "."])?;
    let patch = git(
        repo.path(),
        &["diff", "--binary", "--find-renames", "HEAD", "--"],
    )?;
    fs::write(out.join("source.patch"), &patch)?;

    let mut fx = Fx::new()?;
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
            Event::Notification {
                method: "turn/diff/updated".into(),
                params: json!({"threadId": SUBTRACT, "turnId": turn, "diff": patch}),
            },
            cx,
        )
    });
    fx.cx.run_until_parked();
    let files = fx.read(|root, cx| {
        root.diff_panel()
            .unwrap()
            .read(cx)
            .files
            .iter()
            .map(|file| file.name.to_string())
            .collect::<Vec<_>>()
    });
    ensure!(
        files == ["binary.dat", "code.rs", "empty.txt", "new.txt"],
        "nontext files disappeared: {files:?}"
    );
    let totals = fx.read(|root, cx| {
        root.diff_panel()
            .unwrap()
            .read(cx)
            .files
            .iter()
            .map(|file| (file.added, file.removed))
            .collect::<Vec<_>>()
    });
    fx.reference_shot()?.save(out.join("word-diffs-on.png"))?;
    fx.click((1192., 68.))?;
    choose(&mut fx, "Word diffs")?;
    ensure!(
        !fx.read(|root, cx| root.diff_panel().unwrap().read(cx).shows_word_diffs()),
        "word diffs stayed enabled"
    );
    ensure!(
        totals
            == fx.read(|root, cx| root
                .diff_panel()
                .unwrap()
                .read(cx)
                .files
                .iter()
                .map(|file| (file.added, file.removed))
                .collect::<Vec<_>>()),
        "display option changed diff totals"
    );
    fx.reference_shot()?.save(out.join("word-diffs-off.png"))?;
    fx.click((1192., 68.))?;
    choose(&mut fx, "Word diffs")?;
    ensure!(
        fx.read(|root, cx| root.diff_panel().unwrap().read(cx).shows_word_diffs()),
        "word diffs did not re-enable"
    );
    println!("native diff toggle and nontext files: {}", out.display());
    Ok(())
}

#[test]
fn file_jump_sticky_header_and_native_tab_keep_the_selected_file() -> Result<()> {
    common::install_panic_hook();
    let fallback = tempfile::tempdir()?;
    let out = std::env::var_os("CGX_DIFF_REVIEW_EVIDENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| fallback.path().to_owned());
    fs::create_dir_all(&out)?;
    let repo = tempfile::tempdir_in(&out)?;
    git(repo.path(), &["init", "-q"])?;
    git(repo.path(), &["config", "user.name", "Diff proof"])?;
    git(
        repo.path(),
        &["config", "user.email", "proof@example.invalid"],
    )?;
    let before = (1..=120)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    for name in ["a.txt", "b.txt"] {
        fs::write(repo.path().join(name), &before)?;
    }
    git(repo.path(), &["add", "."])?;
    git(repo.path(), &["commit", "-qm", "base"])?;
    let after = before.replace("line 100\n", "changed 100\n");
    for name in ["a.txt", "b.txt"] {
        fs::write(repo.path().join(name), &after)?;
    }
    let patch = git(
        repo.path(),
        &["diff", "--unified=120", "--no-color", "HEAD", "--"],
    )?;
    fs::write(out.join("navigation.patch"), &patch)?;
    let mut recording = common::script(None, |_| {})?;
    for row in &mut recording {
        if let Some(thread) = row.pointer_mut("/msg/result/thread") {
            thread["cwd"] = repo.path().to_string_lossy().into_owned().into();
        }
    }
    let mut fx = Fx::scripted(recording)?;
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
            Event::Notification {
                method: "turn/diff/updated".into(),
                params: json!({"threadId":SUBTRACT,"turnId":turn,"diff":patch}),
            },
            cx,
        );
    });
    fx.cx.run_until_parked();
    fx.click((1192., 68.))?;
    choose(&mut fx, "Collapse all diffs")?;
    fx.click((1222., 68.))?;
    fx.type_text("b.txt")?;
    fx.reference_shot()?.save(out.join("jump-filtered.png"))?;
    fx.keys("enter")?;
    ensure!(
        fx.read(|root, cx| {
            let files = &root.diff_panel().unwrap().read(cx).files;
            !files[0].open && files[1].open
        }),
        "file jump expanded the wrong file"
    );
    fx.reference_shot()?.save(out.join("jump-b.png"))?;
    fx.scroll((1000., 400.), -400.)?;
    fx.reference_shot()?.save(out.join("sticky-b.png"))?;
    fx.click((950., 110.))?;
    ensure!(
        fx.read(|root, cx| !root.diff_panel().unwrap().read(cx).files[1].open),
        "the scrolled file header did not stay clickable"
    );
    fx.click((1222., 68.))?;
    fx.type_text("b.txt")?;
    fx.keys("enter")?;
    fx.reference_shot()?.save(out.join("tab-trigger-b.png"))?;
    fx.click((1234., 110.))?;
    fx.wait("selected file tab", |fx| {
        fx.read(|root, cx| {
            root.file_view().is_some_and(|file| {
                let file = file.read(cx);
                file.path.ends_with("/b.txt")
                    && file
                        .contents
                        .as_ref()
                        .is_some_and(|text| text.as_deref() == Ok(after.as_str()))
            })
        })
    })?;
    fx.reference_shot()?.save(out.join("file-b.png"))?;
    Ok(())
}
