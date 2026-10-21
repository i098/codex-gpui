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
                if !legacy && modern && (git_dir.is_none() || !path.join(".git").is_file()) {
                    continue;
                }
                let common = answer(
                    &path,
                    &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                )?;
                let repository = common.as_deref().and_then(|path| Path::new(path).parent());
                worktrees
                    .push(json!({"dir": path, "gitDir": git_dir, "repositoryRoot": repository}));
            }
        }
    }
    worktrees.sort_by(|a, b| a["dir"].as_str().cmp(&b["dir"].as_str()));
    Ok(json!({"worktrees": worktrees}))
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

fn snapshot(root: &Path) -> Result<()> {
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
        && let Ok(environment) = environment(&config)
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
    if marker.is_file() {
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
