use super::*;
use std::collections::BTreeSet;

pub(crate) fn apply_cloud_patch(params: &Value) -> Result<Value> {
    let cwd = Path::new(params["cwd"].as_str().context("missing Git cwd")?);
    ensure!(cwd.is_absolute(), "Git cwd must be absolute");
    let diff = params["diff"].as_str().context("missing Git diff")?;
    let found = run(cwd, &["rev-parse", "--show-toplevel"])?;
    if !found.status.success() {
        return Ok(
            json!({"status":"error", "errorCode":"not-git-repo", "appliedPaths":[], "skippedPaths":[], "conflictedPaths":[]}),
        );
    }
    let root = PathBuf::from(String::from_utf8(found.stdout)?.trim_end());
    let scratch = tempfile::tempdir()?;
    let index = git_path(&root, "index")?;
    let temp_index = scratch.path().join("index");
    if index.exists() {
        fs::copy(&index, &temp_index)?;
    }
    if let Some(shared) = answer(&root, &["rev-parse", "--shared-index-path"])? {
        let shared = PathBuf::from(shared);
        let shared = if shared.is_absolute() {
            shared
        } else {
            root.join(shared)
        };
        if shared.is_file() {
            fs::copy(
                &shared,
                scratch
                    .path()
                    .join(shared.file_name().context("shared index filename")?),
            )?;
        }
    }
    let patch = scratch.path().join("patch.diff");
    fs::write(&patch, diff)?;
    let stat = run(
        &root,
        &[
            "apply",
            "--numstat",
            "-z",
            patch.to_str().context("patch path must be UTF-8")?,
        ],
    )?;
    let paths = patch_paths(&checked(stat)?)?;
    let existing: Vec<_> = paths
        .iter()
        .filter(|path| root.join(path).symlink_metadata().is_ok())
        .collect();
    if !existing.is_empty() {
        let mut add = command(&root, &["add", "--"]);
        add.env("GIT_INDEX_FILE", &temp_index).args(existing);
        checked(add.output()?)?;
    }
    let mut apply = command(&root, &["apply"]);
    apply.env("GIT_INDEX_FILE", &temp_index).arg("--3way");
    if params["revert"] == true {
        apply.arg("--reverse");
    }
    let output = apply.arg(&patch).output()?;
    let status = match output.status.code() {
        Some(0) => "success",
        Some(1) => "partial-success",
        _ => "error",
    };
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let (mut applied, mut skipped, conflicted) = parse_results(&text, &paths);
    if status == "success" && applied.is_empty() {
        applied.extend(paths);
    }
    for path in &conflicted {
        applied.remove(path);
        skipped.remove(path);
    }
    for path in &applied {
        skipped.remove(path);
    }
    Ok(
        json!({"status":status,"appliedPaths":applied,"skippedPaths":skipped,"conflictedPaths":conflicted,"execOutput":text}),
    )
}

fn patch_paths(bytes: &[u8]) -> Result<BTreeSet<String>> {
    let text = std::str::from_utf8(bytes).context("Git paths must be UTF-8")?;
    let mut records = text.split('\0').filter(|part| !part.is_empty());
    let mut paths = BTreeSet::new();
    while let Some(record) = records.next() {
        let mut columns = record.splitn(3, '\t');
        columns.next().context("missing patch additions")?;
        columns.next().context("missing patch deletions")?;
        let path = columns.next().context("missing patch path")?;
        if path.is_empty() {
            paths.insert(valid_path(
                records.next().context("missing rename source")?,
            )?);
            paths.insert(valid_path(
                records.next().context("missing rename target")?,
            )?);
        } else {
            paths.insert(valid_path(path)?);
        }
    }
    ensure!(!paths.is_empty(), "patch has no files");
    Ok(paths)
}

fn valid_path(path: &str) -> Result<String> {
    ensure!(
        Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "patch paths must be repository-relative"
    );
    Ok(path.to_owned())
}

