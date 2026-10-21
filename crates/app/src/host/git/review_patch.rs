use super::*;

pub(super) fn read(root: &Path, params: &Value) -> Result<Value> {
    let source = params["reviewSource"]
        .as_str()
        .context("missing review source")?;
    let head = if source == "uncommitted" {
        commit_id(root, "HEAD")?
    } else {
        None
    };
    let before = params["beforeRef"].as_str();
    let after = params["afterRef"].as_str();
    let needs_empty =
        source == "uncommitted" && head.is_none() || source == "commits" && before.is_none();
    let empty = if needs_empty {
        String::from_utf8(checked(run(
            root,
            &["hash-object", "-t", "tree", "/dev/null"],
        )?)?)?
    } else {
        String::new()
    };
    let (revisions, count) = match source {
        "unstaged" => (["", ""], 0),
        "staged" => (["--cached", ""], 1),
        "uncommitted" => ([head.as_deref().unwrap_or(empty.trim()), ""], 1),
        "branch" | "commits" => {
            let after = after.context("missing review target")?;
            let before = before.unwrap_or(empty.trim());
            for revision in [before, after] {
                ensure!(
                    matches!(revision.len(), 40 | 64)
                        && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
                    "invalid review object ID"
                );
            }
            ([before, after], 2)
        }
        _ => bail!("this review source cannot be exported"),
    };
    let mut untracked = if matches!(source, "unstaged" | "uncommitted") {
        untracked_paths(root)?
    } else {
        Vec::new()
    };
    if untracked.len() > MAX_REVIEW_UNTRACKED_FILES {
        untracked.clear();
    }
    let files = review_diffs(root, &revisions[..count], true, &untracked, false)?;
    let mut patch = Vec::with_capacity(files.iter().map(|file| file.diff.len()).sum());
    for file in files {
        patch.extend(file.diff);
    }
    Ok(json!({"diff": String::from_utf8(patch).context("Git patch must be UTF-8")?}))
}
