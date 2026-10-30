#![cfg(target_os = "macos")]
//! Slash commands that run existing chat actions: `/compact`, `/fork`, `/rename`, `/pin`,
//! `/model`, `/reasoning` and `/project`. Each test types the command, presses Enter and
//! compares the requests the app-server received (`Fx::captured`) and the resulting UI.

#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host, LocalHost};
use common::{Fx, GREETING, SUBTRACT, exchange, recorded_thread, script};
use serde_json::{Value, json};
use std::sync::Arc;

#[test]
fn proxy() {
    common::proxy_child();
}

fn running(line: &mut Value) {
    if line.pointer("/msg/result/thread/id") == Some(&json!(GREETING))
        && let Some(status) = line.pointer_mut("/msg/result/thread/turns/0/status")
    {
        *status = json!("inProgress");
    }
}

fn captured(lines: Vec<Value>) -> Result<Fx> {
    Fx::captured("dark", (1280., 832.), lines)
}

/// Saves the window for the before/after review (`CGX_SLASH_SHOTS` names the directory).
fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    if let Some(dir) = std::env::var_os("CGX_SLASH_SHOTS") {
        fx.shot()?
            .save(std::path::Path::new(&dir).join(format!("{name}.png")))?;
    }
    Ok(())
}

/// Opens the recorded chat and focuses its docked composer.
fn open_chat(fx: &mut Fx) -> Result<()> {
    fx.keys("cmd-1")?;
    fx.wait_thread(GREETING)?;
    fx.click(fx.at("thread", "Work with Codex")?)?;
    ensure!(fx.composer_focused()?, "the composer must have focus");
    Ok(())
}

fn rows(fx: &Fx) -> Result<Vec<String>> {
    let (rows, _) = fx
        .read(|root, cx| root.composer(cx).read(cx).slash_state())
        .context("slash menu must be open")?;
    Ok(rows
        .into_iter()
        .map(|row| row.split('\n').next().unwrap().to_owned())
        .collect())
}

fn requests(fx: &mut Fx, method: &str) -> Result<Vec<Value>> {
    Ok(fx
        .sent()?
        .into_iter()
        .filter(|request| request["method"] == method)
        .collect())
}

fn config_write(fx: &mut Fx) -> Result<Value> {
    let writes = requests(fx, "config/batchWrite")?;
    Ok(writes.last().context("config/batchWrite")?["params"]["edits"].clone())
}

