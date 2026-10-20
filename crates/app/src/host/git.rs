use anyhow::{Context as _, Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::hash::BuildHasher as _;
use std::io::{ErrorKind, Read as _, Write as _};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Git in `root` without pathspec options: `check-ignore` rejects `--literal-pathspecs`, and
/// the original's `ls-files` calls use pathspec magic.
fn plain_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .args(["-c", "core.quotePath=false"])
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

fn command(root: &Path, args: &[&str]) -> Command {
    let mut command = plain_command(root);
    command.arg("--literal-pathspecs").args(args);
    command
}

fn run(root: &Path, args: &[&str]) -> Result<Output> {
    command(root, args).output().context("run local Git")
}

fn checked(output: Output) -> Result<Vec<u8>> {
    ensure!(
        output.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output.stdout)
}

fn paths(root: &Path, staged: bool) -> Result<Vec<String>> {
    let args = if staged {
        vec![
            "diff",
            "--cached",
            "--name-only",
            "--no-renames",
            "-z",
            "--",
        ]
    } else {
        vec!["diff", "--name-only", "--no-renames", "-z", "--"]
    };
    let mut bytes = checked(run(root, &args)?)?;
    if !staged {
        bytes.extend(checked(run(
            root,
            &["ls-files", "--others", "--exclude-standard", "-z"],
        )?)?);
    }
    let mut paths = String::from_utf8(bytes)
        .context("Git paths must be UTF-8")?
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

#[derive(Serialize)]
struct CapturedDiff {
    path: String,
    #[serde(serialize_with = "serialize_diff")]
    diff: Vec<u8>,
}

fn serialize_diff<S: serde::Serializer>(
    diff: &[u8],
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(&String::from_utf8_lossy(diff))
}

/// `binary` adds the data a reverse patch needs to restore a binary file. Both forms carry
/// the full `index` line (`--binary` fills it for binary files only), so `head` compares one
/// binary capture with the text Changes shows.
fn diffs(root: &Path, staged: bool, binary: bool) -> Result<Vec<CapturedDiff>> {
    review_diffs(
        root,
        if staged { &["--cached"] } else { &[] },
        binary,
        !staged,
    )
}

fn review_diffs(
    root: &Path,
    revisions: &[&str],
    binary: bool,
    untracked: bool,
) -> Result<Vec<CapturedDiff>> {
    let mut args = vec![
        "diff",
        "--raw",
        "--patch",
        "-z",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--no-renames",
        "--submodule=short",
        "--full-index",
    ];
    args.extend_from_slice(revisions);
    if binary {
        args.push("--binary");
    }
    args.push("--");
    let output = checked(run(root, &args)?)?;
    let mut files = Vec::new();
    if !output.is_empty() {
        let separator = output
            .windows(2)
            .position(|bytes| bytes == b"\0\0")
            .context("missing Git diff path terminator")?;
        let mut records = std::str::from_utf8(&output[..separator])
            .context("Git paths must be UTF-8")?
            .split('\0');
        let patch = &output[separator + 2..];
        ensure!(
            patch.starts_with(b"diff --git "),
            "unsupported Git diff format"
        );
        let delimiter = b"\ndiff --git ";
        let starts = std::iter::once(0)
            .chain(
                patch
                    .windows(delimiter.len())
                    .enumerate()
                    .filter_map(|(i, bytes)| (bytes == delimiter).then_some(i + 1)),
            )
            .chain(std::iter::once(patch.len()))
            .collect::<Vec<_>>();
        for range in starts.windows(2) {
            records.next().context("missing Git diff metadata")?;
            let path = records.next().context("missing Git diff path")?;
            let mut diff = patch[range[0]..range[1]].to_vec();
            if !diff.ends_with(b"\n") {
                diff.push(b'\n');
            }
            files.push(CapturedDiff {
                path: path.to_owned(),
                diff,
            });
        }
        ensure!(
            records.next().is_none(),
            "Git diff paths and patches differ"
        );
    }
    if untracked {
        let untracked = checked(run(
            root,
            &["ls-files", "--others", "--exclude-standard", "-z"],
        )?)?;
        for path in std::str::from_utf8(&untracked)
            .context("Git paths must be UTF-8")?
            .split('\0')
            .filter(|path| !path.is_empty())
        {
            let mut args = vec![
                "diff",
                "--no-index",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--full-index",
            ];
            if binary {
                args.push("--binary");
            }
            args.extend(["--", "/dev/null", path]);
            let output = run(root, &args)?;
            ensure!(
                output.status.success() || output.status.code() == Some(1),
                "Git: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            files.push(CapturedDiff {
                path: path.to_owned(),
                diff: output.stdout,
            });
        }
    }
    Ok(files)
}

/// One file's diff up to its binary data: the text form says `Binary files … differ` where
/// the `--binary` form carries the data, and the full `index` line pins that data.
fn head(diff: &str) -> &str {
    ["\nGIT binary patch\n", "\nBinary files "]
        .into_iter()
        .find_map(|mark| diff.split_once(mark))
        .map_or(diff, |(head, _)| head)
}

/// Applies the reviewed patch to the index (`cached`) or the working tree.
/// Git applies a patch atomically, and refuses it when the target no longer matches.
fn apply(root: &Path, patch: &[u8], cached: bool, reverse: bool) -> Result<()> {
    let mut args = vec!["apply"];
    if reverse {
        args.push("--reverse");
    }
    if cached {
        args.push("--cached");
    }
    let mut child = command(root, &args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("run local Git")?;
    let sent = child.stdin.take().context("Git stdin")?.write_all(patch);
    checked(child.wait_with_output().context("run local Git")?)?;
    sent.context("send patch to Git")
}

/// The header of `diff`, without mode changes, with only its `index`th hunk: a hunk revert
/// must not change the file's permissions.
fn hunk(diff: &[u8], index: usize) -> Result<Vec<u8>> {
    let lines = diff
        .split_inclusive(|byte| *byte == b'\n')
        .collect::<Vec<_>>();
    let starts = (0..lines.len())
        .filter(|&i| lines[i].starts_with(b"@@ "))
        .collect::<Vec<_>>();
    let start = *starts
        .get(index)
        .context("hunk is no longer in the selected diff")?;
    let end = starts.get(index + 1).copied().unwrap_or(lines.len());
    let header = lines[..starts[0]]
        .iter()
        .filter(|line| !line.starts_with(b"old mode ") && !line.starts_with(b"new mode "));
    Ok(header
        .chain(&lines[start..end])
        .flat_map(|line| line.iter().copied())
        .collect())
}

/// Reverts one file, or one hunk of it, in the `source` the caller displayed. One capture
/// serves both the stale check and the patch, so an edit made while the request runs can
/// only make the check fail, never enter the patch. The original reverts a staged change in
/// the index and then in the working tree: when only the second step fails, or the working
/// file already differs from the index so that Git could move the patch onto another
/// matching block, the result is `partial-success`.
fn revert(root: &Path, path: &str, params: &Value) -> Result<&'static str> {
    let staged = match params["source"].as_str() {
        Some("staged") => true,
        Some("unstaged") => false,
        _ => bail!("Git revert source must be staged or unstaged"),
    };
    let displayed = params["diff"].as_str().context("missing Git diff")?;
    let current = diffs(root, staged, true)?
        .into_iter()
        .find(|file| file.path == path)
        .map(|file| file.diff)
        .context("file is no longer in the selected changes")?;
    ensure!(
        head(&String::from_utf8_lossy(&current)) == head(displayed),
        "file changed since its diff was shown; refresh and try again"
    );
    let patch = match params.get("hunk") {
        None | Some(Value::Null) => current,
        Some(index) => hunk(
            &current,
            index.as_u64().context("Git hunk must be an index")? as usize,
        )?,
    };
    if staged {
        let differs = run(root, &["diff", "--quiet", "--no-ext-diff", "--", path])?;
        ensure!(
            differs.status.success() || differs.status.code() == Some(1),
            "Git: {}",
            String::from_utf8_lossy(&differs.stderr).trim()
        );
        apply(root, &patch, true, true)?;
        if !differs.status.success() {
            return Ok("partial-success");
        }
    }
    match apply(root, &patch, false, true) {
        Ok(()) => Ok("success"),
        Err(_) if staged => Ok("partial-success"),
        Err(error) => Err(error),
    }
}

fn selected_patch(root: &Path, params: &Value) -> Result<(Vec<u8>, bool, Vec<String>)> {
    let staged = match params["source"].as_str() {
        Some("staged") => true,
        Some("unstaged") => false,
        _ => bail!("Git patch source must be staged or unstaged"),
    };
    let current = diffs(root, staged, true)?;
    let files = params["files"]
        .as_array()
        .context("missing reviewed Git files")?;
    ensure!(!files.is_empty(), "no reviewed files");
    let mut patch = Vec::new();
    let mut paths = Vec::new();
    for file in files {
        let path = file["path"].as_str().context("missing Git path")?;
        ensure!(
            !path.is_empty()
                && Path::new(path)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_)))
                && !paths.iter().any(|p| p == path),
            "Git paths must be unique and repository-relative"
        );
        let displayed = file["diff"].as_str().context("missing Git diff")?;
        let captured = current
            .iter()
            .find(|f| f.path == path)
            .map(|f| f.diff.as_slice())
            .context("file is no longer in the selected changes")?;
        ensure!(
            head(&String::from_utf8_lossy(captured)) == head(displayed),
            "file changed since its diff was shown; refresh and try again"
        );
        if let Some(index) = file.get("hunk").filter(|v| !v.is_null()) {
            patch.extend_from_slice(&hunk(
                captured,
                index.as_u64().context("Git hunk must be an index")? as usize,
            )?);
        } else {
            patch.extend_from_slice(captured);
        }
        paths.push(path.to_owned());
    }
    Ok((patch, staged, paths))
}

