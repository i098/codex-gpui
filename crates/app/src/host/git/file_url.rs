use super::*;
use url::Url;

pub(super) fn read(root: &Path, params: &Value) -> Result<Value> {
    let Some(remote) = answer(root, &["remote", "get-url", "origin"])? else {
        return Ok(json!({"url": null}));
    };
    let parsed;
    let repository = if let Some((_, path)) = remote
        .split_once(':')
        .filter(|(host, _)| host.eq_ignore_ascii_case("git@github.com"))
    {
        path
    } else {
        let Ok(value) = Url::parse(&remote) else {
            return Ok(json!({"url": null}));
        };
        parsed = value;
        if parsed.host_str() != Some("github.com") {
            return Ok(json!({"url": null}));
        }
        parsed.path()
    };
    let repository = repository.trim_matches('/');
    let repository = repository.strip_suffix(".git").unwrap_or(repository);
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
    let Some(branch) = current_branch(root)? else {
        return Ok(json!({"url": null}));
    };
    let path = Path::new(params["path"].as_str().context("missing path")?);
    let cwd = Path::new(params["cwd"].as_str().context("missing cwd")?);
    let mut file = PathBuf::new();
    for component in cwd.join(path).components() {
        if component == std::path::Component::ParentDir {
            file.pop();
        } else {
            file.push(component.as_os_str());
        }
    }
    let Ok(relative) = file.strip_prefix(root) else {
        return Ok(json!({"url": null}));
    };
    if relative.as_os_str().is_empty() {
        return Ok(json!({"url": null}));
    }
    let mut url = Url::parse("https://github.com")?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow!("GitHub URL cannot hold a path"))?;
        path.clear().push(owner).push(repository).push("blob");
        path.extend(branch.split('/'));
        for component in relative.components() {
            path.push(
                component
                    .as_os_str()
                    .to_str()
                    .context("GitHub file path must be UTF-8")?,
            );
        }
    }
    if let Some(line) = params["line"].as_u64() {
        url.set_fragment(Some(&format!("L{line}")));
    }
    Ok(json!({"url": url.as_str()}))
}