#[test]
fn compact_starts_on_the_open_chat_and_reports_a_refusal() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "thread/resume",
        json!({"thread": recorded_thread(GREETING)?}),
    ));
    // No recorded `thread/compact/start`: the replay answers the first one with an error.
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("/compact")?;
    ensure!(rows(&fx)? == ["Compact"]);
    fx.keys("enter")?;
    fx.wait("failed compact toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|t| t == "Failed to compact chat")
        })
    })?;
    let starts = requests(&mut fx, "thread/compact/start")?;
    ensure!(starts.len() == 1 && starts[0]["params"] == json!({"threadId": GREETING}));
    ensure!(fx.composer_text()?.is_empty() && fx.composer_focused()?);

    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "thread/resume",
        json!({"thread": recorded_thread(GREETING)?}),
    ));
    lines.extend(exchange("thread/compact/start", json!({})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("/compact")?;
    fx.keys("enter")?;
    fx.wait("compact request", |fx| {
        requests(fx, "thread/compact/start").is_ok_and(|r| r.len() == 1)
    })?;
    ensure!(fx.read(|root, cx| root.toasts(cx).is_empty()));
    Ok(())
}

#[test]
fn compact_is_refused_while_a_turn_runs() -> Result<()> {
    let mut lines = script(None, running)?;
    lines.extend(exchange("thread/compact/start", json!({})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("/compact")?;
    fx.keys("enter")?;
    ensure!(fx.read(|root, cx| {
        root.toasts(cx)
            .iter()
            .any(|t| t == "Compact is disabled while a chat is in progress")
    }));
    ensure!(requests(&mut fx, "thread/compact/start")?.is_empty());
    Ok(())
}

#[test]
fn rename_opens_the_dialog_and_returns_focus_to_the_composer() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/name/set", json!({})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("/rename")?;
    fx.keys("enter")?;
    ensure!(
        fx.read(|root, _| root.dialog().is_some()),
        "Rename chat opens"
    );
    fx.keys("escape")?;
    ensure!(fx.read(|root, _| root.dialog().is_none()));
    ensure!(
        fx.composer_focused()?,
        "Cancel returns focus to the composer"
    );
    ensure!(requests(&mut fx, "thread/name/set")?.is_empty());

    fx.type_text("/rename")?;
    fx.keys("enter")?;
    fx.type_text("Greeting tweak")?;
    fx.keys("enter")?;
    fx.wait("name write", |fx| {
        requests(fx, "thread/name/set").is_ok_and(|r| r.len() == 1)
    })?;
    ensure!(
        requests(&mut fx, "thread/name/set")?[0]["params"]
            == json!({"threadId": GREETING, "name": "Greeting tweak"})
    );
    ensure!(fx.composer_focused()?, "Save returns focus to the composer");
    Ok(())
}

#[test]
fn pin_moves_the_chat_into_and_out_of_the_pinned_section() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/section/move", json!({})));
    lines.extend(exchange("thread/section/move", json!({})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    for (n, expected) in [Some(ui::screens::sidebar::PINNED_SECTION_ID), None]
        .into_iter()
        .enumerate()
    {
        fx.type_text("/pin")?;
        fx.keys("enter")?;
        fx.wait("section move", |fx| {
            requests(fx, "thread/section/move").is_ok_and(|r| r.len() == n + 1)
        })?;
        let moves = requests(&mut fx, "thread/section/move")?;
        ensure!(
            moves.last().unwrap()["params"]["sectionId"] == json!(expected),
            "pin toggles: {moves:?}"
        );
        ensure!(moves.last().unwrap()["params"]["threadId"] == GREETING);
    }
    Ok(())
}

#[test]
fn fork_submenu_forks_in_the_same_workspace_and_opens_the_fork() -> Result<()> {
    let mut forked = recorded_thread(SUBTRACT)?;
    forked["id"] = "forked-chat".into();
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/fork", json!({"thread": forked})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("/fork")?;
    fx.keys("enter")?;
    ensure!(
        fx.composer_text()? == "/fork ",
        "the token opens its submenu"
    );
    ensure!(rows(&fx)? == ["Fork chat", "Fork into new worktree"]);
    shot(&mut fx, "fork-submenu")?;
    ensure!(
        requests(&mut fx, "thread/fork")?.is_empty(),
        "opening is not a pick"
    );
    // Backspace over the space returns to the command list.
    fx.keys("backspace")?;
    ensure!(rows(&fx)? == ["Fork chat"] && fx.composer_text()? == "/fork");
    fx.keys("enter enter")?;
    fx.wait_thread("forked-chat")?;
    let forks = requests(&mut fx, "thread/fork")?;
    ensure!(forks.len() == 1 && forks[0]["params"] == json!({"threadId": GREETING}));
    ensure!(requests(&mut fx, "thread/rollback")?.is_empty());
    Ok(())
}

#[test]
fn fork_from_here_uses_the_selected_server_boundary() -> Result<()> {
    let mut thread = recorded_thread(SUBTRACT)?;
    thread["id"] = "bounded-fork".into();
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("thread/fork", json!({"thread": thread})));
    let mut fx = captured(lines)?;
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
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
    fx.click(fx.at("thread", "Fork chat from here")?)?;
    fx.wait_thread("bounded-fork")?;
    let forks = requests(&mut fx, "thread/fork")?;
    ensure!(forks[0]["params"] == json!({"threadId": SUBTRACT, "lastTurnId": turn}));
    ensure!(requests(&mut fx, "thread/rollback")?.is_empty());
    Ok(())
}

#[test]
fn fork_failure_is_reported_and_keeps_the_chat() -> Result<()> {
    let mut fx = captured(script(None, |_| {})?)?;
    open_chat(&mut fx)?;
    fx.type_text("/fork")?;
    fx.keys("enter enter")?;
    fx.wait("fork toast", |fx| {
        fx.read(|root, cx| root.toasts(cx).iter().any(|t| t == "Failed to fork chat"))
    })?;
    ensure!(fx.route() == app::Route::Thread(GREETING.into()));
    Ok(())
}

#[test]
fn model_submenu_filters_keeps_the_draft_and_writes_the_pick() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("config/batchWrite", json!({})));
    lines.extend(exchange("config/batchWrite", json!({})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("draft /model")?;
    shot(&mut fx, "model-before-command-list")?;
    fx.keys("enter")?;
    ensure!(fx.composer_text()? == "draft /model ");
    ensure!(rows(&fx)?.len() == 7, "every model is offered");
    fx.type_text("luna")?;
    ensure!(
        rows(&fx)? == ["GPT-6 Luna", "GPT-5.6 Luna"],
        "{:?}",
        rows(&fx)?
    );
    shot(&mut fx, "model-submenu-filtered")?;
    fx.keys("down")?;
    fx.keys("enter")?;
    shot(&mut fx, "model-after-pick")?;
    ensure!(fx.composer_text()? == "draft " && fx.composer_focused()?);
    fx.wait("model write", |fx| {
        requests(fx, "config/batchWrite").is_ok_and(|r| !r.is_empty())
    })?;
    let edits = config_write(&mut fx)?;
    ensure!(
        edits[0]["keyPath"] == "model" && edits[0]["value"] == "gpt-5.6-luna",
        "{edits}"
    );

    // Escape closes the submenu and leaves the typed text.
    fx.keys("cmd-a backspace")?;
    fx.type_text("/model lu")?;
    fx.keys("escape")?;
    ensure!(fx.read(|root, cx| root.composer(cx).read(cx).slash_state().is_none()));
    ensure!(fx.composer_text()? == "/model lu");
    Ok(())
}

#[test]
fn reasoning_submenu_writes_the_picked_effort() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("config/batchWrite", json!({})));
    let mut fx = captured(lines)?;
    open_chat(&mut fx)?;
    fx.type_text("/reasoning")?;
    fx.keys("enter")?;
    shot(&mut fx, "reasoning-submenu")?;
    ensure!(
        rows(&fx)? == ["Light", "Medium", "High", "Extra High", "Ultra"],
        "{:?}",
        rows(&fx)?
    );
    fx.type_text("extra")?;
    fx.keys("enter")?;
    fx.wait("effort write", |fx| {
        requests(fx, "config/batchWrite").is_ok_and(|r| !r.is_empty())
    })?;
    let edits = config_write(&mut fx)?;
    ensure!(
        edits[1]["keyPath"] == "model_reasoning_effort" && edits[1]["value"] == "xhigh",
        "{edits}"
    );
    ensure!(fx.composer_text()?.is_empty() && fx.composer_focused()?);
    Ok(())
}

