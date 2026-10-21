use super::{checkout, commit_id, current_branch, git};
use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Checkout {
    cwd: PathBuf,
    original: String,
    stash: Option<String>,
}

fn stash_action(cwd: &Path, sha: &str, action: &str) -> Result<()> {
    let listed = git(cwd, &["stash", "list", "--format=%H %gd"])?;
    ensure!(listed.success, "{}", listed.failure());
    let reference = listed
        .stdout
        .lines()
        .find_map(|line| {
            let (id, reference) = line.split_once(' ')?;
            (id == sha).then_some(reference)
        })
        .context("The handoff stash is missing")?;
    let result = git(cwd, &["stash", action, reference])?;
    ensure!(result.success, "{}", result.failure());
    Ok(())
}

fn switch(cwd: &Path, branch: &str, stash: bool) -> Result<Checkout> {
    let original = match current_branch(cwd)? {
        Some(branch) => branch,
        None => commit_id(cwd, "HEAD")?.context("Unable to resolve checkout HEAD")?,
    };
    let saved_stash = if stash {
        let before = commit_id(cwd, "refs/stash")?;
        let saved = git(
            cwd,
            &[
                "stash",
                "push",
                "--include-untracked",
                "-m",
                "Codex handoff",
            ],
        )?;
        ensure!(saved.success, "{}", saved.failure());
        let saved_work = !saved.stdout.contains("No local changes to save")
            && !saved.stderr.contains("No local changes to save");
        let after = match commit_id(cwd, "refs/stash") {
            Ok(after) => after,
            Err(error) => {
                if saved_work {
                    let restored = git(cwd, &["stash", "pop", "stash@{0}"])?;
                    ensure!(
                        restored.success,
                        "{error}; failed to restore stash: {}",
                        restored.failure()
                    );
                }
                return Err(error);
            }
        };
        if before != after { after } else { None }
    } else {
        None
    };
    let checkpoint = Checkout {
        cwd: cwd.to_path_buf(),
        original,
        stash: saved_stash,
    };
    let result = checkout(cwd, &json!({"branch": branch}))
        .unwrap_or_else(|error| json!({"status": "error", "error": error.to_string()}));
    if result["status"] != "success" {
        let mut error = result["error"]
            .as_str()
            .unwrap_or("Failed to check out branch")
            .to_owned();
        if let Some(stash) = &checkpoint.stash
            && let Err(restore) = stash_action(cwd, stash, "pop")
        {
            error.push_str(&format!("\nFailed to restore stash: {restore}"));
        }
        anyhow::bail!(error);
    }
    Ok(checkpoint)
}

fn restore(checkpoint: &Checkout) -> Result<()> {
    let result = checkout(&checkpoint.cwd, &json!({"branch": checkpoint.original}))?;
    ensure!(
        result["status"] == "success",
        "{}",
        result["error"]
            .as_str()
            .unwrap_or("Failed to restore branch")
    );
    if let Some(stash) = &checkpoint.stash {
        stash_action(&checkpoint.cwd, stash, "pop")?;
    }
    Ok(())
}

fn same_repository(left: &Path, right: &Path) -> Result<()> {
    let common = |cwd| -> Result<PathBuf> {
        let value = git(
            cwd,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        ensure!(value.success, "Unable to resolve handoff repository");
        Ok(std::fs::canonicalize(value.stdout.trim())?)
    };
    ensure!(
        common(left)? == common(right)?,
        "Handoff workspaces belong to different repositories"
    );
    Ok(())
}

fn path<'a>(params: &'a Value, key: &str) -> Result<&'a Path> {
    let path = Path::new(
        params[key]
            .as_str()
            .with_context(|| format!("missing {key}"))?,
    );
    ensure!(path.is_absolute(), "handoff paths must be absolute");
    Ok(path)
}

fn branch<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    let branch = params[key]
        .as_str()
        .with_context(|| format!("missing {key}"))?
        .trim();
    ensure!(!branch.is_empty(), "Missing source branch");
    Ok(branch)
}

