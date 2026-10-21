//! AM-021 prerequisite: the original git worker's `create-worktree` and
//! `set-worktree-owner-thread` through `LocalHost`, against task-owned repositories and a local
//! bare remote. Nothing here reaches a real remote or a normal repository.

use anyhow::{Context as _, Result, ensure};
use app::host::{Host as _, LocalHost, Request, json_body};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "-c",
            "user.name=Codex",
            "-c",
            "user.email=codex@example.com",
        ])
        .args(args)
        .output()?;
    ensure!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn call(host: &LocalHost, endpoint: &str, params: Value) -> Result<Value> {
    json_body(&host.fetch(&Request::post(endpoint, Some(params)))?)
}

fn write(path: &Path, text: &str) -> Result<()> {
    fs::create_dir_all(path.parent().context("parent")?)?;
    Ok(fs::write(path, text)?)
}

/// Everything a user could lose in the source checkout.
fn snapshot(source: &Path) -> Result<(String, String, String, String)> {
    Ok((
        git(source, &["status", "--porcelain=v1", "-uall", "--ignored"])?,
        git(source, &["symbolic-ref", "HEAD"])?,
        git(
            source,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/heads",
            ],
        )?,
        fs::read_to_string(source.join("sub/tracked.txt"))?
            + &fs::read_to_string(source.join("sub/draft.txt"))?,
    ))
}

fn reservations(root: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root)?
        .map(|entry| Ok(entry?.path()))
        .collect::<Result<_>>()?;
    dirs.retain(|path| path.is_dir());
    Ok(dirs)
}

/// An origin whose `main` moved after the source last pulled; the source has dirty work on
/// branch `feature`, ignored files, and a tracked local environment.
fn fixture(dir: &Path) -> Result<(PathBuf, String, String)> {
    let origin = dir.join("origin.git");
    git(
        dir,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            origin.to_str().unwrap(),
        ],
    )?;
    let source = dir.join("project");
    git(
        dir,
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            source.to_str().unwrap(),
        ],
    )?;
    write(
        &source.join(".gitignore"),
        ".env\nsecret.txt\nAGENTS.override.md\nsetup-*.txt\n",
    )?;
    write(&source.join(".worktreeinclude"), ".env\n")?;
    write(&source.join("sub/tracked.txt"), "base\n")?;
    write(
        &source.join(".codex/environments/environment.toml"),
        "version = 1\nname = \"test\"\n\n[setup]\nscript = \"\"\"\n\
         printf '%s' \"$CODEX_SOURCE_TREE_PATH\" > setup-source.txt\n\
         printf '%s' \"$CODEX_WORKTREE_PATH\" > setup-worktree.txt\n\
         printf '%s' \"$PWD\" > setup-pwd.txt\n\
         export CODEX_TEST_EXPORT=exported\n\"\"\"\n",
    )?;
    git(&source, &["add", "."])?;
    git(&source, &["commit", "-qm", "base"])?;
    git(&source, &["push", "-q", "-u", "origin", "main"])?;
    let base = git(&source, &["rev-parse", "HEAD"])?;
    let other = dir.join("other");
    git(
        dir,
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    )?;
    write(&other.join("sub/remote.txt"), "moved\n")?;
    git(&other, &["add", "."])?;
    git(&other, &["commit", "-qm", "moved"])?;
    git(&other, &["push", "-q", "origin", "main"])?;
    let moved = git(&other, &["rev-parse", "HEAD"])?;
    git(&source, &["fetch", "-q", "origin"])?;
    git(&source, &["remote", "set-head", "origin", "main"])?;
    git(&source, &["checkout", "-q", "-b", "feature"])?;
    write(&source.join("sub/tracked.txt"), "user edit\n")?;
    write(&source.join("sub/draft.txt"), "untracked user work\n")?;
    write(&source.join(".env"), "TOKEN=local\n")?;
    write(&source.join("AGENTS.override.md"), "override\n")?;
    write(&source.join("secret.txt"), "not listed\n")?;
    Ok((source, base, moved))
}

