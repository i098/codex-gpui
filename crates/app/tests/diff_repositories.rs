#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

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
