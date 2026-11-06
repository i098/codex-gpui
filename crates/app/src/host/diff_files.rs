use super::*;
use std::{io::Read as _, process::Command};

pub(super) fn scoped_path(params: &Value) -> Result<PathBuf> {
    let cwd = Path::new(params["cwd"].as_str().context("missing cwd")?);
    let path = Path::new(params["path"].as_str().context("missing path")?);
    ensure!(
        cwd.is_absolute() && path.is_absolute(),
        "cwd and path must be absolute"
    );
    let root = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map_or_else(
            || cwd.to_path_buf(),
            |root| PathBuf::from(root.trim_end_matches('\n')),
        );
    let root = root.canonicalize().context("resolve the project folder")?;
    let file = path
        .canonicalize()
        .with_context(|| format!("{} no longer exists", path.display()))?;
    ensure!(file.starts_with(&root), "file is outside the project");
    ensure!(file.is_file(), "{} is not a file", file.display());
    Ok(file)
}

pub(super) fn text(params: &Value) -> Result<Value> {
    let path = scoped_path(params)?;
    Ok(json!({"contents": fs::read_to_string(&path)?, "mtimeMs": mtime_ms(&path)?}))
}

/// The original's `write-file`: answers `conflict` without writing when the file changed on
/// disk since `expectedMtimeMs`, so a Files tab edit never overwrites a newer write.
pub(super) fn write(params: &Value) -> Result<Value> {
    let path = scoped_path(params)?;
    let content = params["content"].as_str().context("missing content")?;
    let mtime = mtime_ms(&path)?;
    if let Some(expected) = params["expectedMtimeMs"].as_u64()
        && expected != mtime
    {
        return Ok(json!({"outcome": "conflict", "mtimeMs": mtime}));
    }
    fs::write(&path, content)?;
    Ok(json!({"outcome": "saved", "mtimeMs": mtime_ms(&path)?}))
}

fn mtime_ms(path: &Path) -> Result<u64> {
    let modified = fs::metadata(path)?.modified()?;
    Ok(modified.duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64)
}

/// Matches the original's optional `maxBytes`; omitting it reads the complete file.
pub(super) fn binary(params: &Value) -> Result<Value> {
    let path = scoped_path(params)?;
    let limit = params
        .get("maxBytes")
        .map(|value| {
            value
                .as_u64()
                .context("maxBytes must be an unsigned integer")
        })
        .transpose()?;
    let bytes = if let Some(limit) = limit {
        if fs::metadata(&path)?.len() > limit {
            return Ok(json!({"contentsBase64": null}));
        }
        let mut bytes = Vec::new();
        fs::File::open(&path)?
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Ok(json!({"contentsBase64": null}));
        }
        bytes
    } else {
        fs::read(&path)?
    };
    Ok(binary_value(&bytes, &path))
}

pub(super) fn git_binary(params: &Value) -> Result<Value> {
    let bytes = git::read_file_version(params)?;
    Ok(match bytes {
        Some(bytes) if !bytes.is_empty() => {
            binary_value(&bytes, Path::new(params["path"].as_str().unwrap_or("")))
        }
        _ => json!({"contentsBase64": null}),
    })
}

fn binary_value(bytes: &[u8], path: &Path) -> Value {
    let mut result = json!({"contentsBase64": STANDARD.encode(bytes)});
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
        Some("image/webp")
    } else if bytes.starts_with(b"%PDF-") {
        Some("application/pdf")
    } else if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        Some("image/svg+xml")
    } else {
        None
    };
    if let Some(mime) = mime {
        result["mimeType"] = mime.into();
    }
    result
}