#[test]
fn creates_the_automation_worktree_at_the_remote_default_without_touching_the_source() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("codex");
    let (source, base, moved) = fixture(dir.path())?;
    let before = snapshot(&source)?;
    let host = LocalHost::new(&home);
    let cwd = source.join("sub");
    let environment = source.join(".codex/environments/environment.toml");
    let created = call(
        &host,
        "create-worktree",
        json!({"cwd": cwd, "localEnvironmentConfigPath": environment}),
    )?;
    let root = PathBuf::from(created["worktreeGitRoot"].as_str().unwrap());
    let workspace = PathBuf::from(created["worktreeWorkspaceRoot"].as_str().unwrap());
    assert_eq!(created["setupError"], Value::Null);
    let reserved = root.parent().unwrap();
    assert_eq!(reserved.parent().unwrap(), home.join("worktrees"));
    assert!(reserved.join(".codex-worktree-name").is_file());
    assert_eq!(root.file_name().unwrap(), "project");
    assert_eq!(workspace, root.join("sub"));

    // Only the upstream moved, so the run starts at origin/main, not the stale local main.
    assert_eq!(git(&root, &["rev-parse", "HEAD"])?, moved);
    assert!(root.join("sub/remote.txt").is_file());
    assert_eq!(git(&root, &["symbolic-ref", "-q", "HEAD"]).ok(), None);
    // The source checkout, its dirty work and every branch stay as they were.
    assert_eq!(snapshot(&source)?, before);
    assert_eq!(git(&source, &["rev-parse", "main"])?, base);

    assert_eq!(fs::read_to_string(root.join(".env"))?, "TOKEN=local\n");
    assert_eq!(
        fs::read_to_string(root.join("AGENTS.override.md"))?,
        "override\n"
    );
    assert!(!root.join("secret.txt").exists());
    assert!(!root.join("sub/draft.txt").exists());

    let copied = root.join(".codex/environments/environment.toml");
    assert_eq!(
        git(
            &root,
            &[
                "config",
                "--worktree",
                "--get",
                "codex.localEnvironmentConfigPath"
            ]
        )?,
        copied.to_str().unwrap()
    );
    assert_eq!(
        fs::read_to_string(root.join("setup-source.txt"))?,
        cwd.to_str().unwrap()
    );
    assert_eq!(
        fs::read_to_string(root.join("setup-worktree.txt"))?,
        workspace.to_str().unwrap()
    );
    assert_eq!(
        fs::canonicalize(fs::read_to_string(root.join("setup-pwd.txt"))?)?,
        fs::canonicalize(&root)?
    );
    let exported: Value = serde_json::from_str(&fs::read_to_string(root.join(git(
        &root,
        &["rev-parse", "--git-path", "codex-shell-environment.json"],
    )?))?)?;
    assert_eq!(exported["version"], 1);
    assert_eq!(exported["set"]["CODEX_TEST_EXPORT"], "exported");
    assert!(exported["set"].get("CODEX_WORKTREE_PATH").is_none());
    assert!(exported["set"].get("PWD").is_none());

    for endpoint in ["set-worktree-owner-thread", "worktree-set-owner-thread"] {
        let owned = call(
            &host,
            endpoint,
            json!({"worktree": root, "conversationId": "thread-1"}),
        )?;
        assert_eq!(owned, json!({"success": true}));
        let owner = root.join(git(
            &root,
            &["rev-parse", "--git-path", "codex-thread.json"],
        )?);
        let owner: Value = serde_json::from_slice(&fs::read(owner)?)?;
        assert_eq!(owner, json!({"version": 1, "ownerThreadId": "thread-1"}));
        let refused = call(
            &host,
            endpoint,
            json!({"worktree": source, "conversationId": "thread-1"}),
        );
        assert!(refused.is_err());
        assert!(!source.join(".git/codex-thread.json").exists());
    }

    // Without a local `main`, the remote-tracking start gets a local branch that tracks it.
    git(&source, &["branch", "-q", "-D", "main"])?;
    let second = call(&host, "create-worktree", json!({"cwd": source}))?;
    let second = PathBuf::from(second["worktreeGitRoot"].as_str().unwrap());
    assert_ne!(second, root);
    assert_eq!(git(&second, &["rev-parse", "HEAD"])?, moved);
    assert_eq!(
        git(&source, &["rev-parse", "--abbrev-ref", "main@{u}"])?,
        "origin/main"
    );
    assert_eq!(
        git(&source, &["symbolic-ref", "HEAD"])?,
        "refs/heads/feature"
    );
    Ok(())
}

