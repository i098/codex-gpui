#![cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, SUBTRACT};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::Command,
};
use ui::screens::diff::panel::FullError;

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

fn output() -> Result<PathBuf> {
    let path = std::env::var_os("CGX_DIFF_REVIEW_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("cgx-diff-content"));
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn prepare(unreadable: bool) -> Result<(tempfile::TempDir, Fx)> {
    common::install_panic_hook();
    let repo = tempfile::tempdir_in(output()?)?;
    git(repo.path(), &["init", "-q"])?;
    git(repo.path(), &["config", "user.name", "Full-file proof"])?;
    git(
        repo.path(),
        &["config", "user.email", "proof@example.invalid"],
    )?;
    let before = (1..=60)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    fs::write(repo.path().join("file.txt"), &before)?;
    git(repo.path(), &["add", "."])?;
    git(repo.path(), &["commit", "-qm", "base"])?;
    fs::write(
        repo.path().join("file.txt"),
        before.replace("line 40\n", "changed 40\n"),
    )?;
    let patch = git(repo.path(), &["diff", "--full-index", "--unified=0", "--"])?;
    fs::write(output()?.join("full-file.patch"), &patch)?;
    if unreadable {
        fs::set_permissions(
            repo.path().join("file.txt"),
            fs::Permissions::from_mode(0o000),
        )?;
    }
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
    update_patch(&mut fx, patch);
    fx.cx.run_until_parked();
    Ok((repo, fx))
}

fn find_count(fx: &Fx) -> usize {
    fx.read(|root, cx| {
        root.diff_panel()
            .unwrap()
            .read(cx)
            .search()
            .map_or(0, |(_, count)| count)
    })
}

fn choose(fx: &mut Fx, label: &str) -> Result<()> {
    let index = fx
        .read(|root, cx| {
            root.diff_panel()?
                .read(cx)
                .menu_items()
                .filter(|(_, enabled)| *enabled)
                .position(|(item, _)| item == label)
        })
        .context("diff option")?;
    fx.keys("home")?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")
}

fn update_patch(fx: &mut Fx, patch: String) {
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
}

fn body_menu(fx: &mut Fx, at: (f32, f32)) -> Result<()> {
    fx.right_click(at)?;
    fx.wait("diff context menu", |fx| {
        fx.read(|root, cx| {
            root.diff_panel()
                .is_some_and(|panel| panel.read(cx).menu_items().len() > 0)
        })
    })
}

#[test]
fn full_file_context_expands_twenty_lines_and_collapses_to_the_patch() -> Result<()> {
    let (_repo, mut fx) = prepare(false)?;
    fx.reference_shot()?
        .save(output()?.join("full-partial.png"))?;
    fx.click((1236., 143.))?;
    fx.keys("cmd-f")?;
    fx.type_text("line ")?;
    fx.wait("twenty context lines", |fx| find_count(fx) == 21)?;
    fx.reference_shot()?
        .save(output()?.join("full-twenty-lines.png"))?;
    fx.keys("escape")?;
    fx.click((900., 143.))?;
    fx.keys("cmd-f")?;
    fx.type_text("line ")?;
    fx.wait("all leading context", |fx| find_count(fx) == 40)?;
    fx.reference_shot()?
        .save(output()?.join("full-expanded.png"))?;
    fx.keys("escape")?;
    fx.click((1192., 68.))?;
    choose(&mut fx, "Load full files")?;
    ensure!(
        !fx.read(|root, cx| root.diff_panel().unwrap().read(cx).loads_full_files()),
        "full-file mode stayed enabled"
    );
    fx.click((1000., 177.))?;
    fx.keys("cmd-f")?;
    fx.type_text("line ")?;
    ensure!(
        find_count(&fx) == 1,
        "turning off full files kept expanded context"
    );
    Ok(())
}

#[test]
fn full_file_read_failure_can_retry_without_losing_the_patch() -> Result<()> {
    let (repo, mut fx) = prepare(true)?;
    let path = repo.path().join("file.txt");
    fx.reference_shot()?;
    fx.wait("full-file read error", |fx| {
        fx.read(|root, cx| {
            root.diff_panel().unwrap().read(cx).full_file_error(0) == Some(FullError::Failed)
        })
    })?;
    fx.reference_shot()?
        .save(output()?.join("full-read-error.png"))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
    fx.click((1240., 154.))?;
    fx.reference_shot()?;
    fx.click((900., 143.))?;
    fx.keys("cmd-f")?;
    fx.type_text("line ")?;
    fx.wait("full-file retry", |fx| find_count(fx) == 40)?;
    fx.reference_shot()?
        .save(output()?.join("full-read-recovered.png"))?;
    Ok(())
}

#[test]
fn import_filter_preserves_code_and_annotated_imports() -> Result<()> {
    let (repo, mut fx) = prepare(false)?;
    let mut before = String::from("use std::fmt;\n\n");
    for index in 0..8 {
        before.push_str(&format!("fn unchanged_{index}() {{}}\n"));
    }
    before.push_str("pub fn value() -> u32 { 1 }\n");
    let path = repo.path().join("file.rs");
    fs::write(&path, &before)?;
    git(repo.path(), &["add", "file.rs"])?;
    git(repo.path(), &["commit", "-qm", "Rust before"])?;
    fs::write(
        &path,
        before
            .replace("std::fmt", "std::io")
            .replace("{ 1 }", "{ 2 }"),
    )?;
    update_patch(
        &mut fx,
        git(
            repo.path(),
            &["diff", "--full-index", "--unified=0", "--", "file.rs"],
        )?,
    );
    fx.reference_shot()?;
    fx.click((1000., 160.))?;
    fx.keys("cmd-f")?;
    fx.type_text("std::")?;
    fx.wait("visible import change", |fx| find_count(fx) == 2)?;
    fx.keys("escape")?;
    fx.click((1192., 68.))?;
    choose(&mut fx, "Hide imports")?;
    fx.click((1000., 160.))?;
    fx.keys("cmd-f cmd-a")?;
    fx.type_text("std::")?;
    fx.wait("hidden import change", |fx| find_count(fx) == 0)?;
    fx.keys("cmd-a")?;
    fx.type_text("value")?;
    fx.wait("visible non-import change", |fx| find_count(fx) == 2)?;
    fx.reference_shot()?
        .save(output()?.join("imports-hidden.png"))?;
    fx.keys("escape")?;
    fx.click((1192., 68.))?;
    choose(&mut fx, "Hide imports")?;
    fx.click((704., 163.))?;
    fx.type_text("Keep this import visible.")?;
    fx.keys("tab tab enter")?;
    fx.click((1192., 68.))?;
    choose(&mut fx, "Hide imports")?;
    fx.click((1000., 160.))?;
    fx.keys("cmd-f cmd-a")?;
    fx.type_text("std::")?;
    fx.wait("annotated import stays visible", |fx| find_count(fx) == 2)?;
    fx.reference_shot()?
        .save(output()?.join("imports-annotated.png"))?;
    Ok(())
}

#[test]
fn copied_git_command_keeps_hidden_whitespace_changes() -> Result<()> {
    let (repo, mut fx) = prepare(false)?;
    fs::write(repo.path().join("spacing.txt"), "value = 1\n")?;
    git(repo.path(), &["add", "spacing.txt"])?;
    git(repo.path(), &["commit", "-qm", "spacing before"])?;
    fs::write(repo.path().join("spacing.txt"), "value  =  1\n")?;
    fx.reference_shot()?;
    fx.click((715., 68.))?;
    choose(&mut fx, "Uncommitted")?;
    fx.wait("working tree diff", |fx| {
        fx.read(|root, cx| {
            root.diff_panel()
                .is_some_and(|panel| !panel.read(cx).pending())
        })
    })?;
    fx.click((1192., 68.))?;
    choose(&mut fx, "Hide white space")?;
    fx.wait("whitespace-only file hidden", |fx| {
        fx.read(|root, cx| {
            root.diff_panel().is_some_and(|panel| {
                let panel = panel.read(cx);
                !panel.pending()
                    && panel.files.len() == 1
                    && panel.files[0].name.as_ref() == "file.txt"
            })
        })
    })?;
    fx.click((1192., 68.))?;
    fx.reference_shot()?
        .save(output()?.join("options-working-tree.png"))?;
    choose(&mut fx, "Copy git apply command")?;
    fx.wait("copied patch toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|text| text.as_ref() == "Copied git apply command to the clipboard")
        })
    })?;
    let command = fx
        .cx
        .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
        .context("copied command")?;
    let destination = tempfile::tempdir_in(output()?)?;
    let clone = destination.path().join("copy");
    git(
        repo.path(),
        &[
            "clone",
            "--quiet",
            "--no-hardlinks",
            repo.path().to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    )?;
    let applied = Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(&clone)
        .output()?;
    ensure!(
        applied.status.success(),
        "Copied command failed: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    ensure!(
        fs::read_to_string(clone.join("file.txt"))?
            == fs::read_to_string(repo.path().join("file.txt"))?
    );
    ensure!(fs::read_to_string(clone.join("spacing.txt"))? == "value  =  1\n");
    Ok(())
}

