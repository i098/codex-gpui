use super::*;
use sha1::{Digest, Sha1};

pub(in crate::host) fn managed_worktrees(roots: &[PathBuf]) -> Result<Value> {
    let mut worktrees = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        let entries = match fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            append_reservation(entry, &mut seen, &mut worktrees)?;
        }
    }
    worktrees.sort_by(|a, b| a["dir"].as_str().cmp(&b["dir"].as_str()));
    Ok(json!({"worktrees": worktrees}))
}

fn append_reservation(
    entry: fs::DirEntry,
    seen: &mut HashSet<PathBuf>,
    worktrees: &mut Vec<Value>,
) -> Result<()> {
    let reservation = entry.path();
    let name = entry.file_name();
    let name = name.to_string_lossy();
    let legacy = name.len() == 4 && name.bytes().all(|byte| byte.is_ascii_hexdigit());
    let modern = valid_name(&name) && reservation.join(MARKER).is_file();
    for entry in fs::read_dir(&reservation)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let path = entry.path();
        if !legacy && !modern && !legacy_directory(&entry.file_name().to_string_lossy()) {
            continue;
        }
        if !seen.insert(path.canonicalize()?) {
            continue;
        }
        let git_dir = answer(&path, &["rev-parse", "--absolute-git-dir"])?;
        if modern && (git_dir.is_none() || !path.join(".git").is_file()) {
            continue;
        }
        let common = answer(
            &path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        let repository = common.as_deref().and_then(|path| Path::new(path).parent());
        worktrees.push(json!({"dir": path, "gitDir": git_dir, "repositoryRoot": repository}));
    }
    Ok(())
}

