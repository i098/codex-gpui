#![cfg(target_os = "macos")]
//! DT-139: the `Commit or push` command, form, commit, branch and push, driven by native
//! input against task-owned repositories and local bare remotes. Commit messages come from a
//! recorded app-server answer; nothing here reaches a real remote or account.

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host as _, LocalHost, Request, json_body};
use common::{Fx, GREETING, exchange, script};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, sync::Arc, time::Duration};

#[test]
fn proxy() {
    common::proxy_child();
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn command(fx: &mut Fx, label: &str) -> Result<()> {
    fx.keys("cmd-k")?;
    fx.type_text(label)?;
    let index = fx
        .read(|root, cx| {
            root.command_menu()
                .unwrap()
                .read(cx)
                .describe()
                .into_iter()
                .flat_map(|(_, rows)| rows)
                .position(|(text, _)| {
                    text.split(" | ").next().unwrap().replace(['[', ']'], "") == label
                })
        })
        .with_context(|| format!("visible command {label}"))?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")?;
    ensure!(fx.read(|root, _| root.command_menu().is_none()));
    Ok(())
}

fn open_form(fx: &mut Fx) -> Result<()> {
    command(fx, "Commit or push")?;
    fx.wait("commit form", |fx| {
        fx.read(|root, _| root.dialog().is_some())
    })
}

fn rows(fx: &Fx) -> Vec<(&'static str, bool, Option<&'static str>)> {
    fx.read(|root, cx| root.dialog().unwrap().read(cx).commit_rows())
}

fn target(fx: &Fx) -> (bool, String) {
    fx.read(|root, cx| root.dialog().unwrap().read(cx).commit_target())
}

fn toast(fx: &mut Fx, title: &str) -> Result<()> {
    toast_shot(fx, title, None)
}

/// Waits for the toast and the closed form, saving a screenshot while the toast shows.
fn toast_shot(fx: &mut Fx, title: &str, shot: Option<&Path>) -> Result<()> {
    fx.wait(title, |fx| {
        fx.read(|root, cx| root.toasts(cx).iter().any(|toast| toast == title))
    })?;
    fx.wait("form closed", |fx| {
        fx.read(|root, _| root.dialog().is_none())
    })?;
    if let Some(path) = shot {
        // The toast fades in.
        fx.advance(Duration::from_millis(600))?;
        fx.shot()?.save(path)?;
    }
    // Let the toast expire so that the next wait sees only its own.
    fx.advance(Duration::from_secs(6))
}

/// A generation exchange the recorded app-server would answer: the ephemeral thread's start,
/// the turn, its agent message and its completion.
fn generation(message: &str) -> Vec<Value> {
    let mut lines = exchange(
        "thread/start",
        json!({"thread": {"id": "gen", "ephemeral": true}}),
    )
    .to_vec();
    lines.extend(exchange(
        "turn/start",
        json!({"turn": {"id": "gen-turn", "status": "inProgress", "items": []}}),
    ));
    lines.push(json!({"dir": "recv", "msg": {"method": "item/completed", "params": {
        "threadId": "gen", "turnId": "gen-turn",
        "item": {"type": "agentMessage", "id": "m", "text": json!({"message": message}).to_string()},
    }}}));
    lines.push(
        json!({"dir": "recv", "msg": {"method": "turn/completed", "params": {
            "threadId": "gen",
            "turn": {"id": "gen-turn", "status": "completed", "items": []},
        }}}),
    );
    lines
}

#[test]
fn commit_branch_and_push_use_real_repositories() -> Result<()> {
    let scratch = std::env::var_os("CGX_GIT_COMMIT_EVIDENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("cgx-git-commit"));
    fs::create_dir_all(&scratch)?;
    let remote = tempfile::tempdir_in(&scratch)?;
    let repo = tempfile::tempdir_in(&scratch)?;
    let (remote, root) = (remote.path(), repo.path());
    git(remote, &["init", "-q", "--bare", "-b", "main"])?;
    git(root, &["init", "-q", "-b", "main"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    git(root, &["config", "user.name", "Native proof"])?;
    fs::write(root.join("a.txt"), "base\n")?;
    git(root, &["add", "a.txt"])?;
    git(root, &["commit", "-qm", "base"])?;
    git(root, &["remote", "add", "origin", remote.to_str().unwrap()])?;
    git(root, &["push", "-q", "-u", "origin", "main"])?;
    fs::create_dir(root.join("sub"))?;
    let cwd = root.join("sub").to_string_lossy().into_owned();
    let other_repo = tempfile::tempdir_in(&scratch)?;
    let other = other_repo.path();
    git(other, &["init", "-q", "-b", "main"])?;
    git(other, &["config", "user.email", "proof@example.invalid"])?;
    git(other, &["config", "user.name", "Native proof"])?;
    fs::write(other.join("o.txt"), "other\n")?;
    git(other, &["add", "o.txt"])?;
    git(other, &["commit", "-qm", "other base"])?;
    let other_cwd = other.to_string_lossy().into_owned();
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        if let Some(thread) = line.pointer_mut("/msg/result/thread") {
            let repository = if thread["id"] == GREETING {
                &cwd
            } else {
                &other_cwd
            };
            thread["cwd"] = repository.clone().into();
        }
    }
    lines.splice(0..0, generation("Update proof text"));
    let home = tempfile::tempdir_in(&scratch)?;
    let host = Arc::new(LocalHost::new(home.path()));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    let remote_head = |branch: &str| git(remote, &["rev-parse", &format!("refs/heads/{branch}")]);
    let base = remote_head("main")?;

    // A clean branch with nothing to push: the command has nothing to open or run.
    command(&mut fx, "Commit or push")?;
    fx.advance(Duration::from_secs(2))?;
    ensure!(fx.read(|root, _| root.dialog().is_none()));
    ensure!(remote_head("main")? == base);

    // `Commit`: staged, unstaged and untracked files, with a typed message.
    fs::write(root.join("a.txt"), "base\nunstaged\n")?;
    fs::write(root.join("b.txt"), "staged\n")?;
    git(root, &["add", "b.txt"])?;
    fs::write(root.join("c.txt"), "untracked\n")?;
    open_form(&mut fx)?;
    ensure!(
        rows(&fx)
            == [
                ("Commit", true, None),
                ("Commit and push", true, None),
                ("Push", false, Some("No new commits to push")),
            ]
    );
    ensure!(target(&fx) == (false, "main".into()));
    ensure!(fx.read(|root, cx| root.dialog().unwrap().read(cx).commit_includes_unstaged()));
    fx.type_text("Add proof files")?;
    fx.shot()?.save(scratch.join("after-commit-form.png"))?;
    fx.keys("tab tab enter")?;
    toast(&mut fx, "Committed to main")?;
    ensure!(git(root, &["log", "-1", "--format=%s"])? == "Add proof files");
    let files = git(root, &["show", "--name-only", "--format="])?;
    ensure!(files.lines().collect::<Vec<_>>() == ["a.txt", "b.txt", "c.txt"]);
    ensure!(git(root, &["status", "--porcelain"])?.is_empty());
    ensure!(remote_head("main")? == base, "Commit must not push");

    // Without `Include unstaged changes` only the index is committed.
    fs::write(root.join("a.txt"), "base\nunstaged\nagain\n")?;
    fs::write(root.join("d.txt"), "indexed\n")?;
    git(root, &["add", "d.txt"])?;
    open_form(&mut fx)?;
    fx.keys("tab space")?;
    ensure!(!fx.read(|root, cx| root.dialog().unwrap().read(cx).commit_includes_unstaged()));
    fx.keys("shift-tab")?;
    fx.type_text("Only the index")?;
    fx.keys("tab tab enter")?;
    toast(&mut fx, "Committed to main")?;
    ensure!(git(root, &["show", "--name-only", "--format=%s"])? == "Only the index\n\nd.txt");
    ensure!(git(root, &["diff", "--name-only"])? == "a.txt");

    // Nothing is staged and the box stays unchecked: the command pushes the two commits.
    git(root, &["checkout", "-q", "--", "a.txt"])?;
    command(&mut fx, "Commit or push")?;
    toast(&mut fx, "Pushed main")?;
    ensure!(remote_head("main")? == git(root, &["rev-parse", "HEAD"])?);

    // `Commit and push` by a mouse click on its row; the choice stays unchecked.
    fs::write(root.join("a.txt"), "base\nclicked\n")?;
    git(root, &["add", "a.txt"])?;
    open_form(&mut fx)?;
    ensure!(!fx.read(|root, cx| root.dialog().unwrap().read(cx).commit_includes_unstaged()));
    fx.type_text("Commit and push by click")?;
    // 420 x 269 px dialog centred in 1280 x 832: the second row of the list.
    fx.click((640., 495.))?;
    toast(&mut fx, "Pushed main")?;
    ensure!(git(root, &["log", "-1", "--format=%s"])? == "Commit and push by click");
    ensure!(remote_head("main")? == git(root, &["rev-parse", "HEAD"])?);

    // A new branch: the prefilled name, the exists check, then the branch is pushed.
    fs::write(root.join("a.txt"), "base\nbranch\n")?;
    git(root, &["add", "a.txt"])?;
    open_form(&mut fx)?;
    fx.keys("shift-tab enter down enter")?;
    let (new, name) = target(&fx);
    ensure!(new && name.starts_with("codex/"), "default name {name}");
    fx.keys("tab cmd-a")?;
    fx.type_text("main")?;
    ensure!(rows(&fx).iter().all(|(_, enabled, _)| !enabled));
    fx.keys("cmd-a")?;
    fx.type_text("feature/proof")?;
    ensure!(rows(&fx)[0].1);
    fx.shot()?.save(scratch.join("after-new-branch-form.png"))?;
    fx.keys("tab")?;
    fx.type_text("Start the proof branch")?;
    fx.keys("tab tab tab enter")?;
    toast_shot(
        &mut fx,
        "Pushed feature/proof",
        Some(&scratch.join("after-pushed-toast.png")),
    )?;
    ensure!(git(root, &["rev-parse", "--abbrev-ref", "HEAD"])? == "feature/proof");
    ensure!(git(root, &["rev-parse", "--abbrev-ref", "@{u}"])? == "origin/feature/proof");
    ensure!(remote_head("feature/proof")? == git(root, &["rev-parse", "HEAD"])?);
    ensure!(remote_head("main")? != git(root, &["rev-parse", "HEAD"])?);

    // A blank message is written by the recorded ephemeral generation, with the diff in its
    // prompt and a structured-output schema.
    fs::write(root.join("a.txt"), "base\nbranch\ngenerated line\n")?;
    git(root, &["add", "a.txt"])?;
    open_form(&mut fx)?;
    fx.keys("cmd-enter")?;
    toast(&mut fx, "Committed to feature/proof")?;
    ensure!(git(root, &["log", "-1", "--format=%s"])? == "Update proof text");
    let sent = fx.sent()?;
    let start = sent
        .iter()
        .find(|m| m["method"] == "thread/start" && m["params"]["ephemeral"] == true)
        .context("ephemeral generation thread")?;
    ensure!(start["params"]["permissions"] == ":read-only");
    let turn = sent
        .iter()
        .find(|m| m["method"] == "turn/start" && m["params"]["threadId"] == "gen")
        .context("generation turn")?;
    let prompt = turn["params"]["input"][0]["text"]
        .as_str()
        .unwrap_or_default();
    ensure!(
        prompt.contains("+generated line")
            && prompt.contains("Untracked changes are not included.")
    );
    ensure!(turn["params"]["outputSchema"]["required"] == json!(["message"]));
    ensure!(sent.iter().any(|m| m["method"] == "thread/unsubscribe"));

    // Generation fails when the app-server cannot answer: no commit is made.
    fs::write(root.join("a.txt"), "base\nbranch\nunanswered\n")?;
    git(root, &["add", "a.txt"])?;
    let before = git(root, &["rev-parse", "HEAD"])?;
    open_form(&mut fx)?;
    fx.keys("cmd-enter")?;
    fx.wait("generation failure", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|t| t.starts_with("Failed to generate commit message"))
        })
    })?;
    ensure!(git(root, &["rev-parse", "HEAD"])? == before);
    fx.advance(Duration::from_secs(6))?;

    // A locked index fails the commit with the original's title and leaves the history alone.
    fs::write(root.join(".git/index.lock"), "owned proof lock")?;
    open_form(&mut fx)?;
    fx.type_text("Locked")?;
    fx.keys("cmd-enter")?;
    toast(&mut fx, "Failed to commit changes")?;
    ensure!(git(root, &["rev-parse", "HEAD"])? == before);
    fs::remove_file(root.join(".git/index.lock"))?;

    // The unpushed commit is pushed by the command; an unreachable remote fails visibly.
    git(root, &["add", "-A"])?;
    git(root, &["commit", "-qm", "Unpushed"])?;
    git(
        root,
        &["remote", "set-url", "origin", "/nonexistent/proof-remote"],
    )?;
    command(&mut fx, "Commit or push")?;
    toast(&mut fx, "Failed to push changes")?;
    git(
        root,
        &["remote", "set-url", "origin", remote.to_str().unwrap()],
    )?;
    command(&mut fx, "Commit or push")?;
    toast(&mut fx, "Pushed feature/proof")?;
    ensure!(remote_head("feature/proof")? == git(root, &["rev-parse", "HEAD"])?);

    // Git pushes where `pushRemote` says, not to the fetch remote.
    let fork = tempfile::tempdir_in(&scratch)?;
    git(fork.path(), &["init", "-q", "--bare", "-b", "main"])?;
    git(
        root,
        &["remote", "add", "fork", fork.path().to_str().unwrap()],
    )?;
    git(root, &["config", "branch.feature/proof.pushRemote", "fork"])?;
    fs::write(root.join("a.txt"), "base\nfork\n")?;
    git(root, &["commit", "-qam", "To the fork"])?;
    let origin_before = remote_head("feature/proof")?;
    command(&mut fx, "Commit or push")?;
    toast(&mut fx, "Pushed feature/proof")?;
    ensure!(
        git(fork.path(), &["rev-parse", "refs/heads/feature/proof"])?
            == git(root, &["rev-parse", "HEAD"])?
    );
    ensure!(remote_head("feature/proof")? == origin_before);

    // The form keeps the repository it read when the chat changes under it.
    fs::write(other.join("o.txt"), "other\nstaged\n")?;
    git(other, &["add", "o.txt"])?;
    fs::write(root.join("a.txt"), "base\nform repository\n")?;
    git(root, &["add", "a.txt"])?;
    let (root_head, other_head) = (
        git(root, &["rev-parse", "HEAD"])?,
        git(other, &["rev-parse", "HEAD"])?,
    );
    open_form(&mut fx)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(common::SUBTRACT)?;
    ensure!(
        fx.read(|root, _| root.dialog().is_some()),
        "the form stays open"
    );
    // The chat change took the key focus: click into the message, then on `Commit`.
    fx.click((640., 362.))?;
    fx.type_text("From the first chat")?;
    fx.click((640., 464.))?;
    toast(&mut fx, "Committed to feature/proof")?;
    ensure!(git(root, &["log", "-1", "--format=%s"])? == "From the first chat");
    ensure!(git(root, &["rev-parse", "HEAD"])? != root_head);
    ensure!(
        git(other, &["rev-parse", "HEAD"])? == other_head,
        "the open chat's repository stays untouched"
    );

    // The host boundary: nothing to commit, bad branch names, a detached HEAD, no remote.
    let call = |endpoint: &str, params: Value| -> Result<Value> {
        let mut params = params;
        params["cwd"] = root.to_str().unwrap().into();
        json_body(&host.fetch(&Request::post(endpoint, Some(params)))?)
    };
    let nothing = call(
        "git-commit",
        json!({"message": "empty", "includeUnstaged": false}),
    )?;
    ensure!(nothing["errorType"] == "nothing-to-commit");
    for (name, why) in [
        ("bad..name", "Invalid branch name"),
        ("main", "Branch already exists"),
    ] {
        let refused = call("git-commit", json!({"message": "m", "newBranch": name}))?;
        ensure!(refused["stage"] == "branch" && refused["error"].as_str().unwrap().contains(why));
    }
    fs::write(root.join("HEAD"), "tracked\n")?;
    git(root, &["add", "--", "HEAD"])?;
    git(root, &["commit", "-qm", "A file named HEAD"])?;
    fs::write(root.join("HEAD"), "tracked\nchanged\n")?;
    let named = call("git-commit-status", json!({"includeUnstaged": true}))?;
    ensure!(named["all"]["files"] == 1 && named["all"]["additions"] == 1);
    let diff = call("git-commit-diff", json!({"includeUnstaged": true}))?;
    ensure!(diff["unifiedDiff"].as_str().unwrap().contains("+changed"));
    git(root, &["checkout", "-q", "--", "HEAD"])?;
    git(root, &["checkout", "-q", "--detach"])?;
    let status = call("git-commit-status", json!({}))?;
    ensure!(status["branch"].is_null() && status["remote"].is_string());
    let detached = call("git-push", json!({}))?;
    ensure!(detached["error"] == "Branch information unavailable");
    let lone = tempfile::tempdir_in(&scratch)?;
    git(lone.path(), &["init", "-q", "-b", "main"])?;
    let lone_status = json_body(&host.fetch(&Request::post(
        "git-commit-status",
        Some(json!({"cwd": lone.path()})),
    ))?)?;
    ensure!(lone_status["remote"].is_null() && lone_status["hasCommits"] == false);
    println!(
        "PASS: native commit form, staged and unstaged selection, push-only command, click commit-and-push, new branch with exists check and upstream, recorded message generation, failure toasts, host boundary"
    );
    Ok(())
}
