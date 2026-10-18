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
    fx.wait("available command", |fx| {
        fx.read(|root, cx| root.command_menu().unwrap().read(cx).describe())
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .any(|(text, _)| text.split(" | ").next().unwrap().replace(['[', ']'], "") == label)
    })?;
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

fn offers_commit(fx: &Fx) -> bool {
    fx.read(|root, cx| root.command_menu().unwrap().read(cx).describe())
        .into_iter()
        .flat_map(|(_, rows)| rows)
        .any(|(label, _)| label.contains("Commit"))
}

fn open_form(fx: &mut Fx) -> Result<()> {
    command(fx, "Commit or push")?;
    fx.wait("commit form", |fx| {
        fx.read(|root, _| root.dialog().is_some())
    })
}

fn header_menu(fx: &mut Fx) -> Result<()> {
    if !fx.read(|root, cx| root.thread().unwrap().read(cx).summary_open) {
        fx.click((1217., 22.))?;
    }
    fx.click((1242., 106.))?;
    fx.wait("Git header status", |fx| {
        fx.read(|root, cx| {
            let view = root.thread().unwrap().read(cx);
            view.git_actions_open()
                && view
                    .git_action_rows()
                    .iter()
                    .all(|(_, reason)| !reason.is_some_and(|reason| reason.starts_with("Loading")))
        })
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
    fs::create_dir(root.join("proj"))?;
    let cwd = root.join("proj").to_string_lossy().into_owned();
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

    fx.click((1217., 22.))?;
    fx.shot()?.save(scratch.join("after-summary-header.png"))?;
    fx.keys("cmd-k")?;
    fx.type_text("Commit or push")?;
    fx.advance(Duration::from_secs(2))?;
    fx.shot()?.save(scratch.join("after-clean-command.png"))?;
    ensure!(
        !offers_commit(&fx),
        "a clean synchronized branch must not offer Commit or push"
    );
    fx.keys("escape")?;
    ensure!(remote_head("main")? == base);
    header_menu(&mut fx)?;
    fx.shot()?
        .save(scratch.join("after-clean-header-menu.png"))?;
    fx.click((880., 71.))?;
    ensure!(fx.read(|root, _| root.dialog().is_none()));
    fx.keys("escape enter")?;
    ensure!(fx.read(|root, cx| root.thread().unwrap().read(cx).git_actions_open()));
    fx.keys("escape")?;

    // Match the retained original form's +5 -1 selection without changing the commit cases.
    fs::write(
        root.join("a.txt"),
        "reference\nline 2\nline 3\nline 4\nline 5\n",
    )?;
    header_menu(&mut fx)?;
    fx.shot()?
        .save(scratch.join("after-dirty-header-menu.png"))?;
    fx.hover((880., 100.))?;
    fx.advance(Duration::from_millis(300))?;
    fx.shot()?.save(scratch.join("after-disabled-push.png"))?;
    fx.keys("down enter")?;
    fx.wait("header commit form", |fx| {
        fx.read(|root, _| root.dialog().is_some())
    })?;
    fx.shot()?.save(scratch.join("original-state-blank.png"))?;
    fx.type_text("Add proof files")?;
    fx.shot()?.save(scratch.join("original-state-filled.png"))?;
    fx.keys("cmd-a backspace shift-tab enter")?;
    fx.shot()?
        .save(scratch.join("original-state-target-menu.png"))?;
    fx.keys("down enter tab cmd-a")?;
    fx.type_text("feature/proof")?;
    fx.shot()?
        .save(scratch.join("original-state-new-branch.png"))?;
    fx.keys("escape")?;
    ensure!(fx.composer_focused()?);
    ensure!(!fx.read(|root, cx| root.thread().unwrap().read(cx).summary_open));
    git(root, &["checkout", "-q", "--", "a.txt"])?;

    // `Commit`: staged, unstaged and untracked files, with a typed message.
    fs::write(root.join("a.txt"), "base\nunstaged\n")?;
    fs::write(root.join("b.txt"), "staged\n")?;
    git(root, &["add", "b.txt"])?;
    fs::write(root.join("c.txt"), "untracked\n")?;
    header_menu(&mut fx)?;
    fx.click((880., 71.))?;
    fx.wait("pointer commit form", |fx| {
        fx.read(|root, _| root.dialog().is_some())
    })?;
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

    // Opening the push-only form must not push until the user selects Push.
    git(root, &["checkout", "-q", "--", "a.txt"])?;
    open_form(&mut fx)?;
    ensure!(remote_head("main")? == base);
    ensure!(
        rows(&fx)
            == [
                ("Commit", false, Some("No changes to commit")),
                ("Commit and push", false, Some("No changes to commit")),
                ("Push", true, None),
            ]
    );
    fx.shot()?.save(scratch.join("after-push-only-form.png"))?;
    fx.keys("enter")?;
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
    open_form(&mut fx)?;
    fx.keys("enter")?;
    toast(&mut fx, "Failed to push changes")?;
    git(
        root,
        &["remote", "set-url", "origin", remote.to_str().unwrap()],
    )?;
    header_menu(&mut fx)?;
    fx.keys("down enter")?;
    fx.wait("header push form", |fx| {
        fx.read(|root, _| root.dialog().is_some())
    })?;
    fx.keys("enter")?;
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
    open_form(&mut fx)?;
    fx.keys("enter")?;
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

#[test]
fn queued_git_actions_preserve_the_subtract_checkout() -> Result<()> {
    use gpui::{
        Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
        PlatformInput, point, px,
    };

    let scratch = std::env::var_os("CGX_GIT_COMMIT_EVIDENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("cgx-git-commit"));
    fs::create_dir_all(&scratch)?;
    let checkout = tempfile::tempdir_in(&scratch)?;
    let workspace = checkout.path().join("proj");
    fs::create_dir(&workspace)?;
    let root = workspace.as_path();
    let remote = tempfile::tempdir_in(&scratch)?;
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"])?;
    git(root, &["init", "-q", "-b", "main"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    git(root, &["config", "user.name", "Native proof"])?;
    fs::create_dir(root.join("src"))?;
    fs::write(root.join("hello.py"), "print(\"Hello, world\")\n")?;
    fs::write(
        root.join("src/math.py"),
        "def add(a, b):\n    return a + b\n",
    )?;
    git(root, &["add", "-A"])?;
    git(root, &["commit", "-qm", "base"])?;
    git(
        root,
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    )?;
    git(root, &["push", "-q", "-u", "origin", "main"])?;
    let base = git(root, &["rev-parse", "HEAD"])?;
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        if let Some(thread) = line.pointer_mut("/msg/result/thread") {
            thread["cwd"] = workspace.to_string_lossy().into_owned().into();
        }
    }
    lines.splice(0..0, generation("Apply the recorded updates"));
    let home = tempfile::tempdir_in(&scratch)?;
    let host = Arc::new(LocalHost::new(home.path()));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.keys("cmd-2")?;
    fx.wait_thread(common::SUBTRACT)?;
    fx.click((1217., 22.))?;
    fx.hover((1242., 106.))?;
    fx.shot()?
        .save(scratch.join("comparison-entry-hover.png"))?;
    fx.keys("cmd-k")?;
    fx.type_text("Commit or push")?;
    fx.advance(Duration::from_millis(300))?;
    ensure!(!offers_commit(&fx));
    fx.shot()?
        .save(scratch.join("comparison-clean-command.png"))?;
    fx.keys("escape")?;
    fs::write(root.join("hello.py"), "print(\"Hello, Codex\")\n")?;
    fs::write(
        root.join("src/math.py"),
        "def add(a, b):\n    return a + b\n\n\ndef subtract(a, b):\n    return a - b\n",
    )?;
    let status = json_body(&host.fetch(&Request::post(
        "git-commit-status",
        Some(json!({"cwd": root})),
    ))?)?;
    ensure!(
        status["branch"] == "main"
            && status["commitsAhead"] == 0
            && status["staged"]["files"] == 0
            && status["all"]["files"] == 2
            && status["all"]["additions"] == 5
            && status["all"]["deletions"] == 1
    );
    serde_json::to_writer_pretty(
        fs::File::create(scratch.join("comparison-git-state.json"))?,
        &status,
    )?;
    header_menu(&mut fx)?;
    fx.shot()?.save(scratch.join("comparison-dirty-menu.png"))?;
    fx.click((903., 71.))?;
    fx.wait("matched header commit form", |fx| {
        fx.read(|root, _| root.dialog().is_some())
    })?;
    fx.shot()?.save(scratch.join("comparison-blank-form.png"))?;
    fx.keys("escape")?;
    fx.keys("cmd-k")?;
    fx.type_text("Commit or push")?;
    fx.wait("dirty command registration", |fx| offers_commit(fx))?;
    fx.shot()?
        .save(scratch.join("comparison-dirty-command.png"))?;
    fx.keys("escape")?;
    open_form(&mut fx)?;

    // Capture queued native work before the deterministic background executor runs it.
    let dispatch_keys = |fx: &mut Fx, keys: &str| -> Result<()> {
        for key in keys.split(' ') {
            let stroke = Keystroke::parse(key)?;
            fx.cx.update_window(fx.window, |_, window, cx| {
                window.dispatch_keystroke(stroke, cx);
            })?;
        }
        Ok(())
    };
    let dispatch_click = |fx: &mut Fx, x: f32, y: f32| -> Result<()> {
        fx.cx.update_window(fx.window, |_, window, cx| {
            let position = point(px(x), px(y));
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }),
                cx,
            );
        })?;
        Ok(())
    };
    dispatch_keys(&mut fx, "cmd-enter")?;
    dispatch_click(&mut fx, 1217., 22.)?;
    fx.cx
        .capture_screenshot(fx.window)?
        .save(scratch.join("queued-pending-header.png"))?;
    let pending = fx.read(|root, cx| root.thread().unwrap().read(cx).git_action_rows());
    ensure!(
        pending
            .iter()
            .all(|(_, reason)| *reason == Some("Generating messages…")),
        "native pending phase after direct submit/capture: {pending:?}"
    );
    dispatch_click(&mut fx, 1242., 106.)?;
    fx.cx
        .capture_screenshot(fx.window)?
        .save(scratch.join("queued-pending-menu.png"))?;
    dispatch_keys(&mut fx, "down enter escape cmd-k")?;
    fx.cx.update_window(fx.window, |_, window, cx| {
        for c in "Commit or push".chars() {
            window.dispatch_keystroke(
                Keystroke {
                    key: c.to_string(),
                    ..Default::default()
                },
                cx,
            );
        }
    })?;
    ensure!(!offers_commit(&fx));
    fx.cx
        .capture_screenshot(fx.window)?
        .save(scratch.join("queued-pending-command.png"))?;
    ensure!(
        git(root, &["rev-parse", "HEAD"])? == base,
        "queued work has not run"
    );
    dispatch_keys(&mut fx, "escape")?;
    toast(&mut fx, "Committed to main")?;
    ensure!(git(root, &["log", "-1", "--format=%s"])? == "Apply the recorded updates");
    ensure!(git(root, &["rev-list", "--count", "HEAD"])? == "2");
    ensure!(git(root, &["status", "--porcelain"])?.is_empty());
    ensure!(git(remote.path(), &["rev-parse", "refs/heads/main"])? == base);
    header_menu(&mut fx)?;
    fx.keys("down enter")?;
    fx.wait("push-only comparison form", |fx| {
        fx.read(|root, _| root.dialog().is_some())
    })?;
    fx.shot()?
        .save(scratch.join("comparison-push-only-form.png"))?;
    ensure!(git(remote.path(), &["rev-parse", "refs/heads/main"])? == base);
    fx.keys("enter")?;
    toast(&mut fx, "Pushed main")?;
    ensure!(
        git(remote.path(), &["rev-parse", "refs/heads/main"])?
            == git(root, &["rev-parse", "HEAD"])?
    );
    git(root, &["checkout", "-q", "--detach"])?;
    header_menu(&mut fx)?;
    fx.shot()?
        .save(scratch.join("after-detached-header-menu.png"))?;
    fx.keys("escape")?;

    fs::rename(root.join(".git"), root.join("git-data"))?;
    header_menu(&mut fx)?;
    fx.shot()?
        .save(scratch.join("after-unavailable-header-menu.png"))?;
    fx.keys("down enter")?;
    ensure!(fx.read(|root, _| root.dialog().is_none()));
    fx.keys("escape cmd-k")?;
    fx.type_text("Commit or push")?;
    fx.advance(Duration::from_secs(1))?;
    ensure!(!offers_commit(&fx));
    fx.keys("escape")?;
    fs::rename(root.join("git-data"), root.join(".git"))?;
    git(root, &["checkout", "-q", "main"])?;
    git(root, &["remote", "remove", "origin"])?;
    git(
        root,
        &["commit", "--allow-empty", "-qm", "Without a remote"],
    )?;
    header_menu(&mut fx)?;
    fx.keys("down enter")?;
    fx.wait("push without a remote form", |fx| {
        fx.read(|root, _| root.dialog().is_some())
    })?;
    fx.shot()?
        .save(scratch.join("after-no-remote-push-form.png"))?;
    fx.keys("enter")?;
    toast(&mut fx, "Failed to push changes")?;
    println!(
        "PASS: native queued/pending guard before background drain, real commit and local ref effects, recorded Subtract fixture, detached/unavailable/no-remote feedback"
    );
    Ok(())
}

#[test]
fn checkout_dispatch_preserves_changes_and_resolves_branches() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let root = scratch.path();
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let call = |params: Value| -> Result<Value> {
        let mut params = params;
        params["cwd"] = json!(root);
        json_body(&host.fetch(&Request::post("git-checkout", Some(params)))?)
    };
    git(root, &["init", "-q", "-b", "main"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    git(root, &["config", "user.name", "Branch proof"])?;
    ensure!(call(json!({"newBranch": "unborn"}))?["status"] == "error");
    ensure!(git(root, &["symbolic-ref", "--short", "HEAD"])? == "main");
    fs::write(root.join("tracked"), "base\n")?;
    git(root, &["add", "tracked"])?;
    git(root, &["commit", "-qm", "base"])?;
    let base = git(root, &["rev-parse", "HEAD"])?;
    for name in ["", "-bad", "HEAD", "bad..name", "bad name", "ends/", "main"] {
        ensure!(call(json!({"newBranch": name}))?["status"] == "error");
        ensure!(git(root, &["rev-parse", "HEAD"])? == base);
        ensure!(git(root, &["symbolic-ref", "--short", "HEAD"])? == "main");
    }
    ensure!(call(json!({"branch": "main", "newBranch": "both"})).is_err());
    ensure!(call(json!({})).is_err());
    ensure!(call(json!({"newBranch": "feature/proof"}))?["branch"] == "feature/proof");
    ensure!(git(root, &["rev-parse", "refs/heads/feature/proof"])? == base);
    fs::write(root.join("tracked"), "feature\n")?;
    fs::write(root.join("collision"), "feature file\n")?;
    git(root, &["add", "-A"])?;
    git(root, &["commit", "-qm", "feature"])?;
    let feature = git(root, &["rev-parse", "HEAD"])?;
    ensure!(call(json!({"branch": "main"}))?["branch"] == "main");
    fs::write(root.join("tracked"), "dirty\n")?;
    for staged in [false, true] {
        if staged {
            git(root, &["add", "tracked"])?;
        }
        let before = git(root, &["status", "--porcelain"])?;
        ensure!(call(json!({"branch": "main"}))?["status"] == "success");
        let blocked = call(json!({"branch": "feature/proof"}))?;
        ensure!(blocked["errorType"] == "blocked-by-working-tree-changes");
        ensure!(blocked["conflictedPaths"] == json!(["tracked"]));
        ensure!(git(root, &["symbolic-ref", "--short", "HEAD"])? == "main");
        ensure!(git(root, &["rev-parse", "HEAD"])? == base);
        ensure!(git(root, &["status", "--porcelain"])? == before);
        ensure!(fs::read_to_string(root.join("tracked"))? == "dirty\n");
    }
    fs::write(root.join("collision"), "untracked\n")?;
    let blocked = call(json!({"branch": "feature/proof"}))?;
    ensure!(blocked["errorType"] == "blocked-by-working-tree-changes");
    ensure!(blocked["conflictedPaths"] == json!(["tracked", "collision"]));
    ensure!(git(root, &["symbolic-ref", "--short", "HEAD"])? == "main");
    ensure!(fs::read_to_string(root.join("tracked"))? == "dirty\n");
    ensure!(fs::read_to_string(root.join("collision"))? == "untracked\n");
    git(root, &["reset", "--hard", "HEAD"])?;
    let blocked = call(json!({"branch": "feature/proof"}))?;
    ensure!(blocked["errorType"] == "blocked-by-working-tree-changes");
    ensure!(blocked["conflictedPaths"] == json!(["collision"]));
    ensure!(fs::read_to_string(root.join("collision"))? == "untracked\n");
    fs::remove_file(root.join("collision"))?;
    fs::write(root.join("unrelated"), "keep me\n")?;
    ensure!(call(json!({"branch": "feature/proof"}))?["status"] == "success");
    ensure!(fs::read_to_string(root.join("unrelated"))? == "keep me\n");
    ensure!(git(root, &["rev-parse", "HEAD"])? == feature);
    ensure!(call(json!({"branch": base}))?["branch"].is_null());
    ensure!(git(root, &["rev-parse", "HEAD"])? == base);
    ensure!(call(json!({"newBranch": "from-detached"}))?["branch"] == "from-detached");
    for name in ["missing", "--force", "tracked"] {
        ensure!(call(json!({"branch": name}))?["status"] == "error");
        ensure!(git(root, &["symbolic-ref", "--short", "HEAD"])? == "from-detached");
    }
    let remote = tempfile::tempdir()?;
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"])?;
    git(
        root,
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    )?;
    git(root, &["push", "-q", "origin", "feature/proof:remote-only"])?;
    ensure!(call(json!({"branch": "remote-only"}))?["branch"] == "remote-only");
    ensure!(git(root, &["rev-parse", "HEAD"])? == feature);
    ensure!(git(root, &["rev-parse", "--abbrev-ref", "@{upstream}"])? == "origin/remote-only");
    git(root, &["push", "-q", "origin", "main:ambiguous"])?;
    git(
        root,
        &["remote", "add", "second", remote.path().to_str().unwrap()],
    )?;
    git(root, &["fetch", "-q", "second"])?;
    let refused = call(json!({"branch": "ambiguous"}))?;
    ensure!(refused["status"] == "error");
    ensure!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("multiple remotes")
    );
    ensure!(git(root, &["symbolic-ref", "--short", "HEAD"])? == "remote-only");
    ensure!(git(remote.path(), &["rev-parse", "refs/heads/remote-only"])? == feature);
    println!(
        "PASS: LocalHost git-checkout changes real refs; invalid/existing names, current/detached refs, tracked/staged/untracked protection, safe dirty checkout, local remote tracking and ambiguity"
    );
    Ok(())
}

#[test]
fn native_branch_controls_preserve_drafts_and_protect_changes() -> Result<()> {
    use ui::dialog::Kind;
    let evidence = std::env::var_os("CGX_GIT_COMMIT_EVIDENCE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    fs::create_dir_all(&evidence)?;
    let checkout = tempfile::tempdir_in(&evidence)?;
    let root = checkout.path();
    let remote = tempfile::tempdir_in(&evidence)?;
    git(root, &["init", "-q", "-b", "main"])?;
    git(root, &["config", "user.email", "proof@example.invalid"])?;
    git(root, &["config", "user.name", "Native branch proof"])?;
    fs::write(root.join("tracked"), "base\n")?;
    git(root, &["add", "tracked"])?;
    git(root, &["commit", "-qm", "base"])?;
    let base = git(root, &["rev-parse", "HEAD"])?;
    git(remote.path(), &["init", "-q", "--bare", "-b", "main"])?;
    git(
        root,
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    )?;
    git(root, &["push", "-q", "-u", "origin", "main"])?;
    git(root, &["checkout", "-qb", "feature/proof"])?;
    fs::write(root.join("tracked"), "feature\n")?;
    git(root, &["commit", "-qam", "feature"])?;
    let feature = git(root, &["rev-parse", "HEAD"])?;
    git(root, &["checkout", "-q", "main"])?;
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        if let Some(thread) = line.pointer_mut("/msg/result/thread") {
            thread["cwd"] = json!(root);
        }
    }
    let home = tempfile::tempdir_in(&evidence)?;
    let host = Arc::new(LocalHost::new(home.path()));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.type_text("Keep this unsent draft")?;
    let open = |fx: &mut Fx| -> Result<()> {
        header_menu(fx)?;
        fx.keys("end enter")?;
        fx.wait("branch selector", |fx| {
            fx.read(|root, cx| {
                root.dialog()
                    .is_some_and(|d| d.read(cx).kind() == Kind::Branches)
            })
        })
    };
    let closed = |fx: &mut Fx| {
        fx.wait("branch operation completes", |fx| {
            fx.read(|root, _| root.dialog().is_none())
        })
    };
    let choose = |fx: &mut Fx, name: &str| -> Result<()> {
        open(fx)?;
        fx.type_text(name)?;
        fx.keys("enter")?;
        closed(fx)
    };
    header_menu(&mut fx)?;
    fx.click((890., 128.))?;
    ensure!(fx.read(|root, _| root.dialog().is_none()));
    ensure!(git(root, &["symbolic-ref", "HEAD"])? == "refs/heads/main");
    fx.keys("escape")?;
    open(&mut fx)?;
    fx.shot()?.save(evidence.join("branches-search.png"))?;
    fx.type_text("main")?;
    fx.keys("enter")?;
    closed(&mut fx)?;
    ensure!(git(root, &["rev-parse", "HEAD"])? == base);
    ensure!(fx.composer_text()? == "Keep this unsent draft");
    git(root, &["tag", "MAIN", &feature])?;
    open(&mut fx)?;
    fx.type_text("MAIN")?;
    fx.shot()?.save(evidence.join("branches-exact-ref.png"))?;
    fx.keys("cmd-a")?;
    fx.type_text("refs/tags/MAIN")?;
    fx.keys("enter")?;
    closed(&mut fx)?;
    ensure!(git(root, &["rev-parse", "HEAD"])? == feature);
    ensure!(git(root, &["symbolic-ref", "-q", "HEAD"]).is_err());
    choose(&mut fx, "main")?;
    ensure!(git(root, &["symbolic-ref", "HEAD"])? == "refs/heads/main");
    open(&mut fx)?;
    fx.keys("tab enter")?;
    ensure!(fx.read(|root, cx| root.dialog().unwrap().read(cx).kind()) == Kind::CreateBranch);
    let create_dialog = fx.read(|root, _| root.dialog().unwrap().clone());
    ensure!(fx.cx.update_window(fx.window, |_, window, cx| {
        gpui::Focusable::focus_handle(&create_dialog, cx).is_focused(window)
    })?);
    fx.shot()?.save(evidence.join("branches-create.png"))?;
    for name in ["ends/", "main"] {
        fx.keys("cmd-a")?;
        fx.type_text(name)?;
        fx.keys("enter")?;
        ensure!(git(root, &["symbolic-ref", "HEAD"])? == "refs/heads/main");
        ensure!(fx.read(|root, cx| root.dialog().unwrap().read(cx).query().to_owned()) == name);
        fx.shot()?.save(evidence.join(if name == "main" {
            "branches-existing-name.png"
        } else {
            "branches-invalid-name.png"
        }))?;
    }
    fx.keys("cmd-a")?;
    fx.type_text("bad..name")?;
    fx.keys("enter")?;
    fx.wait("failed checkout preserves name", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|t| t == "Failed to create branch")
        })
    })?;
    ensure!(fx.read(|root, cx| root.dialog().unwrap().read(cx).query().to_owned()) == "bad..name");
    fx.shot()?
        .save(evidence.join("branches-create-failure.png"))?;
    fx.keys("cmd-a")?;
    fx.type_text("feature/new")?;
    fx.keys("enter")?;
    closed(&mut fx)?;
    ensure!(git(root, &["symbolic-ref", "HEAD"])? == "refs/heads/feature/new");
    ensure!(git(root, &["rev-parse", "HEAD"])? == base);
    open(&mut fx)?;
    fx.type_text("feature/proof")?;
    fx.shot()?.save(evidence.join("branches-filtered.png"))?;
    fx.click((1100., 235.))?;
    closed(&mut fx)?;
    ensure!(git(root, &["rev-parse", "HEAD"])? == feature);
    git(root, &["checkout", "-q", "--detach"])?;
    header_menu(&mut fx)?;
    fx.shot()?
        .save(evidence.join("branches-detached-menu.png"))?;
    fx.keys("home enter")?;
    fx.wait("detached create form", |fx| {
        fx.read(|root, cx| {
            root.dialog()
                .is_some_and(|d| d.read(cx).kind() == Kind::CreateBranch)
        })
    })?;
    fx.keys("cmd-a")?;
    fx.type_text("from-detached")?;
    fx.keys("enter")?;
    closed(&mut fx)?;
    ensure!(git(root, &["symbolic-ref", "HEAD"])? == "refs/heads/from-detached");
    choose(&mut fx, "main")?;
    fs::write(root.join("tracked"), "dirty draft\n")?;
    open(&mut fx)?;
    fx.type_text("feature/proof")?;
    fx.keys("enter")?;
    fx.wait("dirty checkout guard", |fx| {
        fx.read(|root, cx| {
            root.dialog()
                .is_some_and(|d| d.read(cx).kind() == Kind::BranchConflict)
        })
    })?;
    ensure!(git(root, &["symbolic-ref", "HEAD"])? == "refs/heads/main");
    ensure!(fs::read_to_string(root.join("tracked"))? == "dirty draft\n");
    fx.shot()?
        .save(evidence.join("branches-dirty-protection.png"))?;
    fx.keys("enter")?;
    fx.wait("commit before switch", |fx| {
        fx.read(|root, cx| {
            root.dialog()
                .is_some_and(|d| d.read(cx).kind() == Kind::Commit)
        })
    })?;
    fx.type_text("Preserve the work before switching")?;
    fs::write(root.join(".git/index.lock"), "")?;
    fx.keys("cmd-enter")?;
    fx.wait("failed commit keeps its draft", |fx| {
        fx.read(|root, cx| {
            root.dialog().is_some_and(|d| !d.read(cx).saving())
                && root
                    .toasts(cx)
                    .iter()
                    .any(|t| t == "Failed to commit changes")
        })
    })?;
    ensure!(
        fx.read(|root, cx| root.dialog().unwrap().read(cx).query().to_owned())
            == "Preserve the work before switching"
    );
    ensure!(git(root, &["symbolic-ref", "HEAD"])? == "refs/heads/main");
    fx.shot()?
        .save(evidence.join("branches-commit-failure.png"))?;
    fs::remove_file(root.join(".git/index.lock"))?;
    fx.keys("cmd-enter")?;
    closed(&mut fx)?;
    ensure!(git(root, &["symbolic-ref", "HEAD"])? == "refs/heads/feature/proof");
    ensure!(git(root, &["show", "refs/heads/main:tracked"])? == "dirty draft");
    ensure!(git(root, &["rev-parse", "HEAD"])? == feature);
    ensure!(git(remote.path(), &["rev-parse", "refs/heads/main"])? == base);
    ensure!(fx.composer_text()? == "Keep this unsent draft");
    fx.shot()?.save(evidence.join("branches-switched.png"))?;
    git(root, &["checkout", "-q", "--detach", &base])?;
    fs::write(root.join("tracked"), "detached draft\n")?;
    open(&mut fx)?;
    fx.type_text("feature/proof")?;
    fx.keys("enter")?;
    fx.wait("detached dirty guard", |fx| {
        fx.read(|root, cx| {
            root.dialog()
                .is_some_and(|d| d.read(cx).kind() == Kind::BranchConflict)
        })
    })?;
    fx.keys("enter")?;
    fx.wait("detached commit form", |fx| {
        fx.read(|root, cx| {
            root.dialog()
                .is_some_and(|d| d.read(cx).kind() == Kind::Commit)
        })
    })?;
    let (new, generated) = target(&fx);
    ensure!(new && generated.starts_with("codex/"));
    fx.shot()?
        .save(evidence.join("branches-detached-commit.png"))?;
    fx.type_text("Keep the detached work")?;
    fx.keys("cmd-enter")?;
    closed(&mut fx)?;
    ensure!(git(root, &["symbolic-ref", "HEAD"])? == "refs/heads/feature/proof");
    ensure!(git(root, &["show", &format!("{generated}:tracked")])? == "detached draft");
    ensure!(fx.composer_text()? == "Keep this unsent draft");
    open(&mut fx)?;
    fx.keys("tab enter cmd-a")?;
    fx.type_text("heads/main")?;
    fx.keys("enter")?;
    closed(&mut fx)?;
    ensure!(git(root, &["symbolic-ref", "HEAD"])? == "refs/heads/heads/main");
    open(&mut fx)?;
    fx.type_text("main")?;
    fx.keys("down enter")?;
    closed(&mut fx)?;
    ensure!(git(root, &["symbolic-ref", "HEAD"])? == "refs/heads/main");
    fs::rename(root.join(".git"), home.path().join("saved-git"))?;
    git(root, &["init", "-q", "-b", "main"])?;
    open(&mut fx)?;
    fx.keys("tab enter")?;
    ensure!(fx.read(|root, cx| root.dialog().unwrap().read(cx).kind()) == Kind::Branches);
    ensure!(git(root, &["rev-parse", "--verify", "HEAD"]).is_err());
    fx.keys("escape")?;
    println!(
        "PASS: native search and clicked branch selection; current, detached, valid/invalid/existing names; preserved branch/message/composer drafts; dirty guard, failed commit, real commit-and-switch; local remote unchanged"
    );
    Ok(())
}
