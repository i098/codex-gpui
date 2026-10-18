use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::{Component, Path};
use std::process::{Command, Output, Stdio};

fn command(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .arg("--literal-pathspecs")
        .args(["-c", "core.quotePath=false"])
        .args(args)
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0");
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

/// `binary` adds the data a reverse patch needs to restore a binary file.
fn diffs(root: &Path, staged: bool, binary: bool) -> Result<Vec<Value>> {
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
            let mut args = vec![
                "diff",
                "--no-index",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
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
            files.push(json!({"path": path, "diff": String::from_utf8_lossy(&output.stdout)}));
        }
    }
    Ok(files)
}

/// Reverse-applies the diff the caller displayed, to the index (`cached`) or the working tree.
/// Git applies a patch atomically, and refuses it when the target no longer matches.
fn apply(root: &Path, patch: &str, cached: bool) -> Result<()> {
    let mut args = vec!["apply", "--reverse"];
    if cached {
        args.push("--cached");
    }
    let mut child = command(root, &args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("run local Git")?;
    let sent = child
        .stdin
        .take()
        .context("Git stdin")?
        .write_all(patch.as_bytes());
    checked(child.wait_with_output().context("run local Git")?)?;
    sent.context("send patch to Git")
}

/// The header of `diff`, without mode changes, with only its `index`th hunk: a hunk revert
/// must not change the file's permissions.
fn hunk(diff: &str, index: usize) -> Result<String> {
    let lines = diff.split_inclusive('\n').collect::<Vec<_>>();
    let starts = (0..lines.len())
        .filter(|&i| lines[i].starts_with("@@ "))
        .collect::<Vec<_>>();
    let start = *starts
        .get(index)
        .context("hunk is no longer in the selected diff")?;
    let end = starts.get(index + 1).copied().unwrap_or(lines.len());
    let header = lines[..starts[0]]
        .iter()
        .filter(|line| !line.starts_with("old mode ") && !line.starts_with("new mode "));
    Ok(header.chain(&lines[start..end]).copied().collect())
}

/// Reverts one file, or one hunk of it, in the `source` the caller displayed. The original
/// reverts a staged change in the index and then in the working tree: when only the second
/// step fails, or the working file already differs from the index so that Git could move the
/// patch onto another matching block, the result is `partial-success`.
fn revert(root: &Path, path: &str, params: &Value) -> Result<&'static str> {
    let staged = match params["source"].as_str() {
        Some("staged") => true,
        Some("unstaged") => false,
        _ => bail!("Git revert source must be staged or unstaged"),
    };
    let shown = params["diff"].as_str().context("missing Git diff")?;
    let current = |binary| -> Result<String> {
        diffs(root, staged, binary)?
            .into_iter()
            .find(|file| file["path"] == path)
            .and_then(|file| file["diff"].as_str().map(str::to_owned))
            .context("file is no longer in the selected changes")
    };
    ensure!(
        current(false)? == shown,
        "file changed since its diff was shown; refresh and try again"
    );
    let patch = match params.get("hunk") {
        None | Some(Value::Null) => current(true)?,
        Some(index) => hunk(
            &current(true)?,
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
        apply(root, &patch, true)?;
        if !differs.status.success() {
            return Ok("partial-success");
        }
    }
    match apply(root, &patch, false) {
        Ok(()) => Ok("success"),
        Err(_) if staged => Ok("partial-success"),
        Err(error) => Err(error),
    }
}

pub(super) fn fetch(endpoint: &str, params: &Value) -> Result<Value> {
    let cwd = params["cwd"].as_str().context("missing Git cwd")?;
    ensure!(Path::new(cwd).is_absolute(), "Git cwd must be absolute");
    let root = checked(run(Path::new(cwd), &["rev-parse", "--show-toplevel"])?)?;
    let root = String::from_utf8(root).context("Git root must be UTF-8")?;
    let root = Path::new(root.strip_suffix('\n').unwrap_or(&root));
    let mut status = None;
    if endpoint != "git-changes" {
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
    let mut body = json!({"root": root, "unstaged": diffs(root, false, false)?, "staged": diffs(root, true, false)?});
    if let Some(status) = status {
        body["status"] = status.into();
    }
    Ok(body)
}
