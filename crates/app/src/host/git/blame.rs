use super::*;

/// `blame-file`: `git blame --porcelain` for the file at `path` (absolute, or relative to `cwd`)
/// in its working tree, one entry per line.
pub(super) fn read(root: &Path, params: &Value) -> Result<Value> {
    let cwd = Path::new(params["cwd"].as_str().context("missing Git cwd")?);
    let path = cwd.join(params["path"].as_str().context("missing Git path")?);
    let file = fs::canonicalize(&path).context("blame file")?;
    let relative = file
        .strip_prefix(fs::canonicalize(root)?)
        .context("blame file is outside the repository")?
        .to_str()
        .context("blame path must be UTF-8")?;
    let output = command(root, &["blame", "--porcelain", "--", relative])
        .env("GIT_NO_LAZY_FETCH", "1")
        .output()
        .context("run local Git")?;
    let stdout = checked(output)?;
    Ok(json!({ "lines": parse(&String::from_utf8_lossy(&stdout)) }))
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Line {
    line_number: u32,
    commit_sha: String,
    author: Option<String>,
    author_time: Option<i64>,
    summary: Option<String>,
}

/// The original's porcelain reader: a header `<sha> <orig> <final> [<count>]` opens each line,
/// commit fields follow only the commit's first appearance, and a tab line closes it.
fn parse(porcelain: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut commits: HashMap<String, Line> = HashMap::new();
    let mut current: Option<Line> = None;
    for line in porcelain.split('\n').filter(|line| !line.is_empty()) {
        if line.starts_with('\t') {
            if let Some(entry) = current.take() {
                commits.insert(entry.commit_sha.clone(), entry.clone());
                lines.push(entry);
            }
            continue;
        }
        let Some(entry) = &mut current else {
            let mut fields = line.split(' ');
            let sha = fields.next().unwrap_or_default();
            current = Some(Line {
                line_number: fields.nth(1).and_then(|n| n.parse().ok()).unwrap_or(0),
                commit_sha: sha.to_owned(),
                ..commits.get(sha).cloned().unwrap_or_default()
            });
            continue;
        };
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        let value = (!value.is_empty()).then(|| value.to_owned());
        match key {
            "author" => entry.author = value,
            "author-time" => entry.author_time = value.and_then(|v| v.parse().ok()),
            "summary" => entry.summary = value,
            _ => {}
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_commits_reuse_their_first_fields() {
        let sha = "a".repeat(40);
        let zero = "0".repeat(40);
        let porcelain = format!(
            "{sha} 1 1 2\nauthor Ada\nauthor-mail <ada@example.com>\nauthor-time 1700000000\n\
             summary Add math\nfilename math.py\n\tdef f():\n{sha} 2 2\n\treturn 1\n\
             {zero} 3 3 1\nauthor Not Committed Yet\nauthor-time 1800000000\nsummary Version of math.py from math.py\n\
             filename math.py\n\tnew\n"
        );
        let lines = serde_json::to_value(parse(&porcelain)).unwrap();
        assert_eq!(lines.as_array().unwrap().len(), 3);
        assert_eq!(
            lines[1],
            json!({"commitSha": sha, "lineNumber": 2, "author": "Ada", "authorTime": 1700000000, "summary": "Add math"})
        );
        assert_eq!(lines[2]["commitSha"], zero);
        assert_eq!(lines[2]["lineNumber"], 3);
    }

    #[test]
    fn blames_working_tree_lines_inside_the_repository_only() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        checked(run(root, &["init"]).unwrap()).unwrap();
        fs::write(root.join("a.txt"), "one\n").unwrap();
        checked(run(root, &["add", "."]).unwrap()).unwrap();
        checked(
            run(
                root,
                &[
                    "-c",
                    "user.name=Ada",
                    "-c",
                    "user.email=ada@example.com",
                    "commit",
                    "-m",
                    "base",
                ],
            )
            .unwrap(),
        )
        .unwrap();
        fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        let body = read(root, &json!({"cwd": root, "path": "a.txt"})).unwrap();
        assert_eq!(body["lines"][0]["author"], "Ada");
        assert_eq!(body["lines"][0]["summary"], "base");
        assert!(
            body["lines"][1]["commitSha"]
                .as_str()
                .unwrap()
                .bytes()
                .all(|b| b == b'0')
        );
        let outside = tempfile::NamedTempFile::new().unwrap();
        assert!(read(root, &json!({"cwd": root, "path": outside.path()})).is_err());
    }
}