#[test]
fn project_submenu_selects_an_existing_project_for_the_next_chat() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let host = Arc::new(LocalHost::new(temp.path().join("codex")));
    let [alpha, beta] = ["alpha", "beta"].map(|name| temp.path().join(name));
    std::fs::create_dir(&alpha)?;
    std::fs::create_dir(&beta)?;
    let [_, beta] = [alpha, beta].map(|root| host.add_project(&root).unwrap());
    let mut new = recorded_thread(GREETING)?;
    new["id"] = "in-beta".into();
    new["cwd"] = json!(beta);
    new["turns"] = json!([]);
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "thread/start",
        json!({"thread": new, "model": "gpt-6.1-sol", "cwd": beta}),
    ));
    lines.extend(exchange(
        "turn/start",
        json!({"turn": {"id": "t", "items": [], "status": "completed", "error": null}}),
    ));
    let mut fx = captured(lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.wait("project rows", |fx| {
        fx.read(|root, cx| root.sidebar().read(cx).projects().len() == 2)
    })?;
    ensure!(fx.route() == app::Route::Home);
    fx.click(fx.at("home", "Do anything")?)?;
    fx.type_text("/project")?;
    fx.keys("enter")?;
    ensure!(rows(&fx)?.len() == 2, "{:?}", rows(&fx)?);
    fx.type_text("bet")?;
    ensure!(rows(&fx)? == ["beta"]);
    shot(&mut fx, "project-submenu")?;
    fx.keys("enter")?;
    ensure!(fx.composer_text()?.is_empty() && fx.composer_focused()?);
    fx.type_text("Start here")?;
    fx.keys("enter")?;
    fx.wait_thread("in-beta")?;
    let starts = requests(&mut fx, "thread/start")?;
    ensure!(
        starts.len() == 1 && starts[0]["params"]["cwd"] == json!(beta),
        "{starts:?}"
    );
    Ok(())
}

