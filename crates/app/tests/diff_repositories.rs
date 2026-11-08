#![cfg(unix)] // shell-script fixtures and Unix permission bits
#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

#[cfg(target_os = "macos")]
use anyhow::Context as _;
use anyhow::{Result, ensure};
use app::host::{Host as _, LocalHost, Request};
use serde_json::json;
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn review_choices_keep_assigned_roots_and_owned_worktrees() -> Result<()> {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    let directory = tempfile::tempdir()?;
    let a = directory.path().join("repo-a");
    let b = directory.path().join("repo-b");
    let other = directory.path().join("unrelated");
    for root in [&a, &b, &other] {
        fs::create_dir_all(root.join("src"))?;
        git(root, &["init", "-q"])?;
        git(root, &["config", "user.name", "Repository proof"])?;
        git(root, &["config", "user.email", "proof@example.invalid"])?;
        fs::write(root.join("src/file.rs"), "fn value() {}\n")?;
        git(root, &["add", "."])?;
        git(root, &["commit", "-qm", "base"])?;
    }
    let home = directory.path().join("home");
    fs::create_dir_all(&home)?;
    let mut state = json!({
        "local-projects": {
            "storage-selected": {"id":"selected","name":"Selected","rootPaths":[a,b,a],"createdAt":1,"updatedAt":1},
            "other": {"id":"other","name":"Other","rootPaths":[other],"createdAt":1,"updatedAt":1}
        },
        "thread-project-assignments": {"thread":{"projectKind":"local","projectId":"selected"}}
    });
    fs::write(
        home.join(".codex-global-state.json"),
        serde_json::to_vec(&state)?,
    )?;
    let owned = home.join("worktrees/owned/repo-a");
    let foreign = home.join("worktrees/foreign/repo-a");
    fs::create_dir_all(owned.parent().unwrap())?;
    fs::create_dir_all(foreign.parent().unwrap())?;
    git(
        &a,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            owned.to_str().unwrap(),
            "HEAD",
        ],
    )?;
    git(
        &a,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            foreign.to_str().unwrap(),
            "HEAD",
        ],
    )?;
    let host = LocalHost::new(&home);
    for (root, thread) in [(&owned, "thread"), (&foreign, "another-thread")] {
        host.fetch(&Request::post(
            "set-worktree-owner-thread",
            Some(json!({
                "worktree":root,"conversationId":thread,
            })),
        ))?;
    }
    let choices = host.review_repositories("thread", a.join("src").to_str().unwrap())?;
    let roots = choices
        .iter()
        .map(|choice| Path::new(&choice.root).canonicalize())
        .collect::<std::io::Result<Vec<_>>>()?;
    ensure!(
        roots == [a.canonicalize()?, b.canonicalize()?, owned.canonicalize()?],
        "wrong repository ownership: {roots:?}"
    );
    ensure!(Path::new(&choices[0].cwd).canonicalize()? == a.join("src").canonicalize()?);
    ensure!(Path::new(&choices[2].cwd).canonicalize()? == owned.join("src").canonicalize()?);
    let nested = a.join("nested");
    fs::create_dir_all(nested.join("src"))?;
    git(&nested, &["init", "-q"])?;
    git(&nested, &["config", "user.name", "Repository proof"])?;
    git(&nested, &["config", "user.email", "proof@example.invalid"])?;
    fs::write(nested.join("src/file.rs"), "fn nested() {}\n")?;
    git(&nested, &["add", "."])?;
    git(&nested, &["commit", "-qm", "base"])?;
    let nested_choices =
        host.review_repositories("thread", nested.join("src").to_str().unwrap())?;
    ensure!(
        nested_choices
            .iter()
            .any(|choice| Path::new(&choice.root) == nested.canonicalize().unwrap())
    );
    let outer = nested_choices
        .iter()
        .find(|choice| Path::new(&choice.root) == a.canonicalize().unwrap())
        .unwrap();
    ensure!(Path::new(&outer.cwd) == a.canonicalize()?);
    state["local-projects"]["storage-selected"]["rootPaths"] = json!([directory.path()]);
    fs::write(
        home.join(".codex-global-state.json"),
        serde_json::to_vec(&state)?,
    )?;
    let parent_choices =
        host.review_repositories("thread", nested.join("src").to_str().unwrap())?;
    ensure!(parent_choices.len() == 1);
    ensure!(Path::new(&parent_choices[0].root) == nested.canonicalize()?);
    #[cfg(unix)]
    {
        let alias = directory.path().join("alias-b");
        std::os::unix::fs::symlink(b.join("src"), &alias)?;
        let linked_worktree = home.join("worktrees/linked/repo-b");
        fs::create_dir_all(linked_worktree.parent().unwrap())?;
        git(
            &b,
            &[
                "worktree",
                "add",
                "--quiet",
                "--detach",
                linked_worktree.to_str().unwrap(),
                "HEAD",
            ],
        )?;
        host.fetch(&Request::post(
            "set-worktree-owner-thread",
            Some(json!({
                "worktree":linked_worktree,"conversationId":"thread"
            })),
        ))?;
        state["local-projects"]["storage-selected"]["rootPaths"] = json!([a, alias]);
        fs::write(
            home.join(".codex-global-state.json"),
            serde_json::to_vec(&state)?,
        )?;
        let linked_choices = host.review_repositories("thread", a.join("src").to_str().unwrap())?;
        let second = linked_choices
            .iter()
            .find(|choice| Path::new(&choice.root) == b.canonicalize().unwrap())
            .unwrap();
        ensure!(Path::new(&second.cwd) == b.join("src").canonicalize()?);
        let linked = linked_choices
            .iter()
            .find(|choice| Path::new(&choice.root) == linked_worktree.canonicalize().unwrap())
            .unwrap();
        ensure!(Path::new(&linked.cwd) == linked_worktree.join("src").canonicalize()?);
    }
    ensure!(
        host.review_repositories("missing", directory.path().join("absent").to_str().unwrap())?
            .is_empty()
    );
    Ok(())
}

