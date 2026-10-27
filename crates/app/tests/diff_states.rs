use anyhow::{Context as _, Result, ensure};
use app::host::{Host as _, LocalHost, Request, json_body};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

fn repository(root: &Path) -> Result<()> {
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.name", "Diff state proof"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    Ok(())
}

#[test]
fn untracked_limit_keeps_tracked_changes_and_never_changes_the_worktree() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let repo = tempfile::tempdir()?;
    repository(repo.path())?;
    fs::write(repo.path().join("tracked.txt"), "before\n")?;
    git(repo.path(), &["add", "."])?;
    git(repo.path(), &["commit", "-qm", "base"])?;
    fs::write(repo.path().join("tracked.txt"), "after\n")?;
    for index in 0..256 {
        fs::write(
            repo.path().join(format!("untracked-{index}.txt")),
            "untracked\n",
        )?;
    }
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let read = || -> Result<Value> {
        json_body(&host.fetch(&Request::post(
            "git-changes",
            Some(json!({
                "cwd": repo.path(), "reviewSource": "uncommitted"
            })),
        ))?)
    };
    let at_limit = read()?;
    ensure!(at_limit["untrackedFilesOmitted"].is_null());
    ensure!(at_limit["review"].as_array().context("review files")?.len() == 257);
    fs::write(repo.path().join("one-too-many.txt"), "untracked\n")?;
    let omitted = read()?;
    ensure!(omitted["untrackedFilesOmitted"] == json!({"count":257,"limit":256}));
    ensure!(
        omitted["review"]
            .as_array()
            .context("tracked review files")?
            .len()
            == 1
    );
    ensure!(omitted["review"][0]["path"] == "tracked.txt");
    ensure!(fs::read_to_string(repo.path().join("one-too-many.txt"))? == "untracked\n");
    ensure!(git(repo.path(), &["diff", "--cached", "--name-only"])?.is_empty());
    Ok(())
}

#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

#[cfg(target_os = "macos")]
fn choose(fx: &mut common::Fx, label: &str) -> Result<()> {
    let index = fx
        .read(|root, cx| {
            root.diff_panel()?
                .read(cx)
                .menu_items()
                .position(|(name, enabled)| name == label && enabled)
        })
        .context("diff menu item")?;
    fx.keys("home")?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")
}

#[cfg(target_os = "macos")]
fn open_repository(root: &Path) -> Result<common::Fx> {
    let mut recording = common::script(None, |_| {})?;
    for line in &mut recording {
        if let Some(thread) = line.pointer_mut("/msg/result/thread") {
            thread["cwd"] = root.to_string_lossy().into_owned().into();
        }
    }
    let mut fx = common::Fx::scripted(recording)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(common::SUBTRACT)?;
    fx.keys("alt-cmd-b")?;
    Ok(fx)
}

#[cfg(target_os = "macos")]
#[test]
fn streamed_patches_and_nested_file_writes_refresh_the_visible_diff() -> Result<()> {
    common::install_panic_hook();
    let repo = tempfile::tempdir()?;
    repository(repo.path())?;
    fs::create_dir(repo.path().join("nested"))?;
    let path = repo.path().join("nested/a.txt");
    fs::write(&path, "before\n")?;
    git(repo.path(), &["add", "."])?;
    git(repo.path(), &["commit", "-qm", "base"])?;
    fs::write(&path, "first\nsecond\n")?;
    let patch = git(repo.path(), &["diff", "--no-color", "--"])?;
    let mut fx = open_repository(repo.path())?;
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
    fx.root.update(&mut fx.cx, |root, cx| root.on_event(bridge::Event::Notification {
        method: "item/fileChange/patchUpdated".into(),
        params: json!({"threadId":common::SUBTRACT,"turnId":turn,"itemId":"streamed-file-change","changes":[{
            "path":path,"kind":{"type":"update"},"diff":patch
        }]}),
    }, cx));
    fx.cx.run_until_parked();
    ensure!(
        fx.read(|root, cx| root
            .diff_panel()
            .unwrap()
            .read(cx)
            .files
            .iter()
            .any(|file| file.name.as_ref() == "a.txt" && file.added == 2)),
        "streamed patch was not shown"
    );
    fx.click((735., 68.))?;
    choose(&mut fx, "Unstaged")?;
    fx.wait("repository source", |fx| {
        fx.read(|root, cx| !root.diff_panel().unwrap().read(cx).pending())
    })?;
    fs::write(&path, "first\nsecond\nthird\n")?;
    fx.wait("nested file watch", |fx| {
        fx.read(|root, cx| {
            root.diff_panel()
                .unwrap()
                .read(cx)
                .files
                .iter()
                .any(|file| file.name.as_ref() == "a.txt" && file.added == 3)
        })
    })?;
    fs::write(repo.path().join("nested/new.txt"), "new file\n")?;
    fx.wait("new nested file", |fx| {
        fx.read(|root, cx| {
            root.diff_panel()
                .unwrap()
                .read(cx)
                .files
                .iter()
                .any(|file| file.name.as_ref() == "new.txt")
        })
    })?;
    if let Some(out) = std::env::var_os("CGX_DIFF_REVIEW_EVIDENCE") {
        let out = std::path::PathBuf::from(out);
        fs::create_dir_all(&out)?;
        fx.reference_shot()?.save(out.join("watch-nested.png"))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn large_diff_next_file_opens_the_selected_native_file_tab() -> Result<()> {
    common::install_panic_hook();
    let repo = tempfile::tempdir()?;
    repository(repo.path())?;
    for index in 0..129 {
        fs::write(repo.path().join(format!("f{index:03}.txt")), "before\n")?;
    }
    git(repo.path(), &["add", "."])?;
    git(repo.path(), &["commit", "-qm", "base"])?;
    for index in 0..129 {
        fs::write(
            repo.path().join(format!("f{index:03}.txt")),
            format!("after {index}\n"),
        )?;
    }
    let patch = git(repo.path(), &["diff", "--no-color", "--"])?;
    let mut fx = open_repository(repo.path())?;
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
                params: json!({"threadId":common::SUBTRACT,"turnId":turn,"diff":patch}),
            },
            cx,
        )
    });
    fx.cx.run_until_parked();
    let first = fx.reference_shot()?;
    fx.click((1242., 131.))?;
    let next = fx.reference_shot()?;
    if let Some(out) = std::env::var_os("CGX_DIFF_REVIEW_EVIDENCE") {
        let out = std::path::PathBuf::from(out);
        fs::create_dir_all(&out)?;
        first.save(out.join("large-first.png"))?;
        next.save(out.join("large-next.png"))?;
    }
    fx.click((1234., 188.))?;
    fx.wait("large diff file tab", |fx| {
        fx.read(|root, cx| {
            root.file_view().is_some_and(|file| {
                let file = file.read(cx);
                file.path.ends_with("/f001.txt")
                    && file
                        .contents
                        .as_ref()
                        .is_some_and(|text| text.as_deref() == Ok("after 1\n"))
            })
        })
    })?;
    Ok(())
}
