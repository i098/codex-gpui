use super::*;
use url::Url;

pub(super) fn read(root: &Path, params: &Value) -> Result<Value> {
    let Some(remote) = answer(root, &["remote", "get-url", "origin"])? else {
        return Ok(json!({"url": null}));
    };
    let parsed = Url::parse(&remote).ok();
    let repository = if let Some(path) = remote.strip_prefix("git@github.com:") {
        path
    } else if let Some(remote) = &parsed {
        if remote.host_str() != Some("github.com") {
            return Ok(json!({"url": null}));
        }
        remote.path()
    } else {
        return Ok(json!({"url": null}));
    };
    let repository = repository
        .trim_matches('/')
        .strip_suffix(".git")
        .unwrap_or(repository.trim_matches('/'));
    let Some((owner, repository)) = repository.split_once('/') else {
        return Ok(json!({"url": null}));
    };
    if repository.contains('/')
        || [owner, repository]
            .iter()
            .any(|part| part.is_empty() || matches!(*part, "." | ".."))
    {
        return Ok(json!({"url": null}));
    }
    let Some(commit) = commit_id(root, "HEAD")? else {
        return Ok(json!({"url": null}));
    };
    let file = super::super::diff_files::scoped_path(params)?;
    let root = root.canonicalize()?;
    let relative = file
        .strip_prefix(root)
        .context("file is outside the repository")?;
    let mut url = Url::parse("https://github.com")?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow!("GitHub URL cannot hold a path"))?;
        path.clear()
            .push(owner)
            .push(repository)
            .push("blob")
            .push(&commit);
        for component in relative.components() {
            path.push(
                component
                    .as_os_str()
                    .to_str()
                    .context("GitHub file path must be UTF-8")?,
            );
        }
    }
    if let Some(line) = params["line"].as_u64().filter(|line| *line > 0) {
        url.set_fragment(Some(&format!("L{line}")));
    }
    Ok(json!({"url": url.as_str()}))
}