fn parse_results(
    text: &str,
    paths: &BTreeSet<String>,
) -> (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>) {
    let (mut applied, mut skipped, mut conflicted) =
        (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for line in text.lines().map(str::trim) {
        for path in paths {
            let quoted = format!("'{path}'");
            if line.starts_with("U ") && line[2..].trim() == path
                || line.starts_with("Applied patch")
                    && line.contains(&quoted)
                    && line.ends_with("with conflicts.")
            {
                conflicted.insert(path.clone());
            } else if line.starts_with("Applied patch")
                && line.contains(&quoted)
                && line.ends_with("cleanly.")
            {
                applied.insert(path.clone());
            } else if line.starts_with("error:") && line.contains(path.as_str()) {
                skipped.insert(path.clone());
            }
        }
    }
    (applied, skipped, conflicted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_and_revert_preserve_the_real_index_and_unrelated_edits() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        checked(run(root, &["init"]).unwrap()).unwrap();
        fs::write(root.join("a.txt"), "before\n").unwrap();
        fs::write(root.join("b.txt"), "keep\n").unwrap();
        checked(run(root, &["add", "."]).unwrap()).unwrap();
        checked(
            run(
                root,
                &[
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.com",
                    "commit",
                    "-m",
                    "base",
                ],
            )
            .unwrap(),
        )
        .unwrap();
        let index = fs::read(git_path(root, "index").unwrap()).unwrap();
        fs::write(root.join("b.txt"), "unrelated\n").unwrap();
        let diff =
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-before\n+after\n";
        for (revert, expected) in [(false, "after\n"), (true, "before\n")] {
            let result =
                apply_cloud_patch(&json!({"cwd":root,"diff":diff,"revert":revert})).unwrap();
            assert_eq!(result["status"], "success", "{result}");
            assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), expected);
            assert_eq!(
                fs::read_to_string(root.join("b.txt")).unwrap(),
                "unrelated\n"
            );
            assert_eq!(fs::read(git_path(root, "index").unwrap()).unwrap(), index);
        }
    }

    #[test]
    fn missing_repository_is_not_reported_as_success() {
        let dir = tempfile::tempdir().unwrap();
        let result = apply_cloud_patch(&json!({"cwd":dir.path(),"diff":""})).unwrap();
        assert_eq!(result["errorCode"], "not-git-repo");
        assert_eq!(result["status"], "error");
    }

    fn repository() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        checked(run(root, &["init"]).unwrap()).unwrap();
        fs::write(root.join("a.txt"), "before\n").unwrap();
        checked(run(root, &["add", "."]).unwrap()).unwrap();
        checked(
            run(
                root,
                &[
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.com",
                    "commit",
                    "-m",
                    "base",
                ],
            )
            .unwrap(),
        )
        .unwrap();
        directory
    }

    #[test]
    fn rejected_hunks_preserve_the_file_and_the_original_partial_status() {
        let directory = repository();
        let root = directory.path();
        fs::write(root.join("a.txt"), "local\n").unwrap();
        let result = apply_cloud_patch(&json!({
            "cwd":root,
            "diff":"diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-before\n+remote\n"
        })).unwrap();
        assert_eq!(result["status"], "partial-success");
        assert_eq!(result["appliedPaths"], json!([]));
        assert_eq!(result["skippedPaths"], json!(["a.txt"]));
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "local\n");
    }

    #[test]
    fn three_way_conflicts_are_reported_without_changing_the_real_index() {
        let directory = repository();
        let root = directory.path();
        fs::write(root.join("a.txt"), "remote\n").unwrap();
        let patch =
            String::from_utf8(checked(run(root, &["diff", "--full-index"]).unwrap()).unwrap())
                .unwrap();
        fs::write(root.join("a.txt"), "local\n").unwrap();
        let index = fs::read(git_path(root, "index").unwrap()).unwrap();
        let result = apply_cloud_patch(&json!({"cwd":root,"diff":patch})).unwrap();
        assert_eq!(result["status"], "partial-success");
        assert_eq!(result["conflictedPaths"], json!(["a.txt"]));
        let text = fs::read_to_string(root.join("a.txt")).unwrap();
        assert!(text.contains("<<<<<<<") && text.contains("local") && text.contains("remote"));
        assert_eq!(fs::read(git_path(root, "index").unwrap()).unwrap(), index);
    }

    #[test]
    fn rename_patches_preserve_repository_relative_paths() {
        let directory = repository();
        let root = directory.path();
        checked(run(root, &["mv", "a.txt", "renamed.txt"]).unwrap()).unwrap();
        let patch = String::from_utf8(
            checked(run(root, &["diff", "--cached", "--full-index"]).unwrap()).unwrap(),
        )
        .unwrap();
        checked(run(root, &["reset", "--hard", "HEAD"]).unwrap()).unwrap();
        let result = apply_cloud_patch(&json!({"cwd":root,"diff":patch})).unwrap();
        assert_eq!(result["status"], "success", "{result}");
        assert!(!root.join("a.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("renamed.txt")).unwrap(),
            "before\n"
        );
    }
}
