use anyhow::{Context as _, Result, ensure};
use app::host::{Host, LocalHost, Request, json_body};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

#[test]
fn native_owner_request_persists_only_in_a_linked_worktree() -> Result<()> {
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    fs::create_dir(&source)?;
    git(&source, &["init", "-q", "-b", "main"])?;
    git(&source, &["config", "user.name", "Worktree proof"])?;
    git(&source, &["config", "user.email", "proof@example.invalid"])?;
    fs::write(source.join("tracked"), "base\n")?;
    git(&source, &["add", "tracked"])?;
    git(&source, &["commit", "-qm", "Base"])?;
    let host = LocalHost::new(fixture.path().join("home"));
    let created = json_body(&host.fetch(&Request::post(
        "create-worktree",
        Some(json!({"cwd": source})),
    ))?)?;
    let worktree = Path::new(
        created["worktreeGitRoot"]
            .as_str()
            .context("missing worktree")?,
    );
    let assigned = json_body(&host.fetch(&Request::post(
        "worktree-set-owner-thread",
        Some(json!({"hostId": "local", "worktree": worktree, "conversationId": "owned-thread"})),
    ))?)?;
    ensure!(assigned["success"] == true);
    let owner_path = git(
        worktree,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "codex-thread.json",
        ],
    )?;
    let owner: Value = serde_json::from_slice(&fs::read(owner_path)?)?;
    ensure!(owner == json!({"version": 1, "ownerThreadId": "owned-thread"}));
    ensure!(
        host.fetch(&Request::post(
            "worktree-set-owner-thread",
            Some(json!({
                "worktree": source, "conversationId": "wrong-target",
            }))
        ))
        .is_err()
    );
    ensure!(!source.join(".git/codex-thread.json").exists());
    Ok(())
}