#[cfg(target_os = "macos")]
fn choose(fx: &mut common::Fx, label: &str) -> Result<()> {
    let index = fx
        .read(|root, cx| {
            root.diff_panel().and_then(|panel| {
                panel
                    .read(cx)
                    .menu_items()
                    .position(|(text, enabled)| text == label && enabled)
            })
        })
        .ok_or_else(|| anyhow::anyhow!("missing menu item {label}"))?;
    fx.keys("home")?;
    for _ in 0..index {
        fx.keys("down")?;
    }
    fx.keys("enter")
}

#[cfg(target_os = "macos")]
fn loaded(fx: &mut common::Fx) -> Result<()> {
    fx.wait("selected repository", |fx| {
        fx.read(|root, cx| {
            root.diff_panel()
                .is_some_and(|panel| !panel.read(cx).pending())
        })
    })
}

#[cfg(target_os = "macos")]
fn search(fx: &mut common::Fx, query: &str) -> Result<usize> {
    fx.click((1000., 177.))?;
    fx.keys("cmd-f cmd-a")?;
    fx.type_text(query)?;
    let count = fx.read(|root, cx| {
        root.diff_panel()
            .unwrap()
            .read(cx)
            .search()
            .map_or(0, |(_, count)| count)
    });
    fx.keys("escape")?;
    Ok(count)
}

