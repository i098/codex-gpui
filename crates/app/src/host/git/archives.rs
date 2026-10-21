use super::{MARKER, checked, command, commit_id, git, linked_root, set_owner_thread};
use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    fs,
    path::{Component, Path},
};

#[derive(Debug)]
struct NestedRepository;

impl std::fmt::Display for NestedRepository {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Worktree snapshots cannot preserve initialized submodules or embedded Git repositories. Keep this checkout.")
    }
}

impl std::error::Error for NestedRepository {}

fn identifier(path: &Path) -> Result<String> {
    let path = path.to_str().context("worktree path must be UTF-8")?;
    let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, path.as_bytes());
    let mut hex = String::with_capacity(40);
    for byte in digest.as_ref() {
        write!(&mut hex, "{byte:02x}")?;
    }
    Ok(hex)
}

fn reference(path: &Path) -> Result<String> {
    Ok(format!("refs/codex/snapshots/{}", identifier(path)?))
}

fn managed(path: &Path, root: &Path) -> Result<()> {
    ensure!(
        path.is_absolute() && root.is_absolute(),
        "worktree paths must be absolute"
    );
    ensure!(
        !path
            .components()
            .any(|part| matches!(part, Component::ParentDir)),
        "invalid worktree path"
    );
    let reservation = path.parent().context("worktree has no reservation")?;
    ensure!(
        reservation.parent() == Some(root),
        "worktree is outside the managed worktree root"
    );
    for ancestor in path.ancestors().take_while(|ancestor| *ancestor != root) {
        if let Ok(metadata) = fs::symlink_metadata(ancestor) {
            ensure!(
                !metadata.file_type().is_symlink(),
                "managed worktree path contains a symlink"
            );
        }
    }
    Ok(())
}

fn snapshot(path: &Path, reason: &str) -> Result<String> {
    let files = checked(run_files(path)?)?;
    for entry in files
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        if entry.starts_with(b"160000 ") || entry.ends_with(b"/") {
            let name = entry
                .iter()
                .position(|byte| *byte == b'\t')
                .map_or(entry, |at| &entry[at + 1..]);
            if path.join(std::str::from_utf8(name)?).join(".git").exists() {
                return Err(NestedRepository.into());
            }
        }
    }
    let scratch = tempfile::tempdir()?;
    let index = scratch.path().join("index");
    let head = commit_id(path, "HEAD")?;
    let read = if head.is_some() { "HEAD" } else { "--empty" };
    checked(
        command(path, &["read-tree", read])
            .env("GIT_INDEX_FILE", &index)
            .output()?,
    )?;
    checked(
        command(path, &["add", "-A", "--", "."])
            .env("GIT_INDEX_FILE", &index)
            .output()?,
    )?;
    let tree = String::from_utf8(checked(
        command(path, &["write-tree"])
            .env("GIT_INDEX_FILE", &index)
            .output()?,
    )?)?;
    let tree = tree.trim();
    let unchanged = if let Some(head) = &head {
        let prior = git(path, &["rev-parse", &format!("{head}^{{tree}}")])?;
        prior.success && prior.stdout.trim() == tree
    } else {
        false
    };
    let commit = if unchanged {
        head.unwrap()
    } else {
        let mut args = vec!["commit-tree".to_owned(), tree.to_owned()];
        if let Some(head) = head {
            args.extend(["-p".into(), head]);
        }
        args.extend([
            "-m".into(),
            format!("Codex worktree snapshot: {reason}"),
            "-m".into(),
            "Co-authored-by: Codex".into(),
        ]);
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        String::from_utf8(checked(super::run(path, &args)?)?)?
            .trim()
            .to_owned()
    };
    let id = identifier(path)?;
    let snapshot_ref = format!("refs/codex/snapshots/{id}");
    checked(super::run(path, &["update-ref", &snapshot_ref, &commit])?)?;
    Ok(id)
}