fn valid_name(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 64
        || name.len() >= 4 && name.bytes().all(|c| c.is_ascii_hexdigit())
    {
        return false;
    }
    let reserved = matches!(name, "con" | "prn" | "aux" | "nul")
        || name
            .strip_prefix("com")
            .or_else(|| name.strip_prefix("lpt"))
            .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
    !reserved
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn legacy_directory(name: &str) -> bool {
    let mut parts = name.split('-');
    let (Some(date), Some(time), Some(id), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    date.len() == 6
        && time.len() == 4
        && id.len() == 8
        && date.bytes().chain(time.bytes()).all(|c| c.is_ascii_digit())
        && id.bytes().all(|c| c.is_ascii_hexdigit())
}

fn check_nested_repositories(root: &Path) -> Result<()> {
    let nested = checked(run(
        root,
        &[
            "ls-files",
            "--stage",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?)?;
    for entry in nested
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let entry = std::str::from_utf8(entry)?;
        let path = if entry.starts_with("160000 ") {
            entry.split_once('\t').map(|(_, path)| path)
        } else if entry.ends_with('/') {
            Some(entry)
        } else {
            None
        };
        ensure!(
            !path.is_some_and(|path| root.join(path).join(".git").exists()),
            "Worktree snapshots cannot preserve initialized submodules or embedded Git repositories. Keep this checkout."
        );
    }
    Ok(())
}

fn snapshot(root: &Path) -> Result<()> {
    check_nested_repositories(root)?;
    let scratch = tempfile::tempdir()?;
    let index = scratch.path().join("index");
    let git = |args: &[&str]| -> Result<String> {
        let output = command(root, args)
            .env("GIT_INDEX_FILE", &index)
            .env("GIT_AUTHOR_NAME", "Codex")
            .env("GIT_AUTHOR_EMAIL", "codex@openai.com")
            .env("GIT_COMMITTER_NAME", "Codex")
            .env("GIT_COMMITTER_EMAIL", "codex@openai.com")
            .output()?;
        Ok(String::from_utf8(checked(output)?)?.trim().to_owned())
    };
    let head = answer(root, &["rev-parse", "--verify", "HEAD"])?;
    if let Some(head) = &head {
        git(&["read-tree", head])?;
    } else {
        git(&["read-tree", "--empty"])?;
    }
    git(&["add", "--all"])?;
    let tree = git(&["write-tree"])?;
    let commit = if let Some(head) = &head {
        git(&[
            "commit-tree",
            &tree,
            "-p",
            head,
            "-m",
            "Codex worktree snapshot: settings-delete-targeted",
        ])?
    } else {
        git(&[
            "commit-tree",
            &tree,
            "-m",
            "Codex worktree snapshot: settings-delete-targeted",
        ])?
    };
    let id = format!("{:x}", Sha1::digest(root.to_string_lossy().as_bytes()));
    git(&["update-ref", &format!("refs/codex/snapshots/{id}"), &commit])?;
    Ok(())
}

pub(in crate::host) fn remove_managed_worktree(root: &Path, roots: &[PathBuf]) -> Result<()> {
    ensure!(root.is_absolute(), "Worktree path must be absolute");
    let canonical = root.canonicalize()?;
    let catalog = managed_worktrees(roots)?;
    let root = catalog["worktrees"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["dir"].as_str())
        .map(PathBuf::from)
        .find(|path| path.canonicalize().ok().as_ref() == Some(&canonical))
        .context("Not a managed worktree")?;
    snapshot(&root)?;
    let selected = answer(
        &root,
        &[
            "config",
            "--worktree",
            "--get",
            "codex.localEnvironmentConfigPath",
        ],
    )?;
    if let Some(config) = selected
        .filter(|path| path != NO_ENVIRONMENT)
        .map(PathBuf::from)
        && let Ok(environment) = environments::config(&config)
        && let Some(script) = environment
            .cleanup
            .as_ref()
            .map(Scripts::current)
            .filter(|script| !script.is_empty())
    {
        run_script(
            &root.join(script_dir(&config)),
            script,
            "Cleanup script",
            &[],
        )?;
    }
    checked(run(
        &root,
        &[
            "worktree",
            "remove",
            "--force",
            root.to_str().context("Worktree path is not UTF-8")?,
        ],
    )?)?;
    let reservation = root.parent().context("Worktree has no parent")?;
    let marker = reservation.join(MARKER);
    if marker.is_file()
        && fs::read_dir(reservation)?
            .all(|entry| entry.is_ok_and(|entry| entry.file_name() == MARKER))
    {
        fs::remove_file(marker)?;
    }
    match fs::remove_dir(reservation) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DirectoryNotEmpty | ErrorKind::NotFound
            ) => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

pub(in crate::host) fn thread_worktree_info(params: &Value) -> Result<Value> {
    let cwd = Path::new(params["cwd"].as_str().context("missing cwd")?);
    ensure!(cwd.is_absolute(), "Workspace must be absolute");
    if !cwd.is_dir() {
        return Ok(json!({"missing": true}));
    }
    let rows = String::from_utf8(checked(run(cwd, &["worktree", "list", "--porcelain"])?)?)?;
    let local = rows
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .context("Local checkout is unavailable")?;
    let root = answer(cwd, &["rev-parse", "--show-toplevel"])?.context("Not a repository")?;
    Ok(json!({"missing": false, "localRoot": local, "worktree": root != local}))
}

fn stash_changes(root: &Path) -> Result<Option<String>> {
    if checked(run(root, &["status", "--porcelain"])?)?.is_empty() {
        return Ok(None);
    }
    checked(plain(
        root,
        &[
            "stash",
            "push",
            "--include-untracked",
            "-m",
            "Codex thread handoff",
        ],
    )?)?;
    Ok(answer(root, &["rev-parse", "refs/stash"])?)
}

fn apply_stash(root: &Path, stash: Option<&str>) -> Result<()> {
    if let Some(stash) = stash {
        checked(run(root, &["stash", "apply", "--index", stash])?)?;
    }
    Ok(())
}

pub(in crate::host) fn move_thread(params: &Value, worktrees_root: &Path) -> Result<Value> {
    let cwd = Path::new(params["cwd"].as_str().context("missing cwd")?);
    ensure!(cwd.is_absolute(), "Workspace must be absolute");
    let info = thread_worktree_info(params)?;
    ensure!(info["missing"] != true, "This checkout no longer exists");
    let to_local = params["toLocal"] == true;
    ensure!(
        info["worktree"] == to_local,
        "Choose the other checkout type for this thread"
    );
    let source =
        PathBuf::from(answer(cwd, &["rev-parse", "--show-toplevel"])?.context("Not a repository")?);
    let prefix = answer(cwd, &["rev-parse", "--show-prefix"])?.unwrap_or_default();
    let head = answer(&source, &["rev-parse", "HEAD"])?.context("Repository has no commits")?;
    let branch = current_branch(&source)?;
    check_nested_repositories(&source)?;
    let target = if to_local {
        let target = PathBuf::from(params["workspace"].as_str().context("missing workspace")?);
        ensure!(target.is_absolute(), "Workspace must be absolute");
        let source_common = answer(
            &source,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        let target_common = answer(
            &target,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        ensure!(
            source_common == target_common,
            "Workspace belongs to another repository"
        );
        ensure!(
            checked(run(&target, &["status", "--porcelain"])?)?.is_empty(),
            "The local checkout has changes. Commit or stash them first."
        );
        target
    } else {
        create_worktree(worktrees_root, cwd, Some(&head), None)?
            .context("Not a repository")?
            .git_root
    };
    let target_branch = current_branch(&target)?;
    let target_head = answer(&target, &["rev-parse", "HEAD"])?.context("Target has no commits")?;
    let stash = stash_changes(&source)?;
    let result = (|| -> Result<()> {
        if to_local {
            let requested = params["branch"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .or(branch.as_deref())
                .context("Choose a branch name")?;
            checked(run(&source, &["checkout", "--detach"])?)?;
            if answer(
                &target,
                &["show-ref", "--verify", &format!("refs/heads/{requested}")],
            )?
            .is_some()
            {
                checked(run(&target, &["checkout", requested])?)?;
                ensure!(
                    answer(&target, &["rev-parse", "HEAD"])?.as_deref() == Some(head.as_str()),
                    "The selected branch points to another commit"
                );
            } else {
                checked(run(&target, &["checkout", "-b", requested, &head])?)?;
            }
        }
        if !to_local || params["bringChanges"] == true {
            apply_stash(&target, stash.as_deref())?;
        } else {
            apply_stash(&source, stash.as_deref())?;
        }
        if !to_local && let Some(thread) = params["conversationId"].as_str() {
            set_owner_thread(&target, thread)?;
        }
        Ok(())
    })();
    let record = json!({"source": source, "target": target, "sourceBranch": branch,
        "sourceHead": head, "targetBranch": target_branch, "targetHead": target_head,
        "stash": stash, "cwd": target.join(prefix.trim_end_matches('/'))});
    if let Err(error) = result {
        rollback_move(&record).with_context(|| {
            format!("Move failed: {error:#}; rollback failed; the stash is retained")
        })?;
        return Err(error);
    }
    Ok(record)
}

pub(in crate::host) fn rollback_move(params: &Value) -> Result<Value> {
    let source = Path::new(params["source"].as_str().context("missing source")?);
    let target = Path::new(params["target"].as_str().context("missing target")?);
    ensure!(
        source.is_absolute() && target.is_absolute(),
        "Workspace must be absolute"
    );
    // Preserve a partial apply before switching back; never discard a user's files.
    stash_changes(target)?;
    let target_ref = params["targetBranch"]
        .as_str()
        .or(params["targetHead"].as_str())
        .context("missing target revision")?;
    checked(run(target, &["checkout", target_ref])?)?;
    let source_ref = params["sourceBranch"]
        .as_str()
        .or(params["sourceHead"].as_str())
        .context("missing source revision")?;
    checked(run(source, &["checkout", source_ref])?)?;
    apply_stash(source, params["stash"].as_str())?;
    Ok(json!({"success": true}))
}

pub(in crate::host) fn restore_thread_worktree(params: &Value) -> Result<Value> {
    let cwd = Path::new(params["cwd"].as_str().context("missing cwd")?);
    ensure!(
        cwd.is_absolute() && !cwd.exists(),
        "The workspace must be missing"
    );
    for root in params["candidateRoots"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let root = Path::new(root);
        if !root.is_absolute() || !root.is_dir() {
            continue;
        }
        for candidate in cwd.ancestors().take_while(|candidate| !candidate.exists()) {
            let id = format!("{:x}", Sha1::digest(candidate.to_string_lossy().as_bytes()));
            let reference = format!("refs/codex/snapshots/{id}");
            if let Some(snapshot) = answer(root, &["rev-parse", "--verify", &reference])? {
                checked(run(
                    root,
                    &[
                        "worktree",
                        "add",
                        "--detach",
                        candidate.to_str().context("Invalid workspace")?,
                        &snapshot,
                    ],
                )?)?;
                if let Some(parent) = answer(candidate, &["rev-parse", "--verify", "HEAD^"])? {
                    checked(run(candidate, &["reset", "--mixed", &parent])?)?;
                }
                ensure!(
                    cwd.is_dir(),
                    "The restored snapshot does not contain the working directory"
                );
                if let Some(thread) = params["conversationId"].as_str() {
                    set_owner_thread(candidate, thread)?;
                }
                return Ok(json!({"success": true, "cwd": cwd}));
            }
        }
    }
    bail!("No saved snapshot exists for this worktree");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> Result<(tempfile::TempDir, PathBuf)> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("repo");
        fs::create_dir(&root)?;
        checked(run(&root, &["init", "-b", "main"])?)?;
        checked(run(&root, &["config", "user.name", "Test"])?)?;
        checked(run(&root, &["config", "user.email", "test@example.com"])?)?;
        fs::write(root.join("file"), "original\n")?;
        checked(run(&root, &["add", "."])?)?;
        checked(run(&root, &["commit", "-m", "Initial"])?)?;
        Ok((temp, root))
    }

    #[test]
    fn moving_preserves_index_untracked_files_and_can_roll_back() -> Result<()> {
        let (temp, root) = repository()?;
        fs::write(root.join("file"), "staged\n")?;
        checked(run(&root, &["add", "file"])?)?;
        fs::write(root.join("file"), "unstaged\n")?;
        fs::write(root.join("new"), "untracked\n")?;
        let before = checked(run(&root, &["status", "--porcelain"])?)?;
        let moved = move_thread(&json!({"cwd": root}), &temp.path().join("worktrees"))?;
        let target = Path::new(moved["target"].as_str().unwrap());
        assert_eq!(fs::read_to_string(target.join("file"))?, "unstaged\n");
        assert_eq!(fs::read_to_string(target.join("new"))?, "untracked\n");
        assert_eq!(checked(run(target, &["status", "--porcelain"])?)?, before);
        assert!(checked(run(&root, &["status", "--porcelain"])?)?.is_empty());
        rollback_move(&moved)?;
        assert_eq!(checked(run(&root, &["status", "--porcelain"])?)?, before);
        assert_eq!(fs::read_to_string(root.join("file"))?, "unstaged\n");
        Ok(())
    }

    #[test]
    fn handoff_refuses_dirty_local_checkout_without_moving_changes() -> Result<()> {
        let (temp, root) = repository()?;
        let worktree =
            create_worktree(&temp.path().join("worktrees"), &root, Some("HEAD"), None)?.unwrap();
        fs::write(root.join("file"), "local\n")?;
        fs::write(worktree.git_root.join("file"), "worktree\n")?;
        let result = move_thread(
            &json!({
                "cwd": worktree.git_root, "workspace": root, "branch": "feature",
                "toLocal": true, "bringChanges": true,
            }),
            &temp.path().join("worktrees"),
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("local checkout has changes")
        );
        assert_eq!(fs::read_to_string(root.join("file"))?, "local\n");
        assert_eq!(
            fs::read_to_string(worktree.git_root.join("file"))?,
            "worktree\n"
        );
        Ok(())
    }

    #[test]
    fn handoff_moves_changes_to_selected_local_branch() -> Result<()> {
        let (temp, root) = repository()?;
        let managed = temp.path().join("worktrees");
        let worktree = create_worktree(&managed, &root, Some("HEAD"), None)?.unwrap();
        fs::write(worktree.git_root.join("file"), "staged\n")?;
        checked(run(&worktree.git_root, &["add", "file"])?)?;
        fs::write(worktree.git_root.join("new"), "untracked\n")?;
        let before = checked(run(&worktree.git_root, &["status", "--porcelain"])?)?;
        move_thread(
            &json!({
                "cwd": worktree.git_root, "workspace": root, "branch": "handoff",
                "toLocal": true, "bringChanges": true,
            }),
            &managed,
        )?;
        assert_eq!(current_branch(&root)?.as_deref(), Some("handoff"));
        assert_eq!(checked(run(&root, &["status", "--porcelain"])?)?, before);
        assert_eq!(fs::read_to_string(root.join("new"))?, "untracked\n");
        assert!(checked(run(&worktree.git_root, &["status", "--porcelain"])?)?.is_empty());
        Ok(())
    }

    #[test]
    fn handoff_without_changes_leaves_the_worktree_draft_intact() -> Result<()> {
        let (temp, root) = repository()?;
        let managed = temp.path().join("worktrees");
        let worktree = create_worktree(&managed, &root, Some("HEAD"), None)?.unwrap();
        fs::write(worktree.git_root.join("file"), "draft\n")?;
        fs::write(worktree.git_root.join("new"), "untracked\n")?;
        move_thread(
            &json!({
                "cwd": worktree.git_root, "workspace": root, "branch": "handoff",
                "toLocal": true, "bringChanges": false,
            }),
            &managed,
        )?;
        assert_eq!(
            fs::read_to_string(worktree.git_root.join("file"))?,
            "draft\n"
        );
        assert_eq!(
            fs::read_to_string(worktree.git_root.join("new"))?,
            "untracked\n"
        );
        assert!(checked(run(&root, &["status", "--porcelain"])?)?.is_empty());
        Ok(())
    }

    #[test]
    fn cleaned_worktree_restores_saved_changes() -> Result<()> {
        let (temp, root) = repository()?;
        let managed = temp.path().join("worktrees");
        let worktree = create_worktree(&managed, &root, Some("HEAD"), None)?.unwrap();
        fs::write(worktree.git_root.join("file"), "saved\n")?;
        fs::write(worktree.git_root.join("new"), "untracked\n")?;
        remove_managed_worktree(&worktree.git_root, &[managed])?;
        assert!(!worktree.git_root.exists());
        restore_thread_worktree(&json!({"cwd": worktree.git_root, "candidateRoots": [root]}))?;
        assert_eq!(
            fs::read_to_string(worktree.git_root.join("file"))?,
            "saved\n"
        );
        assert_eq!(
            fs::read_to_string(worktree.git_root.join("new"))?,
            "untracked\n"
        );
        assert!(!checked(run(&worktree.git_root, &["status", "--porcelain"])?)?.is_empty());
        Ok(())
    }

    #[test]
    fn restoring_a_thread_in_a_subdirectory_finds_the_checkout_snapshot() -> Result<()> {
        let (temp, root) = repository()?;
        let managed = temp.path().join("worktrees");
        let worktree = create_worktree(&managed, &root, Some("HEAD"), None)?.unwrap();
        let cwd = worktree.git_root.join("project");
        fs::create_dir(&cwd)?;
        fs::write(cwd.join("draft"), "saved draft\n")?;
        remove_managed_worktree(&worktree.git_root, &[managed])?;
        restore_thread_worktree(&json!({"cwd": cwd, "candidateRoots": [root]}))?;
        assert_eq!(fs::read_to_string(cwd.join("draft"))?, "saved draft\n");
        Ok(())
    }

    #[test]
    fn reservation_names_exclude_legacy_and_reserved_paths() {
        for name in [
            "con", "nul", "com1", "lpt9", "abcd", "-one", "one-", "a--b", "A", "",
        ] {
            assert!(!valid_name(name), "{name}");
        }
        for name in ["one", "worktree-two", "012g"] {
            assert!(valid_name(name), "{name}");
        }
        assert!(legacy_directory("250101-1200-abcdef12"));
        assert!(!legacy_directory("250101-1200-abcdef1g"));
        assert!(!legacy_directory("250101-1200-abcdef12-more"));
    }
}