fn failure(code: &str, message: impl ToString, rollback: Vec<&str>, warnings: Vec<&str>) -> Value {
    json!({"status": "error", "error": code, "message": message.to_string(), "rollbackErrors": rollback, "warnings": warnings})
}

pub(in crate::host) fn to_local(params: &Value) -> Result<Value> {
    let source = path(params, "sourceWorktreeCwd")?;
    let source_root = path(params, "sourceWorktreeRoot")?;
    let local = path(params, "localGitRoot")?;
    let branch = branch(params, "sourceBranch")?;
    same_repository(source, local)?;
    let listed = git(local, &["worktree", "list", "--porcelain", "-z"])?;
    ensure!(listed.success, "{}", listed.failure());
    let mut listed_root = None;
    for part in listed.stdout.split('\0') {
        if let Some(root) = part.strip_prefix("worktree ") {
            listed_root = Some(PathBuf::from(root));
        }
        if part.strip_prefix("branch refs/heads/") == Some(branch)
            && let Some(root) = &listed_root
            && std::fs::canonicalize(root)? != std::fs::canonicalize(local)?
            && std::fs::canonicalize(root)? != std::fs::canonicalize(source_root)?
        {
            return Ok(failure(
                "branch-checked-out-elsewhere",
                "Branch is already checked out in another worktree",
                vec![],
                vec![],
            ));
        }
    }
    let head = commit_id(source, "HEAD")?.context("Unable to resolve worktree HEAD commit")?;
    let source_checkpoint = match switch(source, &head, true) {
        Ok(checkpoint) => checkpoint,
        Err(error) => return Ok(failure("source-detach-failed", error, vec![], vec![])),
    };
    let outcome = (|| -> Result<Value> {
        let mut rollback = Vec::new();
        let status = git(
            local,
            &["status", "--porcelain=v1", "--untracked-files=all"],
        )?;
        if !status.success || !status.stdout.trim().is_empty() {
            if restore(&source_checkpoint).is_err() {
                rollback.push("restore-source-worktree-failed");
            }
            return Ok(failure(
                "local-destination-has-tracked-changes",
                "You have uncommitted local changes. Commit or stash them first.",
                rollback,
                vec![],
            ));
        }
        if current_branch(local)?.as_deref() != Some(branch) {
            if let Some(existing) = commit_id(local, &format!("refs/heads/{branch}"))? {
                if existing != head {
                    if restore(&source_checkpoint).is_err() {
                        rollback.push("restore-source-worktree-failed");
                    }
                    return Ok(failure(
                        "local-branch-head-mismatch",
                        "The local branch points to a different commit",
                        rollback,
                        vec![],
                    ));
                }
            } else {
                let created = git(local, &["branch", "--", branch, &head])?;
                if !created.success {
                    if restore(&source_checkpoint).is_err() {
                        rollback.push("restore-source-worktree-failed");
                    }
                    return Ok(failure(
                        "local-branch-head-mismatch",
                        created.failure(),
                        rollback,
                        vec![],
                    ));
                }
            }
        }
        let local_checkpoint = match switch(local, branch, false) {
            Ok(checkpoint) => checkpoint,
            Err(error) => {
                if restore(&source_checkpoint).is_err() {
                    rollback.push("restore-source-worktree-failed");
                }
                return Ok(failure("checkout-local-failed", error, rollback, vec![]));
            }
        };
        let mut warnings = Vec::new();
        if let Some(stash) = &source_checkpoint.stash {
            if let Err(error) = stash_action(local, stash, "apply") {
                if restore(&local_checkpoint).is_err() {
                    rollback.push("restore-local-branch-failed");
                }
                if restore(&source_checkpoint).is_err() {
                    rollback.push("restore-source-worktree-failed");
                }
                return Ok(failure(
                    "apply-source-stash-failed",
                    error,
                    rollback,
                    warnings,
                ));
            }
            if stash_action(local, stash, "drop").is_err() {
                warnings.push("drop-source-stash-failed");
            }
        }
        Ok(json!({"status": "success", "warnings": warnings}))
    })();
    match outcome {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            let mut rollback = Vec::new();
            if restore(&source_checkpoint).is_err() {
                rollback.push("restore-source-worktree-failed");
            }
            Ok(failure("unexpected-error", error, rollback, vec![]))
        }
    }
}