fn git(root: &std::path::Path, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .output()?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn worktree_project() -> Result<(tempfile::TempDir, Arc<LocalHost>, std::path::PathBuf)> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("project");
    std::fs::create_dir_all(source.join("nested"))?;
    git(&source, &["init", "-b", "main"])?;
    std::fs::write(source.join("nested/branch.txt"), "main")?;
    git(&source, &["add", "."])?;
    git(
        &source,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "main",
        ],
    )?;
    git(&source, &["checkout", "-b", "selected"])?;
    std::fs::write(source.join("nested/branch.txt"), "selected")?;
    git(
        &source,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-am",
            "selected",
        ],
    )?;
    let host = Arc::new(LocalHost::new(temp.path().join("codex")));
    let project = source.join("nested");
    host.add_project(&project)?;
    Ok((temp, host, project))
}

fn managed_worktrees(host: &LocalHost) -> Result<Value> {
    app::host::json_body(&host.fetch(&app::host::Request::post("codex-worktrees", None))?)
}

fn assert_worktree_owner(cwd: &std::path::Path, thread: &str) -> Result<()> {
    ensure!(std::fs::read_to_string(cwd.join("branch.txt"))? == "selected");
    let file = git(
        cwd,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "codex-thread.json",
        ],
    )?;
    let owner: Value = serde_json::from_slice(&std::fs::read(file)?)?;
    ensure!(owner["ownerThreadId"] == thread, "{owner}");
    Ok(())
}

