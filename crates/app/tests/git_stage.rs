#![cfg(target_os = "macos")]

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host as _, LocalHost, Request, json_body};
use common::{Fx, GREETING, script};
use serde_json::json;
use std::{fs, path::Path, process::Command, sync::Arc};

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

fn choose(fx: &mut Fx, label: &str) -> Result<()> {
    let index = fx
        .read(|root, cx| {
            root.diff_panel()
                .unwrap()
                .read(cx)
                .menu_items()
                .filter(|(_, enabled)| *enabled)
                .position(|(text, _)| text == label)
        })
        .with_context(|| format!("enabled menu item {label}"))?;
    fx.keys("home")?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")
}

fn loaded(fx: &mut Fx) -> Result<()> {
    fx.wait("repository changes", |fx| {
        fx.read(|root, cx| !root.diff_panel().unwrap().read(cx).pending())
    })?;
    let error = fx.read(|root, cx| {
        root.diff_panel()
            .unwrap()
            .read(cx)
            .repository_error()
            .map(str::to_owned)
    });
    ensure!(error.is_none(), "repository error: {error:?}");
    Ok(())
}

fn source(fx: &mut Fx, label: &str) -> Result<()> {
    fx.click((735., 70.))?;
    choose(fx, label)?;
    loaded(fx)
}

fn files(fx: &Fx) -> Vec<(String, usize, usize)> {
    fx.read(|root, cx| {
        root.diff_panel()
            .unwrap()
            .read(cx)
            .files
            .iter()
            .map(|f| (format!("{}{}", f.dir, f.name), f.added, f.removed))
            .collect()
    })
}

fn file_action(fx: &mut Fx, label: &str) -> Result<()> {
    fx.click((1258., 110.))?;
    choose(fx, label)
}

fn displayed(fx: &mut Fx, word: &str) -> Result<()> {
    fx.click((900., 145.))?;
    fx.keys("cmd-f")?;
    fx.type_text(word)?;
    ensure!(
        fx.read(|root, cx| root
            .diff_panel()
            .unwrap()
            .read(cx)
            .search()
            .is_some_and(|(query, count)| query == word && count == 1)),
        "missing displayed diff text {word}"
    );
    fx.keys("escape")
}