#[test]
fn context_menu_opens_line_comments_and_file_tabs() -> Result<()> {
    let (repo, mut fx) = prepare(false)?;
    fx.reference_shot()?;
    body_menu(&mut fx, (900., 184.))?;
    fx.reference_shot()?
        .save(output()?.join("diff-context-menu.png"))?;
    choose(&mut fx, "Request changes")?;
    fx.wait("context line comment", |fx| {
        fx.read(|root, cx| {
            root.diff_panel()
                .is_some_and(|panel| panel.read(cx).comment_draft().is_some())
        })
    })?;
    fx.type_text("Keep this line checked.")?;
    fx.keys("escape")?;
    body_menu(&mut fx, (900., 184.))?;
    choose(&mut fx, "Copy path")?;
    let copied = fx
        .cx
        .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    ensure!(copied.as_deref() == repo.path().join("file.txt").to_str());
    body_menu(&mut fx, (900., 184.))?;
    choose(&mut fx, "Copy relative path")?;
    let copied = fx
        .cx
        .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    ensure!(copied.as_deref() == Some("file.txt"));
    body_menu(&mut fx, (900., 184.))?;
    choose(&mut fx, "Open with")?;
    fx.reference_shot()?
        .save(output()?.join("diff-open-with.png"))?;
    choose(&mut fx, "New tab")?;
    fx.wait("context file tab", |fx| {
        fx.read(|root, cx| {
            root.file_view().is_some_and(|view| {
                let view = view.read(cx);
                view.path.ends_with("/file.txt")
                    && view.contents.as_ref().is_some_and(|text| {
                        text.as_deref()
                            .is_ok_and(|text| text.contains("changed 40"))
                    })
            })
        })
    })?;
    fx.reference_shot()?
        .save(output()?.join("diff-context-file-tab.png"))?;
    Ok(())
}

#[test]
fn removed_files_hide_github_and_new_tab_actions() -> Result<()> {
    let (repo, mut fx) = prepare(false)?;
    git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "git@github.com:example/project.git",
        ],
    )?;
    fs::remove_file(repo.path().join("file.txt"))?;
    let patch = git(repo.path(), &["diff", "--full-index", "--", "file.txt"])?;
    update_patch(&mut fx, patch);
    fx.reference_shot()?;
    body_menu(&mut fx, (900., 164.))?;
    ensure!(fx.read(|root, cx| {
        root.diff_panel()
            .unwrap()
            .read(cx)
            .menu_items()
            .all(|(label, _)| label != "Open in GitHub")
    }));
    fx.reference_shot()?
        .save(output()?.join("diff-deleted-menu.png"))?;
    choose(&mut fx, "Open with")?;
    ensure!(fx.read(|root, cx| {
        root.diff_panel()
            .unwrap()
            .read(cx)
            .menu_items()
            .all(|(label, _)| label != "New tab")
    }));
    Ok(())
}