pub(in crate::host) fn to_worktree(params: &Value, worktrees_root: &Path) -> Result<Value> {
    let local = path(params, "localCwd")?;
    let target = path(params, "worktreeWorkspaceRoot")?;
    let source_branch = branch(params, "sourceBranch")?;
    let target_branch = params["worktreeCheckoutBranch"]
        .as_str()
        .unwrap_or(source_branch)
        .trim();
    ensure!(
        !target_branch.is_empty(),
        "Missing worktree checkout branch"
    );
    if params["defaultBranch"].as_str() == Some(target_branch) {
        return Ok(failure(
            "default-branch-blocked",
            "Move to worktree is not available on the default branch",
            vec![],
            vec![],
        ));
    }
    same_repository(local, target)?;
    let Some(local_branch) = current_branch(local)? else {
        return Ok(failure(
            "local-not-on-branch",
            "Local repository must be on a branch to move to a worktree",
            vec![],
            vec![],
        ));
    };
    let local_checkpoint = if local_branch == source_branch {
        let fallback = params["defaultBranch"].as_str() == Some(source_branch)
            || target_branch != source_branch;
        let destination = params["localCheckoutBranch"]
            .as_str()
            .map(str::trim)
            .filter(|branch| !branch.is_empty())
            .or(if fallback { Some(source_branch) } else { None });
        let Some(destination) = destination else {
            return Ok(failure(
                "invalid-params",
                "Missing local checkout branch",
                vec![],
                vec![],
            ));
        };
        match switch(local, destination, true) {
            Ok(checkpoint) => Some(checkpoint),
            Err(error) => return Ok(failure("checkout-local-failed", error, vec![], vec![])),
        }
    } else {
        None
    };
    let mut rollback = Vec::new();
    let target_checkpoint =
        match switch(target, target_branch, params["stashTargetWorktree"] == true) {
            Ok(checkpoint) => checkpoint,
            Err(error) => {
                if let Some(local) = &local_checkpoint
                    && restore(local).is_err()
                {
                    rollback.push("restore-local-branch-failed");
                }
                cleanup_created(params, worktrees_root, &mut rollback);
                return Ok(failure("checkout-worktree-failed", error, rollback, vec![]));
            }
        };
    let mut warnings = Vec::new();
    if target_checkpoint.stash.is_some() {
        warnings.push("stashed-target-worktree-changes");
    }
    if let Some(local) = &local_checkpoint
        && let Some(stash) = &local.stash
        && let Err(error) = stash_action(target, stash, "apply")
    {
        if restore(&target_checkpoint).is_err() {
            rollback.push("restore-target-stash-failed");
        }
        if restore(local).is_err() {
            rollback.push("restore-local-branch-failed");
        }
        cleanup_created(params, worktrees_root, &mut rollback);
        return Ok(failure(
            "apply-source-stash-failed",
            error,
            rollback,
            warnings,
        ));
    }
    if let Some(stash) = &target_checkpoint.stash
        && stash_action(target, stash, "drop").is_err()
    {
        warnings.push("drop-target-stash-failed");
    }
    if let Some(local) = &local_checkpoint
        && let Some(stash) = &local.stash
        && stash_action(local.cwd.as_path(), stash, "drop").is_err()
    {
        warnings.push("drop-source-stash-failed");
    }
    Ok(json!({"status": "success", "warnings": warnings}))
}

fn cleanup_created(params: &Value, worktrees_root: &Path, errors: &mut Vec<&'static str>) {
    if params["createdWorktree"] != true {
        return;
    }
    let Some(root) = params["worktreeGitRoot"].as_str() else {
        errors.push("cleanup-created-worktree-failed");
        return;
    };
    let workspace = params["worktreeWorkspaceRoot"].as_str().unwrap_or(root);
    let worktree = super::Worktree {
        git_root: PathBuf::from(root),
        workspace_root: PathBuf::from(workspace),
    };
    if super::remove_unstarted_worktree(worktrees_root, &worktree).is_err() {
        errors.push("cleanup-created-worktree-failed");
    }
}