#[test]
fn head_start_records_the_synced_branch_and_failures_clean_up_only_safe_worktrees() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("codex");
    let worktrees = home.join("worktrees");
    let source = dir.path().join("local");
    git(
        dir.path(),
        &["init", "-q", "-b", "main", source.to_str().unwrap()],
    )?;
    write(&source.join(".gitignore"), "/fail/\n")?;
    write(&source.join("sub/tracked.txt"), "base\n")?;
    write(
        &source.join("fail/.codex/environments/clean.toml"),
        "name = \"clean\"\n[setup]\nscript = \"exit 3\"\n",
    )?;
    write(
        &source.join("fail/.codex/environments/dirty.toml"),
        "name = \"dirty\"\n[setup]\nscript = \"touch ../made-by-setup\\nexit 4\"\n",
    )?;
    git(&source, &["add", "."])?;
    git(&source, &["commit", "-qm", "base"])?;
    git(&source, &["checkout", "-q", "-b", "topic"])?;
    write(&source.join("sub/draft.txt"), "untracked user work\n")?;
    let host = LocalHost::new(&home);

    // No remote: the run starts at HEAD and remembers the source branch for a later sync.
    let created = call(&host, "create-worktree", json!({"cwd": source}))?;
    let root = PathBuf::from(created["worktreeGitRoot"].as_str().unwrap());
    assert_eq!(
        git(&root, &["rev-parse", "HEAD"])?,
        git(&source, &["rev-parse", "HEAD"])?
    );
    let synced: Value = serde_json::from_str(&fs::read_to_string(root.join(git(
        &root,
        &["rev-parse", "--git-path", "codex-synced-branch.json"],
    )?))?)?;
    assert_eq!(synced["branch"], "refs/heads/topic");
    assert_eq!(
        synced["lastSyncedTreeRef"],
        git(&source, &["rev-parse", "topic^{tree}"])?
    );
    assert_eq!(
        git(
            &root,
            &[
                "config",
                "--worktree",
                "--get",
                "codex.localEnvironmentConfigPath"
            ]
        )?,
        "__none__"
    );

    let count = reservations(&worktrees)?.len();
    let listed = git(&source, &["worktree", "list", "--porcelain"])?;
    // A start that does not exist leaves no reservation behind.
    let missing = call(
        &host,
        "create-worktree",
        json!({"cwd": source, "startingState": {"type": "branch", "branchName": "no-such-branch"}}),
    );
    assert!(format!("{:#}", missing.unwrap_err()).contains("git worktree add failed"));
    assert_eq!(reservations(&worktrees)?.len(), count);

    // A failed setup removes the clean worktree it made.
    let failed = call(
        &host,
        "create-worktree",
        json!({"cwd": source,
            "localEnvironmentConfigPath": source.join("fail/.codex/environments/clean.toml")}),
    );
    let failed = format!("{:#}", failed.unwrap_err());
    assert!(
        failed.contains("Setup script exited with code 3"),
        "{failed}"
    );
    assert!(!failed.contains("was kept"), "{failed}");
    assert_eq!(reservations(&worktrees)?.len(), count);
    assert_eq!(git(&source, &["worktree", "list", "--porcelain"])?, listed);

    // A setup that left untracked files is never force-removed.
    let failed = call(
        &host,
        "create-worktree",
        json!({"cwd": source,
            "localEnvironmentConfigPath": source.join("fail/.codex/environments/dirty.toml")}),
    );
    let failed = format!("{:#}", failed.unwrap_err());
    assert!(
        failed.contains("Setup script exited with code 4"),
        "{failed}"
    );
    assert!(failed.contains("was kept"), "{failed}");
    let kept: Vec<PathBuf> = reservations(&worktrees)?
        .into_iter()
        .filter(|dir| dir.join("local/made-by-setup").is_file())
        .collect();
    assert_eq!(kept.len(), 1);

    assert_eq!(
        fs::read_to_string(source.join("sub/draft.txt"))?,
        "untracked user work\n"
    );
    assert_eq!(git(&source, &["symbolic-ref", "HEAD"])?, "refs/heads/topic");
    let outside = dir.path().join("outside");
    fs::create_dir(&outside)?;
    let plain = call(&host, "create-worktree", json!({"cwd": outside}));
    assert!(format!("{:#}", plain.unwrap_err()).contains("Not a git repository"));
    let remote_ref = call(
        &host,
        "create-worktree",
        json!({"cwd": source, "startingState":
            {"type": "branch", "branchName": "main", "remoteRef": "refs/remotes/origin/main"}}),
    );
    assert!(remote_ref.is_err());
    Ok(())
}

#[test]
fn invalid_action_platform_rejects_setup_like_the_original_schema() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (source, _, _) = fixture(dir.path())?;
    let environment = source.join(".codex/environments/invalid.toml");
    write(
        &environment,
        "version=1\nname='Invalid action'\n[setup]\nscript='exit 0'\n[[actions]]\nname='Run'\ncommand='echo run'\nplatform='macos'\n",
    )?;
    let before = snapshot(&source)?;
    let home = dir.path().join("codex");
    let host = LocalHost::new(&home);
    let result = call(
        &host,
        "create-worktree",
        json!({"cwd": source, "localEnvironmentConfigPath": environment}),
    );
    assert!(result.is_err());
    assert_eq!(snapshot(&source)?, before);
    assert!(reservations(&home.join("worktrees"))?.is_empty());
    Ok(())
}