#[cfg(target_os = "macos")]
#[test]
fn repository_branch_and_commit_pickers_load_the_chosen_files() -> Result<()> {
    use common::{Fx, SUBTRACT};
    use std::{path::PathBuf, sync::Arc};
    common::install_panic_hook();
    let out = std::env::var_os("CGX_DIFF_REVIEW_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    fs::create_dir_all(&out)?;
    let fixture = tempfile::tempdir_in(&out)?;
    let a = fixture.path().join("repo-a");
    let b = fixture.path().join("repo-b");
    for root in [&a, &b] {
        fs::create_dir_all(root.join("src"))?;
        git(root, &["init", "-q", "-b", "main"])?;
        git(root, &["config", "user.name", "Picker proof"])?;
        git(root, &["config", "user.email", "proof@example.invalid"])?;
        fs::write(
            root.join("src/file.rs"),
            "pub fn value() -> &'static str { \"base\" }\n",
        )?;
        git(root, &["add", "."])?;
        git(root, &["commit", "-qm", "base"])?;
        git(root, &["branch", "feature/needle"])?;
        fs::write(
            root.join("src/file.rs"),
            "pub fn value() -> &'static str { \"committed\" }\n",
        )?;
        git(
            root,
            &[
                "commit",
                "-qam",
                &format!("change {}", root.file_name().unwrap().to_string_lossy()),
            ],
        )?;
        fs::write(
            root.join("src/file.rs"),
            "pub fn value() -> &'static str { \"working\" }\n",
        )?;
    }
    let home = fixture.path().join("home");
    fs::create_dir_all(&home)?;
    fs::write(
        home.join(".codex-global-state.json"),
        serde_json::to_vec(&json!({
            "local-projects":{"project":{"id":"project","name":"Project","rootPaths":[a,b],"createdAt":1,"updatedAt":1}},
            "thread-project-assignments":{(SUBTRACT):{"projectKind":"local","projectId":"project"}}
        }))?,
    )?;
    let owned = home.join("worktrees/calm-sparrow/repo-a");
    fs::create_dir_all(owned.parent().context("worktree parent")?)?;
    git(
        &a,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            owned.to_str().context("worktree path")?,
            "HEAD",
        ],
    )?;
    fs::write(
        owned.join("src/file.rs"),
        "pub fn value() -> &'static str { \"worktree\" }\n",
    )?;
    let host = Arc::new(LocalHost::new(&home));
    host.fetch(&Request::post(
        "set-worktree-owner-thread",
        Some(json!({
            "worktree":owned,"conversationId":SUBTRACT,
        })),
    ))?;
    let mut script = common::script(None, |_| {})?;
    for row in &mut script {
        if let Some(thread) = row.pointer_mut("/msg/result/thread") {
            thread["cwd"] = a.to_string_lossy().into_owned().into();
        }
    }
    let mut fx = Fx::scripted(script)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.keys("cmd-2")?;
    fx.wait_thread(SUBTRACT)?;
    fx.keys("alt-cmd-b")?;
    fx.wait("repository choices", |fx| {
        fx.read(|root, cx| root.diff_panel().unwrap().read(cx).repositories().len() == 3)
    })?;
    fx.click((710., 68.))?;
    choose(&mut fx, "Calm sparrow")?;
    loaded(&mut fx)?;
    ensure!(
        search(&mut fx, "worktree")? == 1,
        "The owned worktree did not load its working file"
    );
    fx.reference_shot()?
        .save(out.join("repository-worktree.png"))?;
    fx.click((710., 68.))?;
    choose(&mut fx, "repo-b")?;
    loaded(&mut fx)?;
    ensure!(
        search(&mut fx, "working")? == 1,
        "The selected repository did not load its working file"
    );
    fx.reference_shot()?.save(out.join("repository-b.png"))?;
    fx.click((800., 68.))?;
    choose(&mut fx, "Branch")?;
    loaded(&mut fx)?;
    fx.click((880., 68.))?;
    fx.type_text("needle")?;
    choose(&mut fx, "feature/needle")?;
    loaded(&mut fx)?;
    ensure!(
        search(&mut fx, "working")? == 0 && search(&mut fx, "committed")? == 1,
        "The branch picker kept the worktree diff"
    );
    fx.reference_shot()?
        .save(out.join("branch-comparison.png"))?;
    fx.click((790., 68.))?;
    choose(&mut fx, "Committed")?;
    loaded(&mut fx)?;
    let commit = fx
        .read(|root, cx| {
            root.diff_panel()
                .unwrap()
                .read(cx)
                .menu_items()
                .find(|(label, _)| label.ends_with("change repo-b"))
                .map(|(label, _)| label.to_owned())
        })
        .ok_or_else(|| anyhow::anyhow!("missing commit choice"))?;
    choose(&mut fx, &commit)?;
    loaded(&mut fx)?;
    ensure!(
        search(&mut fx, "working")? == 0 && search(&mut fx, "committed")? == 1,
        "The commit picker kept the worktree diff"
    );
    fx.reference_shot()?
        .save(out.join("commit-comparison.png"))?;
    fx.click((790., 68.))?;
    choose(&mut fx, "Last Turn")?;
    ensure!(fx.read(
        |root, cx| root.diff_panel().unwrap().read(cx).review_params()["reviewSource"]
            == "last-turn"
    ));
    Ok(())
}