fn run_files(path: &Path) -> Result<std::process::Output> {
    super::run(
        path,
        &[
            "ls-files",
            "--stage",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )
}

pub(in crate::host) fn fetch(
    endpoint: &str,
    params: &Value,
    worktrees_root: &Path,
) -> Result<Value> {
    match endpoint {
        "delete-worktree" | "worktree-delete" => {
            let path = Path::new(params["worktree"].as_str().context("missing worktree")?);
            managed(path, worktrees_root)?;
            ensure!(linked_root(path)?, "worktree is not a linked checkout");
            let reason = params["reason"].as_str().unwrap_or("archive-cleanup");
            let id = match snapshot(path, reason) {
                Ok(id) => id,
                Err(error) if reason == "settings-delete-targeted" => {
                    eprintln!(
                        "codex-gpui: worktree snapshot failed before explicit deletion: {error}"
                    );
                    identifier(path)?
                }
                Err(error) if error.is::<NestedRepository>() => {
                    return Ok(
                        json!({"success": false, "reason": "unsupported-nested-repository"}),
                    );
                }
                Err(error) => return Err(error),
            };
            let reservation = path.parent().unwrap();
            fs::write(reservation.join(MARKER), "")?;
            if params["skipCleanupScript"] != true {
                super::cleanup_worktree_environment(path)?;
            }
            let mut remove = command(path, &["worktree", "remove"]);
            if params["force"] == true {
                remove.arg("--force");
            }
            let removed = remove.arg(path).output()?;
            if !removed.status.success() && reason == "archive-cleanup" {
                return Ok(json!({"success": false, "reason": "deletion-failed"}));
            }
            checked(removed)?;
            Ok(json!({"success": true, "worktreeId": id}))
        }
        "worktree-snapshot-ref" => {
            let path = Path::new(
                params["worktreePath"]
                    .as_str()
                    .context("missing worktreePath")?,
            );
            let snapshot_ref = reference(path)?;
            let roots = params["candidateRoots"]
                .as_array()
                .context("missing candidateRoots")?;
            for candidate in roots.iter().filter_map(Value::as_str).map(Path::new) {
                if !candidate.is_dir() {
                    continue;
                }
                if commit_id(candidate, &snapshot_ref)?.is_some() {
                    return Ok(
                        json!({"exists": true, "repoRoot": candidate, "snapshotRef": snapshot_ref}),
                    );
                }
            }
            Ok(json!({"exists": false, "repoRoot": null, "snapshotRef": snapshot_ref}))
        }
        "restore-worktree" => {
            let source = Path::new(params["repoRoot"].as_str().context("missing repoRoot")?);
            let path = Path::new(
                params["worktreePath"]
                    .as_str()
                    .context("missing worktreePath")?,
            );
            let cwd = Path::new(params["cwd"].as_str().context("missing cwd")?);
            managed(path, worktrees_root)?;
            ensure!(
                cwd.starts_with(path)
                    && !cwd
                        .components()
                        .any(|part| matches!(part, Component::ParentDir)),
                "Invalid restored worktree cwd"
            );
            let snapshot_ref = reference(path)?;
            let saved = commit_id(source, &snapshot_ref)?
                .context("The archived worktree snapshot is missing.")?;
            let existing =
                path.exists() && commit_id(path, "HEAD")?.as_deref() == Some(saved.as_str());
            if !existing {
                fs::create_dir_all(path.parent().unwrap())?;
                checked(
                    command(source, &["worktree", "add", "--detach"])
                        .arg(path)
                        .arg(&snapshot_ref)
                        .output()?,
                )?;
            }
            if !cwd.is_dir() {
                let relative = cwd.strip_prefix(path)?;
                let materialized = command(path, &["sparse-checkout", "add", "--"])
                    .arg(relative)
                    .output()?;
                if !materialized.status.success() || !cwd.is_dir() {
                    if !existing {
                        let _ = command(source, &["worktree", "remove", "--force"])
                            .arg(path)
                            .output();
                    }
                    anyhow::bail!(
                        "Failed to materialize restored worktree cwd: {}",
                        cwd.display()
                    );
                }
            }
            if let Some(thread) = params["conversationId"].as_str()
                && let Err(error) = set_owner_thread(path, thread)
            {
                eprintln!("codex-gpui: failed to set restored worktree owner: {error}");
            }
            Ok(json!({"success": true}))
        }
        _ => anyhow::bail!("unsupported worktree archive operation: {endpoint}"),
    }
}