fn patch_changes(root: &Path, endpoint: &str, params: &Value) -> Result<&'static str> {
    let (patch, staged, paths) = selected_patch(root, params)?;
    match endpoint {
        "git-stage" => {
            ensure!(!staged, "stage requires unstaged changes");
            apply(root, &patch, true, false)?;
        }
        "git-unstage" => {
            ensure!(staged, "unstage requires staged changes");
            apply(root, &patch, true, true)?;
        }
        "git-revert" => {
            if staged {
                let mut args = vec!["diff", "--quiet", "--no-ext-diff", "--"];
                args.extend(paths.iter().map(String::as_str));
                let differs = run(root, &args)?;
                ensure!(
                    differs.status.success() || differs.status.code() == Some(1),
                    "Git diff failed"
                );
                apply(root, &patch, true, true)?;
                if !differs.status.success() {
                    return Ok("partial-success");
                }
            }
            match apply(root, &patch, false, true) {
                Ok(()) => {}
                Err(_) if staged => return Ok("partial-success"),
                Err(error) => return Err(error),
            }
        }
        _ => bail!("unsupported Git patch operation"),
    }
    Ok("success")
}

fn changes(root: &Path, params: &Value) -> Result<Value> {
    let mut body = json!({"root": root, "unstaged": diffs(root, false, false)?, "staged": diffs(root, true, false)?});
    let head = commit_id(root, "HEAD")?;
    let branches = checked(run(
        root,
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads",
            "refs/remotes",
        ],
    )?)?;
    let branches = String::from_utf8(branches)?;
    body["branches"] = branches
        .lines()
        .map(|s| Value::from(s.to_owned()))
        .collect();
    let commits = if head.is_some() {
        String::from_utf8(checked(run(
            root,
            &["log", "-100", "--format=%H %s", "HEAD", "--"],
        )?)?)?
    } else {
        String::new()
    };
    body["commits"] = commits
        .lines()
        .filter_map(|line| {
            let (id, subject) = line.split_once(' ')?;
            Some(json!({"id": id, "label": format!("{} {subject}", &id[..8])}))
        })
        .collect();
    let empty = String::from_utf8(checked(run(
        root,
        &["hash-object", "-t", "tree", "/dev/null"],
    )?)?)?;
    let base = head.as_deref().unwrap_or(empty.trim());
    let source = params["reviewSource"].as_str().unwrap_or("unstaged");
    body["review"] = match source {
        "uncommitted" => json!(review_diffs(root, &[base], false, true)?),
        "branch" | "commits" if head.is_none() => json!([]),
        "branch" => {
            let branch = params["baseBranch"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| branches.lines().find(|s| *s == "main" || *s == "master"))
                .or_else(|| branches.lines().next())
                .context("select a base branch")?;
            ensure!(
                branches.lines().any(|s| s == branch),
                "base branch no longer exists"
            );
            let target = commit_id(root, branch)?.context("base branch has no commit")?;
            let merge = answer(root, &["merge-base", &target, "HEAD"])?
                .context("branches have no common ancestor")?;
            body["base_branch"] = branch.into();
            json!(review_diffs(root, &[&merge, "HEAD"], false, false)?)
        }
        "commits" => {
            let id = params["commit"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or(base);
            ensure!(id.bytes().all(|b| b.is_ascii_hexdigit()), "invalid commit");
            let id = commit_id(root, id)?.context("commit no longer exists")?;
            let parent = commit_id(root, &format!("{id}^"))?;
            body["commit"] = id.clone().into();
            json!(review_diffs(
                root,
                &[parent.as_deref().unwrap_or(empty.trim()), &id],
                false,
                false
            )?)
        }
        _ => json!([]),
    };
    Ok(body)
}

pub(super) fn fetch(endpoint: &str, params: &Value) -> Result<Value> {
    let cwd = params["cwd"].as_str().context("missing Git cwd")?;
    ensure!(Path::new(cwd).is_absolute(), "Git cwd must be absolute");
    let cwd = Path::new(cwd);
    let found = run(cwd, &["rev-parse", "--show-toplevel"])?;
    if !found.status.success() {
        let error = String::from_utf8_lossy(&found.stderr);
        if endpoint == "git-init-repo" {
            ensure!(
                error.contains("not a git repository"),
                "Git: {}",
                error.trim()
            );
            ensure!(cwd.is_dir(), "Git cwd must be a directory");
            checked(run(cwd, &["init"])?)?;
        } else if endpoint == "git-changes" {
            ensure!(
                error.contains("not a git repository"),
                "Git: {}",
                error.trim()
            );
            return Ok(json!({"root": cwd, "unstaged": [], "staged": [], "not_repository": true}));
        } else {
            checked(found)?;
        }
    } else if endpoint == "git-init-repo" {
        bail!("folder is already in a Git repository");
    }
    let root = checked(run(cwd, &["rev-parse", "--show-toplevel"])?)?;
    let root = String::from_utf8(root).context("Git root must be UTF-8")?;
    let root = Path::new(root.strip_suffix('\n').unwrap_or(&root));
    if let Some(result) = commit_flow(endpoint, root, params) {
        return result;
    }
    let mut status = None;
    if params.get("files").is_some() {
        status = Some(patch_changes(root, endpoint, params)?);
    } else if !matches!(endpoint, "git-changes" | "git-init-repo") {
        let path = params["path"].as_str().context("missing Git path")?;
        ensure!(
            !path.is_empty()
                && Path::new(path)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
            "Git path must be repository-relative"
        );
        if endpoint == "git-revert" {
            status = Some(revert(root, path, params)?);
        } else {
            let staged = endpoint == "git-unstage";
            ensure!(
                paths(root, staged)?.iter().any(|p| p == path),
                "file is no longer in the selected changes"
            );
            let args = if staged {
                vec!["reset", "--quiet", "--", path]
            } else {
                vec!["add", "--", path]
            };
            checked(run(root, &args)?)?;
        }
    }
    let mut body = changes(root, params)?;
    if let Some(status) = status {
        body["status"] = status.into();
    }
    Ok(body)
}

