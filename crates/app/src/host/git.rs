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