#[test]
fn normal_composer_worktree_owns_the_selected_checkout_and_restores_failed_drafts() -> Result<()> {
    for accepted in [false, true] {
        let (_temp, host, project) = worktree_project()?;
        let mut lines = script(None, |_| {})?;
        if accepted {
            let mut thread = recorded_thread(GREETING)?;
            thread["id"] = "worktree-chat".into();
            thread["turns"] = json!([]);
            lines.extend(exchange(
                "thread/start",
                json!({
                    "thread": thread, "model": "gpt-6.1-sol", "cwd": project
                }),
            ));
            lines.extend(exchange(
                "turn/start",
                json!({"turn": {
                    "id": "turn", "items": [], "status": "completed", "error": null
                }}),
            ));
        }
        let mut fx = captured(lines)?;
        fx.root
            .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
        fx.action(Box::new(ui::screens::sidebar::SelectProject(
            project.to_string_lossy().into_owned(),
        )))?;
        fx.wait("Git project probe", |fx| {
            fx.read(|root, cx| root.home().read(cx).location.worktree_available == Some(true))
        })?;
        fx.focus_composer()?;
        fx.keys("shift-tab space")?;
        shot(&mut fx, "worktree-menu")?;
        fx.keys("down enter")?;
        shot(&mut fx, "worktree-selected")?;
        fx.focus_composer()?;
        fx.type_text("Keep this draft")?;
        fx.keys("enter")?;
        if accepted {
            fx.wait_thread("worktree-chat")?;
            let starts = requests(&mut fx, "thread/start")?;
            let cwd =
                std::path::Path::new(starts.last().unwrap()["params"]["cwd"].as_str().unwrap());
            ensure!(cwd != project);
            assert_worktree_owner(cwd, "worktree-chat")?;
        } else {
            fx.wait("restored draft", |fx| {
                fx.composer_text()
                    .is_ok_and(|text| text == "Keep this draft")
            })?;
            ensure!(requests(&mut fx, "thread/start")?.len() == 1);
            ensure!(
                managed_worktrees(&host)?["worktrees"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            ensure!(fx.route() == app::Route::Home);
        }
        ensure!(git(&project, &["branch", "--show-current"])? == "selected");
        ensure!(std::fs::read_to_string(project.join("branch.txt"))? == "selected");
    }
    Ok(())
}

#[test]
fn worktree_fork_keeps_the_source_transcript_and_cleans_up_a_refusal() -> Result<()> {
    for accepted in [false, true] {
        let (_temp, host, project) = worktree_project()?;
        let mut lines = script(None, |_| {})?;
        for line in &mut lines {
            if line.pointer("/msg/result/thread/id") == Some(&json!(GREETING)) {
                line["msg"]["result"]["thread"]["cwd"] = json!(project);
            }
        }
        let mut original = recorded_thread(GREETING)?;
        original["cwd"] = json!(project);
        if accepted {
            let mut forked = original.clone();
            forked["id"] = "worktree-fork".into();
            lines.extend(exchange("thread/fork", json!({"thread": forked})));
            let mut reread = exchange("thread/read", json!({"thread": original}));
            reread[0]["msg"]["params"] = json!({"threadId": GREETING, "includeTurns": true});
            lines.extend(reread);
        }
        let mut fx = captured(lines)?;
        fx.root
            .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
        open_chat(&mut fx)?;
        let source_turns =
            fx.read(|root, cx| format!("{:?}", root.thread().unwrap().read(cx).turns()));
        fx.type_text("/fork")?;
        fx.keys("enter down")?;
        if accepted {
            fx.composer().update(&mut fx.cx, |_, cx| {
                cx.emit(ui::composer::SlashCommand::ForkWorktree);
                cx.emit(ui::composer::SlashCommand::ForkWorktree);
            });
        } else {
            fx.keys("enter")?;
        }
        if accepted {
            fx.wait_thread("worktree-fork")?;
            let forks = requests(&mut fx, "thread/fork")?;
            ensure!(forks.len() == 1);
            let params = &forks.last().unwrap()["params"];
            ensure!(params["threadId"] == GREETING);
            let cwd = std::path::Path::new(params["cwd"].as_str().unwrap());
            ensure!(cwd != project);
            assert_worktree_owner(cwd, "worktree-fork")?;
            ensure!(
                fx.read(|root, cx| format!("{:?}", root.thread().unwrap().read(cx).turns()))
                    == source_turns
            );
            fx.root.update(&mut fx.cx, |root, cx| {
                root.sidebar().update(cx, |_, cx| {
                    cx.emit(ui::screens::sidebar::OpenThread(GREETING.into()));
                });
            });
            fx.wait_thread(GREETING)?;
        } else {
            fx.wait("fork refusal", |fx| {
                fx.read(|root, cx| {
                    root.toasts(cx)
                        .iter()
                        .any(|toast| toast == "Failed to fork chat")
                })
            })?;
            ensure!(
                managed_worktrees(&host)?["worktrees"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        ensure!(fx.route() == app::Route::Thread(GREETING.into()));
        ensure!(
            fx.read(|root, cx| format!("{:?}", root.thread().unwrap().read(cx).turns()))
                == source_turns
        );
        ensure!(requests(&mut fx, "thread/rollback")?.is_empty());
        ensure!(git(&project, &["branch", "--show-current"])? == "selected");
    }
    Ok(())
}

#[test]
fn temporary_chat_does_not_reuse_the_home_worktree_choice() -> Result<()> {
    let (_temp, host, project) = worktree_project()?;
    let mut fx = captured(script(None, |_| {})?)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.action(Box::new(ui::screens::sidebar::SelectProject(
        project.to_string_lossy().into_owned(),
    )))?;
    fx.wait("Git project probe", |fx| {
        fx.read(|root, cx| root.home().read(cx).location.worktree_available == Some(true))
    })?;
    fx.focus_composer()?;
    fx.keys("shift-tab space down enter")?;
    let action = fx
        .cx
        .update(|cx| cx.build_action("app::NewTemporaryChat", None))?;
    fx.action(action)?;
    fx.focus_composer()?;
    fx.type_text("temporary draft")?;
    fx.keys("enter")?;
    fx.wait("restored temporary draft", |fx| {
        fx.composer_text()
            .is_ok_and(|text| text == "temporary draft")
    })?;
    let starts = requests(&mut fx, "thread/start")?;
    ensure!(starts.len() == 1);
    ensure!(starts[0]["params"]["ephemeral"] == true);
    ensure!(starts[0]["params"]["cwd"] == json!(project));
    ensure!(
        managed_worktrees(&host)?["worktrees"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    Ok(())
}

#[test]
fn worktree_choice_stays_disabled_for_a_project_without_git() -> Result<()> {
    let (_temp, host, project) = worktree_project()?;
    std::fs::remove_dir_all(project.parent().unwrap().join(".git"))?;
    let mut fx = captured(script(None, |_| {})?)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.action(Box::new(ui::screens::sidebar::SelectProject(
        project.to_string_lossy().into_owned(),
    )))?;
    fx.wait("non-Git project probe", |fx| {
        fx.read(|root, cx| root.home().read(cx).location.worktree_available == Some(false))
    })?;
    fx.focus_composer()?;
    fx.keys("shift-tab space down enter")?;
    ensure!(
        fx.read(|root, cx| root.home().read(cx).location.mode) == ui::screens::hotkey::Mode::Local
    );
    Ok(())
}