struct Git {
    success: bool,
    stdout: String,
    stderr: String,
}

impl Git {
    fn failure(&self) -> String {
        let text = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        text.trim().to_owned()
    }
}

fn git(root: &Path, args: &[&str]) -> Result<Git> {
    let output = run(root, args)?;
    Ok(Git {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn failed(stage: &str, error: impl Into<String>) -> Value {
    json!({"status": "error", "stage": stage, "error": error.into()})
}

fn commit_flow(endpoint: &str, root: &Path, params: &Value) -> Option<Result<Value>> {
    let include = params["includeUnstaged"].as_bool().unwrap_or(false);
    Some(match endpoint {
        "git-commit-status" => status(root),
        "git-commit-diff" => commit_diff(root, include),
        "git-commit" => commit(root, include, params),
        "git-push" => push(root, params),
        "git-checkout" => checkout(root, params),
        _ => return None,
    })
}

fn current_branch(root: &Path) -> Result<Option<String>> {
    let head = git(root, &["symbolic-ref", "-q", "HEAD"])?;
    if !head.success {
        return Ok(None);
    }
    Ok(Some(
        head.stdout
            .trim()
            .strip_prefix("refs/heads/")
            .context("HEAD is not a local branch reference")?
            .to_owned(),
    ))
}

fn upstream(root: &Path) -> Result<Option<String>> {
    let upstream = git(
        root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )?;
    Ok(upstream.success.then(|| upstream.stdout.trim().to_owned()))
}

/// The remote Git pushes `branch` to: its `pushRemote`, `remote.pushDefault`, its fetch
/// remote, else the first remote.
fn remote(root: &Path, branch: Option<&str>) -> Result<Option<String>> {
    let branch_keys = branch.into_iter().flat_map(|branch| {
        [
            format!("branch.{branch}.pushRemote"),
            "remote.pushDefault".to_owned(),
            format!("branch.{branch}.remote"),
        ]
    });
    for key in branch_keys {
        let configured = git(root, &["config", "--get", &key])?;
        if configured.success && !configured.stdout.trim().is_empty() {
            return Ok(Some(configured.stdout.trim().to_owned()));
        }
    }
    let remotes = git(root, &["remote"])?;
    Ok(remotes.stdout.lines().next().map(str::to_owned))
}

/// `(files, additions, deletions)` of one `git diff --numstat -z` call.
fn numstat(root: &Path, args: &[&str]) -> Result<(usize, usize, usize)> {
    let output = checked(run(root, args)?)?;
    let mut total = (0, 0, 0);
    for record in output.split(|byte| *byte == 0).filter(|r| !r.is_empty()) {
        let record = String::from_utf8_lossy(record);
        let mut columns = record.splitn(3, '\t');
        total.0 += 1;
        total.1 += columns.next().and_then(|n| n.parse().ok()).unwrap_or(0);
        total.2 += columns.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    }
    Ok(total)
}

fn untracked_lines(root: &Path) -> Result<(usize, usize)> {
    let paths = checked(run(
        root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?)?;
    let mut total = (0, 0);
    for path in paths.split(|byte| *byte == 0).filter(|p| !p.is_empty()) {
        total.0 += 1;
        let path = root.join(String::from_utf8_lossy(path).as_ref());
        if let Ok(bytes) = std::fs::read(path)
            && !bytes.contains(&0)
        {
            total.1 += bytes.iter().filter(|byte| **byte == b'\n').count()
                + usize::from(bytes.last().is_some_and(|byte| *byte != b'\n'));
        }
    }
    Ok(total)
}

fn status(root: &Path) -> Result<Value> {
    let branch = current_branch(root)?;
    let has_head = git(root, &["rev-parse", "--verify", "-q", "HEAD"])?.success;
    let upstream = upstream(root)?;
    let local_branches: Vec<String> = checked(run(
        root,
        &["for-each-ref", "--format=%(refname:lstrip=2)", "refs/heads"],
    )?)
    .map(|names| {
        String::from_utf8_lossy(&names)
            .lines()
            .map(str::to_owned)
            .collect()
    })?;
    let ahead = if !has_head {
        0
    } else {
        let range = if upstream.is_some() {
            vec!["rev-list", "--count", "@{u}..HEAD", "--"]
        } else {
            vec!["rev-list", "--count", "HEAD", "--not", "--remotes", "--"]
        };
        checked(run(root, &range)?).and_then(|count| {
            String::from_utf8_lossy(&count)
                .trim()
                .parse::<usize>()
                .context("Git commit count")
        })?
    };
    let staged = numstat(
        root,
        &["diff", "--cached", "--numstat", "-z", "--no-renames"],
    )?;
    let mut all = if has_head {
        numstat(
            root,
            &["diff", "HEAD", "--numstat", "-z", "--no-renames", "--"],
        )?
    } else {
        let unstaged = numstat(root, &["diff", "--numstat", "-z", "--no-renames"])?;
        (
            staged.0 + unstaged.0,
            staged.1 + unstaged.1,
            staged.2 + unstaged.2,
        )
    };
    let untracked = untracked_lines(root)?;
    all.0 += untracked.0;
    all.1 += untracked.1;
    let selection = |(files, additions, deletions): (usize, usize, usize)| json!({"files": files, "additions": additions, "deletions": deletions});
    Ok(json!({
        "root": root,
        "branch": branch,
        "hasCommits": has_head,
        "upstream": upstream,
        "remote": remote(root, branch.as_deref())?,
        "branches": local_branches,
        "commitsAhead": ahead,
        "staged": selection(staged),
        "all": selection(all),
    }))
}

/// The diff the original sends to message generation: tracked changes only.
fn commit_diff(root: &Path, include: bool) -> Result<Value> {
    let has_head = git(root, &["rev-parse", "--verify", "-q", "HEAD"])?.success;
    let base = [
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--no-renames",
    ];
    let mut calls: Vec<Vec<&str>> = Vec::new();
    match (include, has_head) {
        (true, true) => calls.push([&base[..], &["HEAD", "--"]].concat()),
        (true, false) => {
            calls.push([&base[..], &["--cached", "--"]].concat());
            calls.push([&base[..], &["--"]].concat());
        }
        (false, _) => calls.push([&base[..], &["--cached", "--"]].concat()),
    }
    let mut diff = String::new();
    for args in calls {
        diff.push_str(&String::from_utf8_lossy(&checked(run(root, &args)?)?));
    }
    Ok(json!({"unifiedDiff": diff}))
}

/// `git checkout -b` of `params.newBranch`, when given; an error value when it cannot.
fn create_branch(root: &Path, params: &Value) -> Result<Option<Value>> {
    let Some(name) = params["newBranch"].as_str() else {
        return Ok(None);
    };
    let reference = format!("refs/heads/{name}");
    if !git(root, &["check-ref-format", &reference])?.success {
        return Ok(Some(failed(
            "branch",
            format!("Invalid branch name {name}"),
        )));
    }
    if git(root, &["show-ref", "--verify", "--quiet", &reference])?.success {
        return Ok(Some(failed("branch", "Branch already exists")));
    }
    let created = git(root, &["checkout", "-b", name])?;
    Ok((!created.success).then(|| failed("branch", created.failure())))
}

fn checkout(root: &Path, params: &Value) -> Result<Value> {
    let branch = params["branch"].as_str();
    let new_branch = params["newBranch"].as_str();
    ensure!(
        branch.is_some() != new_branch.is_some(),
        "provide exactly one of branch or newBranch"
    );
    if let Some(name) = new_branch {
        if name.trim() != name || name.is_empty() || name.starts_with('-') || name == "HEAD" {
            return Ok(failed("branch", "Invalid branch name"));
        }
        if !git(root, &["rev-parse", "--verify", "-q", "HEAD"])?.success {
            return Ok(failed(
                "branch",
                "Commit changes to create and checkout a new branch",
            ));
        }
        if let Some(failure) = create_branch(root, params)? {
            return Ok(failure);
        }
    } else {
        let name = branch.unwrap().trim();
        if name.is_empty() {
            return Ok(failed("checkout", "Branch name is required"));
        }
        let reference = format!("refs/heads/{name}");
        let mut checkout = command(root, &["checkout"]);
        checkout.env("LC_ALL", "C");
        let mut found = false;
        if !name.starts_with('-') && git(root, &["check-ref-format", &reference])?.success {
            if git(root, &["show-ref", "--verify", "--quiet", &reference])?.success {
                checkout.args([name, "--"]);
                found = true;
            } else {
                let pattern = format!("refs/remotes/*/{name}");
                let remotes = checked(run(
                    root,
                    &["for-each-ref", "--count=2", "--format=%(refname)", &pattern],
                )?)?;
                let remotes = String::from_utf8(remotes)?;
                let mut refs = remotes.lines();
                if let Some(remote) = refs.next() {
                    if refs.next().is_some() {
                        return Ok(failed(
                            "checkout",
                            format!("Branch '{name}' exists on multiple remotes"),
                        ));
                    }
                    checkout.args(["--track", "-b", name, remote, "--"]);
                    found = true;
                }
            }
        }
        if !found {
            let commit = git(
                root,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "--end-of-options",
                    &format!("{name}^{{commit}}"),
                ],
            )?;
            if !commit.success {
                return Ok(failed(
                    "checkout",
                    format!("Branch or ref '{name}' was not found"),
                ));
            }
            checkout.args(["--detach", commit.stdout.trim(), "--"]);
        }
        let output = checkout.output().context("run local Git checkout")?;
        let result = Git {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        };
        if !result.success {
            let mut failure = failed("checkout", result.failure());
            let mut collecting = false;
            let mut paths = Vec::new();
            for line in result.stderr.lines() {
                if line.ends_with("would be overwritten by checkout:") {
                    collecting = true;
                } else if collecting {
                    if line.starts_with('\t') {
                        paths.push(line.trim());
                    } else {
                        collecting = false;
                    }
                }
            }
            failure["errorType"] = if !paths.is_empty() {
                "blocked-by-working-tree-changes"
            } else {
                "unknown"
            }
            .into();
            failure["conflictedPaths"] = json!(paths);
            return Ok(failure);
        }
    }
    Ok(json!({"status": "success", "branch": current_branch(root)?}))
}

fn commit(root: &Path, include: bool, params: &Value) -> Result<Value> {
    let message = params["message"]
        .as_str()
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .context("missing commit message")?;
    if let Some(failure) = create_branch(root, params)? {
        return Ok(failure);
    }
    if include {
        let staged = git(root, &["add", "-A"])?;
        if !staged.success {
            return Ok(failed("commit", staged.failure()));
        }
    }
    if git(root, &["diff", "--cached", "--quiet", "--exit-code"])?.success {
        let mut nothing = failed("commit", "No changes to commit");
        nothing["errorType"] = "nothing-to-commit".into();
        return Ok(nothing);
    }
    let committed = git(root, &["commit", "-m", message])?;
    if !committed.success {
        return Ok(failed("commit", committed.failure()));
    }
    let sha = git(root, &["rev-parse", "HEAD"])?;
    Ok(json!({
        "status": "success",
        "branch": current_branch(root)?,
        "commitSha": sha.success.then(|| sha.stdout.trim().to_owned()),
    }))
}

/// The original's push: set the upstream of a branch that has none; a tracked branch is
/// pushed by plain `git push`, so Git chooses the destination.
fn push(root: &Path, params: &Value) -> Result<Value> {
    if let Some(failure) = create_branch(root, params)? {
        return Ok(failure);
    }
    let Some(branch) = current_branch(root)? else {
        return Ok(failed("push", "Branch information unavailable"));
    };
    let has_upstream = upstream(root)?.is_some();
    let Some(remote) = remote(root, Some(&branch))? else {
        return Ok(failed("push", "No git remote configured for push"));
    };
    let refspec = format!("HEAD:refs/heads/{branch}");
    let args = if has_upstream {
        vec!["push", "--porcelain"]
    } else {
        vec!["push", "--porcelain", "-u", &remote, &refspec]
    };
    let pushed = git(root, &args)?;
    if !pushed.success {
        return Ok(failed("push", pushed.failure()));
    }
    Ok(json!({"status": "success", "branch": branch}))
}

/// A worktree made for one automation run: its Git root, and the source cwd's counterpart in
/// it, where the thread runs (original `worktreeGitRoot`, `worktreeWorkspaceRoot`).
pub(super) struct Worktree {
    pub git_root: PathBuf,
    pub workspace_root: PathBuf,
}

/// The reservation marker the original writes in each `<worktrees root>/<id>` directory.
const MARKER: &str = ".codex-worktree-name";
/// The owner-thread file in a worktree's Git directory (`e4`).
const OWNER: &str = "codex-thread.json";
const SHELL_ENVIRONMENT: &str = "codex-shell-environment.json";
const OVERRIDE: &str = "AGENTS.override.md";
const NO_ENVIRONMENT: &str = "__none__";
/// Variables a setup script never hands on (`Ewe`).
const UNSHARED: [&str; 9] = [
    "CODEX_SOURCE_TREE_PATH",
    "CODEX_WORKTREE_PATH",
    "CODEX_SETUP_EXIT_CODE",
    "OLDPWD",
    "PWD",
    "SHELLOPTS",
    "SHLVL",
    "_",
    "CODEX_SHELL",
];
const LOGIN_ENV: &str = "printf '\\0%s\\0' '_SHELL_ENV_DELIMITER_'; command env -0 || exit; printf '\\0%s\\0' '_SHELL_ENV_DELIMITER_'; exit";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Owner<'a> {
    version: u8,
    owner_thread_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncedBranch<'a> {
    branch: &'a str,
    last_synced_tree_ref: &'a str,
}

#[derive(Serialize)]
struct ShellEnvironment {
    version: u8,
    set: BTreeMap<String, String>,
    exclude: Vec<String>,
}

/// A local-environment file, `.codex/environments/<name>.toml` (`UF`).
#[derive(Deserialize)]
struct Environment {
    version: Option<i64>,
    #[allow(dead_code)]
    name: String,
    setup: Scripts,
    cleanup: Option<Scripts>,
}

#[derive(Deserialize)]
struct Scripts {
    script: String,
    darwin: Option<Script>,
    linux: Option<Script>,
    win32: Option<Script>,
}

#[derive(Deserialize)]
struct Script {
    script: String,
}

impl Scripts {
    /// This platform's script when it has one, else the shared one (`Lwe`).
    fn current(&self) -> &str {
        let platform = if cfg!(target_os = "macos") {
            &self.darwin
        } else if cfg!(windows) {
            &self.win32
        } else {
            &self.linux
        };
        platform
            .as_ref()
            .map(|s| s.script.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or(self.script.trim())
    }
}

/// Creates the worktree an automation runs in under `worktrees_root` (original git worker
/// `create-worktree`, `D2`). `Ok(None)` when `cwd` is in no Git repository: the original
/// then runs in `cwd` itself. `starting_ref` `None` is the automation's start (`Ol`): the
/// remote default branch, else `HEAD`. A failure removes what this call made, except a
/// worktree Git refuses to remove because it has changes.
pub(super) fn create_worktree(
    worktrees_root: &Path,
    cwd: &Path,
    starting_ref: Option<&str>,
    local_env_path: Option<&Path>,
) -> Result<Option<Worktree>> {
    let Some(source) = answer(cwd, &["rev-parse", "--show-toplevel"])? else {
        return Ok(None);
    };
    let source = PathBuf::from(source);
    let prefix = answer(cwd, &["rev-parse", "--show-prefix"])?.unwrap_or_default();
    let default = default_branch(&source)?;
    let name = starting_ref
        .map(str::to_owned)
        .or_else(|| default.clone())
        .unwrap_or_else(|| "HEAD".to_owned());
    fs::create_dir_all(worktrees_root)?;
    if cfg!(target_os = "macos") {
        fs::write(worktrees_root.join(".metadata_never_index"), "")?;
    }
    let repository = source
        .file_name()
        .context("Failed to resolve the worktree repository.")?;
    let id = format!("{:04x}", random() & 0xffff);
    let reserved = reserve(worktrees_root, &id).context(
        "Could not reserve the worktree directory. Check the worktrees directory permissions.",
    )?;
    let git_root = reserved.join(repository);
    let prefix = prefix.trim_end_matches('/');
    let workspace_root = if prefix.is_empty() {
        git_root.clone()
    } else {
        git_root.join(prefix)
    };
    let worktree = Worktree {
        git_root,
        workspace_root,
    };
    let start = match starting_commit(&source, &name)
        .and_then(|start| add_worktree(&source, &worktree.git_root, &start).map(|()| start))
    {
        Ok(start) => start,
        Err(error) => {
            let _ = fs::remove_dir_all(&reserved);
            return Err(error);
        }
    };
    let prepared = prepare(
        &source,
        cwd,
        &worktree,
        &name,
        &start,
        default.as_deref(),
        local_env_path,
    );
    if let Err(error) = prepared {
        return Err(match remove_unstarted_worktree(worktrees_root, &worktree) {
            Ok(()) => error,
            Err(kept) => anyhow!(
                "{error:#}; the new worktree {} was kept: {kept:#}",
                worktree.git_root.display()
            ),
        });
    }
    Ok(Some(worktree))
}

/// Records the thread that owns a worktree (`set-worktree-owner-thread`, `e4`).
pub(super) fn set_owner_thread(git_root: &Path, thread_id: &str) -> Result<()> {
    ensure!(
        linked_root(git_root)?,
        "Failed to set thread worktree owner"
    );
    write_json(
        &git_path(git_root, OWNER)?,
        &Owner {
            version: 1,
            owner_thread_id: thread_id,
        },
    )
}

/// Removes a worktree `create_worktree` made that no thread owns, after its environment's
/// cleanup script (`R0` without `--force`): Git refuses a worktree with changes, and this
/// refuses any path that is not a marked reservation directly under `worktrees_root`.
pub(super) fn remove_unstarted_worktree(worktrees_root: &Path, worktree: &Worktree) -> Result<()> {
    let root = &worktree.git_root;
    let reserved = root.parent().context("worktree has no reservation")?;
    let owned = reserved.parent().is_some_and(|parent| {
        fs::canonicalize(parent).ok() == fs::canonicalize(worktrees_root).ok()
    }) && fs::symlink_metadata(reserved.join(MARKER)).is_ok_and(|m| m.is_file())
        && linked_root(root)?;
    ensure!(
        owned && !git_path(root, OWNER)?.exists(),
        "refusing to remove {}: not an unstarted automation worktree",
        root.display()
    );
    let selected = answer(
        root,
        &[
            "config",
            "--worktree",
            "--get",
            "codex.localEnvironmentConfigPath",
        ],
    )?
    .filter(|path| path != NO_ENVIRONMENT);
    if let Some(config) = selected.map(PathBuf::from)
        && let Ok(environment) = environment(&config)
    {
        let script = environment.cleanup.as_ref().map_or("", Scripts::current);
        if !script.is_empty() {
            run_script(
                &root.join(script_dir(&config)),
                script,
                "Cleanup script",
                &[],
            )?;
        }
    }
    let removed = git(root, &["worktree", "remove", &root.to_string_lossy()])?;
    ensure!(removed.success, "{}", removed.failure());
    fs::remove_file(reserved.join(MARKER))?;
    let _ = fs::remove_dir(reserved);
    Ok(())
}

/// Trimmed stdout of a successful Git call that printed something.
fn answer(root: &Path, args: &[&str]) -> Result<Option<String>> {
    let output = git(root, args)?;
    let text = output.stdout.trim();
    Ok((output.success && !text.is_empty()).then(|| text.to_owned()))
}

fn commit_id(root: &Path, reference: &str) -> Result<Option<String>> {
    let commit = format!("{reference}^{{commit}}");
    answer(root, &["rev-parse", "--verify", "--quiet", &commit])
}

fn has_commit(root: &Path, reference: &str) -> Result<bool> {
    Ok(commit_id(root, reference)?.is_some())
}

/// True when `root` is the top of a linked worktree, not a main checkout.
fn linked_root(root: &Path) -> Result<bool> {
    let Some(top) = answer(root, &["rev-parse", "--show-toplevel"])? else {
        return Ok(false);
    };
    let git_dir = answer(root, &["rev-parse", "--absolute-git-dir"])?;
    let common = answer(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    Ok(fs::canonicalize(top)? == fs::canonicalize(root)? && git_dir.is_some() && git_dir != common)
}

/// `name` in the Git directory of the checkout at `root` (`X1`).
fn git_path(root: &Path, name: &str) -> Result<PathBuf> {
    let path =
        answer(root, &["rev-parse", "--git-path", name])?.context("No git repository found")?;
    Ok(root.join(path))
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(value)? + "\n")?;
    Ok(())
}

fn random() -> u64 {
    std::collections::hash_map::RandomState::new().hash_one(Instant::now())
}

/// Creates `<root>/<4 hex digits>` and its marker for one new worktree (`ETe`, `GTe`); a
/// taken name gets a random 4-digit suffix, at most five times.
fn reserve(root: &Path, id: &str) -> Result<PathBuf> {
    let mut dir = root.join(id);
    let mut attempt = 0;
    while let Err(error) = fs::create_dir(&dir) {
        if attempt == 5 || fs::symlink_metadata(&dir).is_err() {
            return Err(error.into());
        }
        attempt += 1;
        dir = root.join(format!("{id}-{:04}", random() % 10_000));
    }
    if let Err(error) = fs::write(dir.join(MARKER), "") {
        let _ = fs::remove_dir_all(&dir);
        return Err(error.into());
    }
    Ok(dir)
}

/// Remotes, `origin` first (`g0`).
fn remotes(root: &Path) -> Result<Vec<String>> {
    let mut remotes: Vec<String> = git(root, &["remote"])?
        .stdout
        .lines()
        .map(str::trim)
        .filter(|remote| !remote.is_empty())
        .map(str::to_owned)
        .collect();
    remotes.sort_by_key(|remote| remote != "origin");
    Ok(remotes)
}

/// `(remote, branch)` of `<remote>/<branch>`, by the first remote that prefixes it.
fn split_remote<'a>(name: &'a str, remotes: &'a [String]) -> Option<(&'a str, &'a str)> {
    remotes.iter().find_map(|remote| {
        let branch = name.strip_prefix(remote.as_str())?.strip_prefix('/')?;
        Some((remote.as_str(), branch))
    })
}

/// The remote default branch name (`h2`): for each remote, its `HEAD`, else what
/// `git remote show` reports, else `main` or `master`.
fn default_branch(root: &Path) -> Result<Option<String>> {
    for remote in remotes(root)? {
        let prefix = format!("refs/remotes/{remote}/");
        let head = answer(root, &["symbolic-ref", "--quiet", &format!("{prefix}HEAD")])?;
        if let Some(branch) = head.and_then(|head| Some(head.strip_prefix(&prefix)?.to_owned())) {
            return Ok(Some(branch));
        }
        let shown = bounded(&mut command(root, &["remote", "show", &remote]))?;
        let branch = shown.and_then(|text| {
            let line = text
                .lines()
                .find_map(|l| l.trim().strip_prefix("HEAD branch:"))?;
            Some(line.trim().to_owned())
        });
        if let Some(branch) = branch.filter(|b| !b.is_empty() && b != "(unknown)") {
            return Ok(Some(branch));
        }
        for branch in ["main", "master"] {
            let reference = format!("{prefix}{branch}");
            if git(root, &["show-ref", "--verify", "--quiet", &reference])?.success {
                return Ok(Some(branch.to_owned()));
            }
        }
    }
    Ok(None)
}

/// Stdout of `command` when it succeeds within the ten seconds the original gives network
/// and login-shell probes.
fn bounded(command: &mut Command) -> Result<Option<String>> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("run local command")?;
    let mut stdout = child.stdout.take().context("command stdout")?;
    let reader = std::thread::spawn(move || {
        let mut text = Vec::new();
        stdout.read_to_end(&mut text).map(|_| text)
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let text = reader
        .join()
        .map_err(|_| anyhow!("command reader panicked"))??;
    Ok(status
        .success()
        .then(|| String::from_utf8_lossy(&text).into_owned()))
}

/// What `git worktree add` starts from for branch `name` (`vwe`, `x0`): the local branch, or
/// its upstream when only the upstream moved; else a remote-tracking branch, fetched when
/// only the remote has it; else `name` as a commit.
fn starting_commit(root: &Path, name: &str) -> Result<String> {
    if name == "HEAD" || name == "@" {
        return Ok(name.to_owned());
    }
    let local = if name.starts_with("refs/heads/") {
        Some(name.to_owned())
    } else if name.starts_with("refs/") {
        None
    } else {
        Some(format!("refs/heads/{name}"))
    };
    let upstream = format!("{name}@{{u}}");
    let upstream = answer(
        root,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            &upstream,
        ],
    )?;
    if let Some(upstream) = upstream {
        let remote = format!("refs/remotes/{upstream}");
        let target = if has_commit(root, &remote)? {
            remote
        } else if upstream.starts_with("refs/") {
            upstream
        } else {
            format!("refs/heads/{upstream}")
        };
        let base = local.unwrap_or_else(|| name.to_owned());
        let range = format!("{base}...{target}");
        let counts = answer(root, &["rev-list", "--left-right", "--count", &range])?;
        let counts = counts.as_deref().and_then(|counts| {
            let mut counts = counts.split_whitespace().map(str::parse::<u64>);
            Some((counts.next()?.ok()?, counts.next()?.ok()?))
        });
        let only_upstream_moved = counts.is_some_and(|(ahead, behind)| ahead == 0 && behind > 0);
        return Ok(if only_upstream_moved { target } else { base });
    }
    if let Some(local) = local
        && has_commit(root, &local)?
    {
        return Ok(local);
    }
    let remotes = remotes(root)?;
    let tracking = name.starts_with("refs/remotes/");
    if tracking && has_commit(root, name)? {
        return Ok(name.to_owned());
    }
    let short = name.strip_prefix("refs/remotes/").unwrap_or(name);
    let candidates: Vec<(&str, &str)> = if name.starts_with("refs/") && !tracking {
        Vec::new()
    } else if let Some(pair) = split_remote(short, &remotes) {
        vec![pair]
    } else if tracking {
        Vec::new()
    } else {
        remotes
            .iter()
            .map(|remote| (remote.as_str(), name))
            .collect()
    };
    for (remote, branch) in &candidates {
        let reference = format!("refs/remotes/{remote}/{branch}");
        if has_commit(root, &reference)? {
            return Ok(reference);
        }
    }
    let mut unverified = false;
    for (remote, branch) in &candidates {
        let reference = format!("refs/remotes/{remote}/{branch}");
        let head = format!("refs/heads/{branch}");
        let listed = run(root, &["ls-remote", "--exit-code", remote, &head])?;
        match listed.status.code() {
            Some(0) => {
                let refspec = format!("+{head}:{reference}");
                let fetched = git(root, &["fetch", remote, &refspec])?;
                ensure!(
                    fetched.success && has_commit(root, &reference)?,
                    "Failed to fetch the worktree starting ref"
                );
                return Ok(reference);
            }
            Some(2) => {}
            _ => unverified = true,
        }
    }
    if let Some(sha) = commit_id(root, name)? {
        return Ok(sha);
    }
    ensure!(!unverified, "Failed to validate the worktree starting ref");
    Ok(name.to_owned())
}

/// `git worktree add --detach` without clean or smudge filters and tree attributes (`STe`,
/// `z0`), then a local branch tracking a remote-tracking start that has none (`CTe`).
fn add_worktree(source: &Path, path: &Path, start: &str) -> Result<()> {
    let found = run(
        source,
        &[
            "config",
            "--name-only",
            "--get-regexp",
            r"^filter\..*\.(clean|smudge|process|required)$",
        ],
    )?;
    ensure!(
        found.status.success() || found.status.code() == Some(1),
        "git config filter discovery failed: {}",
        String::from_utf8_lossy(&found.stderr).trim()
    );
    // `--get-regexp` already matched the four filter keys.
    let filters: std::collections::BTreeSet<String> = String::from_utf8_lossy(&found.stdout)
        .lines()
        .filter_map(|line| {
            Some(
                line.trim()
                    .strip_prefix("filter.")?
                    .rsplit_once('.')?
                    .0
                    .to_owned(),
            )
        })
        .collect();
    let mut args = vec![
        "-c".to_owned(),
        "attr.tree=".to_owned(),
        "-c".to_owned(),
        "core.attributesFile=".to_owned(),
    ];
    for name in &filters {
        for setting in ["clean=", "smudge=", "process=", "required=false"] {
            args.extend(["-c".to_owned(), format!("filter.{name}.{setting}")]);
        }
    }
    let path_text = path.to_str().context("worktree path must be UTF-8")?;
    args.extend(["worktree", "add", "--detach", path_text, start].map(str::to_owned));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let added = git(source, &args)?;
    ensure!(
        added.success,
        "git worktree add failed: {}",
        added.failure()
    );
    let Some(rest) = start.strip_prefix("refs/remotes/") else {
        return Ok(());
    };
    let remotes = remotes(source)?;
    let Some((_, branch)) = split_remote(rest, &remotes) else {
        return Ok(());
    };
    if branch.is_empty() || branch == "HEAD" || has_commit(source, &format!("refs/heads/{branch}"))?
    {
        return Ok(());
    }
    let tracked = git(source, &["branch", "--track", branch, start])?;
    if !tracked.success {
        let _ = git(source, &["worktree", "remove", path_text]);
        bail!("{}", tracked.failure());
    }
    Ok(())
}

/// Everything after `git worktree add` (`D2`): the synced branch, the copied ignored files,
/// the environment selection, and its setup script with the variables it exports.
fn prepare(
    source: &Path,
    cwd: &Path,
    worktree: &Worktree,
    name: &str,
    start: &str,
    default: Option<&str>,
    local_env_path: Option<&Path>,
) -> Result<()> {
    let root = &worktree.git_root;
    if let Some(branch) = synced_branch(source, name, start, default)? {
        let branch = format!("refs/heads/{branch}");
        let tree = answer(source, &["rev-parse", &format!("{branch}^{{tree}}")])?
            .context("Failed to resolve tree")?;
        write_json(
            &git_path(root, "codex-synced-branch.json")?,
            &SyncedBranch {
                branch: &branch,
                last_synced_tree_ref: &tree,
            },
        )?;
    }
    copy_ignored(source, root)?;
    let selected = local_env_path.and_then(|path| {
        select_environment(source, root, path).unwrap_or_else(|_| Some(path.to_owned()))
    });
    let stored = selected.as_deref().map(Path::to_string_lossy);
    // The original only logs a failed store.
    let _ = store_environment(root, stored.as_deref().unwrap_or(NO_ENVIRONMENT));
    let Some(config) = selected else {
        return Ok(());
    };
    let environment = environment(&config)?;
    let script = environment.setup.current();
    let exported = if script.is_empty() {
        None
    } else {
        let injected = [
            ("CODEX_SOURCE_TREE_PATH", cwd),
            ("CODEX_WORKTREE_PATH", worktree.workspace_root.as_path()),
        ];
        run_script(
            &root.join(script_dir(&config)),
            script,
            "Setup script",
            &injected,
        )?
    };
    // The original only logs a failed store.
    let _ = git_path(root, SHELL_ENVIRONMENT).and_then(|path| match exported {
        Some(exported) => write_json(&path, &exported),
        None => match fs::remove_file(&path) {
            Err(error) if error.kind() != ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        },
    });
    Ok(())
}

/// The branch a later sync compares the worktree with (`WTe`): kept only when it is not the
/// remote default branch.
fn synced_branch(
    source: &Path,
    name: &str,
    start: &str,
    default: Option<&str>,
) -> Result<Option<String>> {
    let branch = if name == "HEAD" || name == "@" {
        current_branch(source)?
    } else {
        let candidate = if let Some(branch) = start.strip_prefix("refs/heads/") {
            Some(branch.to_owned())
        } else if let Some(rest) = start.strip_prefix("refs/remotes/") {
            let remotes = remotes(source)?;
            split_remote(rest, &remotes)
                .map(|(_, branch)| branch.to_owned())
                .filter(|branch| !branch.is_empty() && branch != "HEAD")
        } else {
            None
        };
        match candidate {
            Some(branch) if has_commit(source, &format!("refs/heads/{branch}"))? => Some(branch),
            _ => None,
        }
    };
    Ok(branch.filter(|branch| Some(branch.as_str()) != default))
}

fn plain(root: &Path, args: &[&str]) -> Result<Output> {
    plain_command(root)
        .args(args)
        .output()
        .context("run local Git")
}

/// `git ls-files --others --ignored -z`.
fn ignored(root: &Path, args: &[&str]) -> Result<Vec<String>> {
    let listing = ["ls-files", "--others", "--ignored", "-z"];
    let output = plain(root, &[&listing[..], args].concat())?;
    Ok(checked(output)?
        .split(|byte| *byte == 0)
        .filter_map(|path| std::str::from_utf8(path).ok())
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect())
}

/// The ignored files the original copies into a new local worktree (`BTe`, `UTe`): every
/// `AGENTS.override.md`, and the ignored files `.worktreeinclude` names.
fn copy_ignored(source: &Path, worktree: &Path) -> Result<()> {
    let overrides = ignored(
        source,
        &["--exclude-standard", "--", &format!(":(glob)**/{OVERRIDE}")],
    )?;
    for path in overrides
        .iter()
        .filter(|path| *path == OVERRIDE || path.ends_with(&format!("/{OVERRIDE}")))
    {
        copy_new(source, worktree, path, OVERRIDE)?;
    }
    match fs::metadata(source.join(".worktreeinclude")) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        other => other?,
    };
    let standard: HashSet<String> = ignored(source, &["--exclude-standard"])?
        .into_iter()
        .collect();
    for path in ignored(source, &["--exclude-from=.worktreeinclude"])? {
        if standard.contains(&path) {
            copy_new(source, worktree, &path, ".worktreeinclude")?;
        }
    }
    Ok(())
}

/// Copies regular file `relative` from `source` to the same place in `worktree` (`k2`, `j2`),
/// never through a symlinked worktree directory and never over an existing file.
fn copy_new(source: &Path, worktree: &Path, relative: &str, label: &str) -> Result<()> {
    let relative = Path::new(relative);
    ensure!(
        relative
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "Cannot copy {label} from outside the workspace"
    );
    let from = source.join(relative);
    let metadata = match fs::symlink_metadata(&from) {
        Ok(metadata) if metadata.is_file() => metadata,
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(error.into()),
        _ => return Ok(()),
    };
    let to = worktree.join(relative);
    let mut dir = worktree.to_owned();
    for part in relative.parent().into_iter().flat_map(Path::components) {
        dir.push(part);
        match fs::symlink_metadata(&dir) {
            Ok(metadata) => ensure!(
                !metadata.file_type().is_symlink(),
                "Cannot copy {label} through a symlinked workspace path"
            ),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                fs::create_dir_all(to.parent().unwrap_or(worktree))?;
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&to)
    {
        Err(error) if error.kind() == ErrorKind::AlreadyExists => return Ok(()),
        file => file?,
    };
    std::io::copy(&mut fs::File::open(&from)?, &mut file)?;
    file.set_permissions(metadata.permissions())?;
    Ok(())
}

/// The environment file the worktree uses (`VTe`): its copy in the worktree, copied there
/// when the source has it untracked; `None` when it is tracked but absent at the start.
fn select_environment(source: &Path, worktree: &Path, path: &Path) -> Result<Option<PathBuf>> {
    // Git reports the repository root with symlinks resolved (`/private/tmp` for `/tmp`), while
    // the selected path may be spelled through one: compare the resolved spellings too.
    let resolved = path
        .parent()
        .zip(path.file_name())
        .and_then(|(dir, name)| Some(fs::canonicalize(dir).ok()?.join(name)));
    let inside = path.strip_prefix(source).ok().or_else(|| {
        resolved
            .as_deref()?
            .strip_prefix(fs::canonicalize(source).ok()?)
            .ok()
    });
    let Some(relative) = inside.map(Path::to_owned).filter(|relative| {
        relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    }) else {
        return Ok(Some(path.to_owned()));
    };
    let relative = relative.as_path();
    let target = worktree.join(relative);
    let regular = |target: &Path| -> Result<Option<PathBuf>> {
        ensure!(
            fs::symlink_metadata(target)?.is_file(),
            "Selected local environment configuration is not a regular file"
        );
        Ok(Some(target.to_owned()))
    };
    let relative_text = relative
        .to_str()
        .context("environment path must be UTF-8")?;
    match fs::symlink_metadata(&target) {
        Ok(_) => {
            let mut dir = worktree.to_owned();
            for part in relative.parent().into_iter().flat_map(Path::components) {
                dir.push(part);
                ensure!(
                    !fs::symlink_metadata(&dir)?.file_type().is_symlink(),
                    "Cannot copy selected local environment through a symlinked workspace path"
                );
            }
            return regular(&target);
        }
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(error.into()),
        Err(_) => {}
    }
    let tracked = run(
        source,
        &["ls-files", "--error-unmatch", "--", relative_text],
    )?;
    match tracked.status.code() {
        Some(0) => return Ok(None),
        Some(1) => {}
        _ => bail!("Failed to check selected environment tracking"),
    }
    let ignored = plain(source, &["check-ignore", "--quiet", "--", relative_text])?;
    match ignored.status.code() {
        Some(0) => {
            let there = plain(worktree, &["check-ignore", "--quiet", "--", relative_text])?;
            if there.status.code() != Some(0) {
                return Ok(Some(path.to_owned()));
            }
        }
        Some(1) => {}
        _ => return Ok(Some(path.to_owned())),
    }
    copy_new(
        source,
        worktree,
        relative_text,
        "selected local environment",
    )?;
    regular(&target)
}

/// Stores the selection in the worktree's own Git config (`DEe`), enabling
/// `extensions.worktreeConfig` when Git requires it, like the original.
fn store_environment(worktree: &Path, value: &str) -> Result<()> {
    let args = [
        "config",
        "--worktree",
        "codex.localEnvironmentConfigPath",
        value,
    ];
    let stored = git(worktree, &args)?;
    if stored.success {
        return Ok(());
    }
    ensure!(
        stored.stderr.to_lowercase().contains("worktreeconfig"),
        "{}",
        stored.failure()
    );
    checked(run(
        worktree,
        &["config", "extensions.worktreeConfig", "true"],
    )?)?;
    checked(run(worktree, &args)?)?;
    Ok(())
}

/// Reads an environment file (`l$`).
fn environment(path: &Path) -> Result<Environment> {
    let shown = path.display();
    let table: toml::Table = fs::read_to_string(path)
        .map_err(anyhow::Error::from)
        .and_then(|text| Ok(text.parse()?))
        .map_err(|e| anyhow!("Failed to parse environment config at {shown}: {e}"))?;
    let environment: Environment = toml::Value::Table(table)
        .try_into()
        .map_err(|e| anyhow!("Invalid environment config at {shown}: {e}"))?;
    ensure!(
        environment.version.unwrap_or(1) >= 1,
        "Invalid environment config at {shown}: version must be at least 1"
    );
    Ok(environment)
}

/// The directory a script runs in, relative to its Git root (`cCe`, `lCe`): the project that
/// holds `.codex/environments/<file>`.
fn script_dir(config: &Path) -> PathBuf {
    let marked = |dir: &Path| dir.join(".codex").exists() || dir.join(".git").exists();
    let dir = config.parent().unwrap_or(config);
    let codex = dir.parent().filter(|codex| {
        dir.file_name() == Some("environments".as_ref())
            && codex.file_name() == Some(".codex".as_ref())
    });
    let project = match codex {
        Some(_) if marked(dir) => dir,
        Some(codex) if marked(codex) => codex,
        Some(codex) => codex.parent().unwrap_or(codex),
        None => dir.ancestors().find(|dir| marked(dir)).unwrap_or(config),
    };
    let root = project
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(project);
    project
        .strip_prefix(root)
        .unwrap_or(Path::new(""))
        .to_owned()
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

/// Runs a local-environment script in `dir` with the user's login environment (`T0`);
/// returns the variables a setup script exported (`kwe`).
fn run_script(
    dir: &Path,
    script: &str,
    kind: &str,
    injected: &[(&str, &Path)],
) -> Result<Option<ShellEnvironment>> {
    let scratch = tempfile::tempdir()?;
    let file = scratch.path().join("setup_script.sh");
    let wrapper = scratch.path().join("setup_wrapper.sh");
    let before = scratch.path().join("before_env.txt");
    let after = scratch.path().join("captured_env.txt");
    fs::write(&file, script.replace("\r\n", "\n"))?;
    fs::write(
        &wrapper,
        [
            "set -xeo pipefail".to_owned(),
            format!("capture_path={}", shell_quote(&after)),
            format!("before_capture_path={}", shell_quote(&before)),
            r#"env > "$before_capture_path""#.to_owned(),
            r#"trap 'code=$?; if [ "$code" -eq 0 ]; then env > "$capture_path"; fi' EXIT"#
                .to_owned(),
            format!(". {}", shell_quote(&file)),
        ]
        .join("\n"),
    )?;
    let env = login_env(dir);
    let status = Command::new("bash")
        .arg(&wrapper)
        .current_dir(dir)
        .env_clear()
        .envs(&env)
        .envs([
            ("COLORTERM", "truecolor"),
            ("FORCE_COLOR", "1"),
            ("TERM", "xterm-256color"),
        ])
        .envs(injected.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| anyhow!("{kind} failed: {e}"))?;
    match status.code() {
        Some(0) => {}
        Some(code) => bail!("{kind} exited with code {code}"),
        None => bail!("{kind} exited with {status}"),
    }
    let read = |path: &Path| {
        fs::read_to_string(path).map(|text| parse_env(text.trim_start_matches('\u{feff}').lines()))
    };
    Ok(match (read(&before), read(&after)) {
        (Ok(before), Ok(after)) => exported(&before, &after),
        _ => None,
    })
}

/// `env` output as variables (`E0`).
fn parse_env<'a>(lines: impl Iterator<Item = &'a str>) -> HashMap<String, String> {
    lines
        .filter_map(|line| line.split_once('='))
        .filter(|(key, _)| !key.is_empty())
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

/// The variables a script set or removed (`kwe`), without shell bookkeeping.
fn exported(
    before: &HashMap<String, String>,
    after: &HashMap<String, String>,
) -> Option<ShellEnvironment> {
    let multiline =
        |value: Option<&String>| value.is_some_and(|v| v.contains('\n') || v.contains('\r'));
    let mut set = BTreeMap::new();
    let mut exclude = Vec::new();
    let keys: HashSet<&String> = before.keys().chain(after.keys()).collect();
    for key in keys {
        if UNSHARED.contains(&key.as_str())
            || key.starts_with("BASH_FUNC_")
            || multiline(before.get(key))
            || multiline(after.get(key))
        {
            continue;
        }
        match after.get(key) {
            None => exclude.push(key.clone()),
            Some(value) if before.get(key) != Some(value) => {
                set.insert(key.clone(), value.clone());
            }
            Some(_) => {}
        }
    }
    exclude.sort();
    (!set.is_empty() || !exclude.is_empty()).then_some(ShellEnvironment {
        version: 1,
        set,
        exclude,
    })
}

/// The user's login-shell environment over this process's (`bxe`), which a launch from the
/// Dock lacks; this process's alone when no shell answers.
fn login_env(dir: &Path) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = std::env::vars().collect();
    let user = std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_owned());
    let mut shells = vec![user.as_str()];
    shells.extend(["/bin/zsh", "/bin/bash"].into_iter().filter(|s| *s != user));
    for shell in shells {
        let mut probe = Command::new(shell);
        probe.args(["-ilc", LOGIN_ENV]).current_dir(dir).envs([
            ("CODEX_SHELL", "1"),
            ("DISABLE_AUTO_UPDATE", "true"),
            ("ZSH_TMUX_AUTOSTARTED", "true"),
            ("ZSH_TMUX_AUTOSTART", "false"),
        ]);
        let Ok(Some(output)) = bounded(&mut probe) else {
            continue;
        };
        let mut parts = output.split("\0_SHELL_ENV_DELIMITER_\0");
        if let (Some(_), Some(vars), Some(_)) = (parts.next(), parts.next(), parts.next()) {
            env.extend(parse_env(vars.split('\0')));
            env.remove("CODEX_SHELL");
            return env;
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unstarted_removal_refuses_foreign_and_owned_worktrees() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("repo");
        let init = Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .arg(&source)
            .output()?;
        checked(init)?;
        let commit = [
            "-c",
            "user.name=C",
            "-c",
            "user.email=c@example.com",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "base",
        ];
        checked(run(&source, &commit)?)?;
        let root = dir.path().join("worktrees");
        let worktree = create_worktree(&root, &source, None, None)?.context("repository")?;
        let elsewhere = dir.path().join("elsewhere");
        fs::create_dir(&elsewhere)?;
        assert!(remove_unstarted_worktree(&elsewhere, &worktree).is_err());
        // The main checkout is refused even inside a marked reservation.
        fs::write(dir.path().join(MARKER), "")?;
        let main = Worktree {
            git_root: source.clone(),
            workspace_root: source.clone(),
        };
        let parent = dir.path().parent().context("temporary parent")?;
        assert!(remove_unstarted_worktree(parent, &main).is_err());
        set_owner_thread(&worktree.git_root, "thread-1")?;
        assert!(remove_unstarted_worktree(&root, &worktree).is_err());
        assert!(worktree.git_root.join(".git").is_file());
        fs::remove_file(git_path(&worktree.git_root, OWNER)?)?;
        remove_unstarted_worktree(&root, &worktree)?;
        assert!(!worktree.git_root.parent().unwrap().exists());
        assert!(source.join(".git").is_dir());
        Ok(())
    }

    #[test]
    fn a_taken_reservation_name_gets_a_suffix_and_stays_untouched() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::create_dir(dir.path().join("abcd"))?;
        let reserved = reserve(dir.path(), "abcd")?;
        let name = reserved
            .file_name()
            .and_then(|n| n.to_str())
            .context("name")?;
        let suffix = name.strip_prefix("abcd-").context("collision suffix")?;
        assert!(suffix.len() == 4 && suffix.bytes().all(|b| b.is_ascii_digit()));
        assert!(reserved.join(MARKER).is_file());
        assert_eq!(fs::read_dir(dir.path().join("abcd"))?.count(), 0);
        Ok(())
    }
}