#[test]
fn stage_and_unstage_preserve_worktree_and_show_the_real_index() -> Result<()> {
    let scratch = std::env::var_os("CGX_GIT_STAGE_EVIDENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("cgx-git-stage"));
    fs::create_dir_all(&scratch)?;
    let repo = tempfile::tempdir_in(&scratch)?;
    let root = repo.path();
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    git(root, &["config", "user.name", "Native proof"])?;
    fs::write(root.join("a.txt"), "base\n")?;
    git(root, &["add", "a.txt"])?;
    git(root, &["commit", "-qm", "base"])?;
    fs::write(root.join("a.txt"), "index\n")?;
    git(root, &["add", "a.txt"])?;
    fs::write(root.join("a.txt"), "worktree\n")?;
    fs::create_dir(root.join("subdir"))?;
    let cwd = root.join("subdir").to_string_lossy().into_owned();
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        if let Some(thread) = line.pointer_mut("/msg/result/thread") {
            thread["cwd"] = cwd.clone().into();
        }
    }
    let home = tempfile::tempdir_in(&scratch)?;
    let host = Arc::new(LocalHost::new(home.path()));
    let mut fx = Fx::scripted(lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.keys("alt-cmd-b")?;
    let last_turn = files(&fx);
    source(&mut fx, "Unstaged")?;
    ensure!(files(&fx) == [("a.txt".into(), 1, 1)]);
    displayed(&mut fx, "worktree")?;
    file_action(&mut fx, "Stage file")?;
    loaded(&mut fx)?;
    ensure!(files(&fx).is_empty());
    ensure!(git(root, &["show", ":a.txt"])? == "worktree\n");
    source(&mut fx, "Staged")?;
    ensure!(files(&fx) == [("a.txt".into(), 1, 1)]);
    displayed(&mut fx, "worktree")?;
    fx.shot()?.save(scratch.join("after-staged.png"))?;
    fx.click((1258., 110.))?;
    fx.shot()?.save(scratch.join("after-unstage-menu.png"))?;
    choose(&mut fx, "Unstage file")?;
    loaded(&mut fx)?;
    ensure!(files(&fx).is_empty());
    ensure!(git(root, &["show", ":a.txt"])? == "base\n");
    ensure!(fs::read_to_string(root.join("a.txt"))? == "worktree\n");
    source(&mut fx, "Unstaged")?;
    displayed(&mut fx, "worktree")?;
    fx.click((1258., 110.))?;
    fx.shot()?.save(scratch.join("after-stage-menu.png"))?;
    fx.keys("escape")?;

    fs::write(root.join(".git/index.lock"), "owned proof lock")?;
    file_action(&mut fx, "Stage file")?;
    fx.wait("index lock failure", |fx| {
        fx.read(|root, cx| {
            root.diff_panel()
                .unwrap()
                .read(cx)
                .repository_error()
                .is_some()
        })
    })?;
    ensure!(git(root, &["show", ":a.txt"])? == "base\n");
    fs::remove_file(root.join(".git/index.lock"))?;
    source(&mut fx, "Unstaged")?;
    ensure!(files(&fx) == [("a.txt".into(), 1, 1)]);
    fs::remove_file(root.join("a.txt"))?;
    source(&mut fx, "Unstaged")?;
    ensure!(files(&fx) == [("a.txt".into(), 0, 1)]);
    file_action(&mut fx, "Stage file")?;
    loaded(&mut fx)?;
    ensure!(git(root, &["ls-files"])?.is_empty());
    source(&mut fx, "Staged")?;
    ensure!(files(&fx) == [("a.txt".into(), 0, 1)]);
    file_action(&mut fx, "Unstage file")?;
    loaded(&mut fx)?;
    ensure!(!root.join("a.txt").exists());
    ensure!(git(root, &["show", ":a.txt"])? == "base\n");
    source(&mut fx, "Last Turn")?;
    ensure!(files(&fx) == last_turn);

    let unborn = tempfile::tempdir_in(&scratch)?;
    git(unborn.path(), &["init", "-q"])?;
    fs::write(unborn.path().join("literal[1].txt"), "new file\n")?;
    fs::write(unborn.path().join("literal1.txt"), "not selected\n")?;
    let params = json!({"cwd": unborn.path(), "path": "literal[1].txt"});
    let staged = json_body(&host.fetch(&Request::post("git-stage", Some(params.clone())))?)?;
    ensure!(staged["staged"][0]["path"] == "literal[1].txt");
    ensure!(git(unborn.path(), &["ls-files"])? == "literal[1].txt\n");
    host.fetch(&Request::post("git-unstage", Some(params)))?;
    ensure!(git(unborn.path(), &["ls-files"])?.is_empty());
    ensure!(fs::read_to_string(unborn.path().join("literal[1].txt"))? == "new file\n");

    let unusual = "line\ndiff --git odd.txt";
    fs::write(unborn.path().join(unusual), "before\n")?;
    git(unborn.path(), &["add", "."])?;
    git(
        unborn.path(),
        &[
            "-c",
            "user.name=Proof",
            "-c",
            "user.email=proof@example.invalid",
            "commit",
            "-qm",
            "base",
        ],
    )?;
    fs::write(unborn.path().join("literal[1].txt"), "changed literal\n")?;
    fs::write(unborn.path().join(unusual), "changed newline\n")?;
    let changes = json_body(&host.fetch(&Request::post(
        "git-changes",
        Some(json!({"cwd": unborn.path()})),
    ))?)?;
    let patches = changes["unstaged"].as_array().context("unstaged patches")?;
    ensure!(patches.len() == 2);
    for (path, content) in [
        ("literal[1].txt", "+changed literal"),
        (unusual, "+changed newline"),
    ] {
        ensure!(patches.iter().any(|patch| {
            patch["path"] == path
                && patch["diff"]
                    .as_str()
                    .is_some_and(|diff| diff.contains(content))
        }));
    }
    git(unborn.path(), &["add", "."])?;
    let changes = json_body(&host.fetch(&Request::post(
        "git-changes",
        Some(json!({"cwd": unborn.path()})),
    ))?)?;
    ensure!(
        changes["unstaged"] == json!([]),
        "staged tracked files must not become false untracked additions"
    );
    ensure!(
        changes["staged"]
            .as_array()
            .context("staged patches")?
            .len()
            == 2
    );
    ensure!(
        host.fetch(&Request::post(
            "git-stage",
            Some(json!({"cwd": root, "path": "../outside"}))
        ))
        .is_err()
    );
    println!(
        "PASS: native stage/unstage, partial index, subdirectory cwd, deletion, lock recovery, literal untracked path, unborn index, bulk patches with newline paths, no false unstaged additions, path boundary"
    );
    Ok(())
}
