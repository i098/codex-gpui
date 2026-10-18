use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};
use std::path::{Component, Path};
use std::process::{Command, Output};

fn run(root: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .arg("--literal-pathspecs")
        .args(["-c", "core.quotePath=false"])
        .args(args)
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .context("run local Git")
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

fn diffs(root: &Path, staged: bool) -> Result<Vec<Value>> {
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
    ];
    if staged {
        args.push("--cached");
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
        let patch = String::from_utf8_lossy(&output[separator + 2..]);
        let patches = patch
            .strip_prefix("diff --git ")
            .context("unsupported Git diff format")?
            .split("\ndiff --git ");
        for patch in patches {
            records.next().context("missing Git diff metadata")?;
            let path = records.next().context("missing Git diff path")?;
            let mut diff = format!("diff --git {patch}");
            if !diff.ends_with('\n') {
                diff.push('\n');
            }
            files.push(json!({"path": path, "diff": diff}));
        }
        ensure!(
            records.next().is_none(),
            "Git diff paths and patches differ"
        );
    }
    if !staged {
        let untracked = checked(run(
            root,
            &["ls-files", "--others", "--exclude-standard", "-z"],
        )?)?;
        for path in std::str::from_utf8(&untracked)
            .context("Git paths must be UTF-8")?
            .split('\0')
            .filter(|path| !path.is_empty())
        {
            let output = run(
                root,
                &[
                    "diff",
                    "--no-index",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-color",
                    "--",
                    "/dev/null",
                    path,
                ],
            )?;
            ensure!(
                output.status.success() || output.status.code() == Some(1),
                "Git: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            files.push(json!({"path": path, "diff": String::from_utf8_lossy(&output.stdout)}));
        }
    }
    Ok(files)
}

pub(super) fn fetch(endpoint: &str, params: &Value) -> Result<Value> {
    let cwd = params["cwd"].as_str().context("missing Git cwd")?;
    ensure!(Path::new(cwd).is_absolute(), "Git cwd must be absolute");
    let root = checked(run(Path::new(cwd), &["rev-parse", "--show-toplevel"])?)?;
    let root = String::from_utf8(root).context("Git root must be UTF-8")?;
    let root = Path::new(root.strip_suffix('\n').unwrap_or(&root));
    if let Some(result) = commit_flow(endpoint, root, params) {
        return result;
    }
    if endpoint != "git-changes" {
        let path = params["path"].as_str().context("missing Git path")?;
        ensure!(
            !path.is_empty()
                && Path::new(path)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
            "Git path must be repository-relative"
        );
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
    Ok(json!({"root": root, "unstaged": diffs(root, false)?, "staged": diffs(root, true)?}))
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
        _ => return None,
    })
}

fn current_branch(root: &Path) -> Result<Option<String>> {
    let head = git(root, &["symbolic-ref", "--short", "-q", "HEAD"])?;
    Ok(head.success.then(|| head.stdout.trim().to_owned()))
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
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
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
