use super::*;
use sha2::{Digest, Sha256};

fn revision(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn parse(path: &Path, bytes: &[u8]) -> Result<Value> {
    let text = std::str::from_utf8(bytes).context("Environment file is not UTF-8")?;
    let table: toml::Value = toml::from_str(text)?;
    let value = serde_json::to_value(table)?;
    ensure!(value["version"] == 1, "Unsupported environment version");
    ensure!(
        value["name"]
            .as_str()
            .is_some_and(|name| !name.trim().is_empty()),
        "Enter an environment name"
    );
    for lifecycle in ["setup", "cleanup"] {
        if lifecycle == "cleanup" && value[lifecycle].is_null() {
            continue;
        }
        ensure!(
            value[lifecycle]["script"].is_string(),
            "Invalid {lifecycle} script"
        );
        for platform in ["darwin", "linux", "win32"] {
            let script = &value[lifecycle][platform];
            ensure!(
                script.is_null() || script["script"].is_string(),
                "Invalid {lifecycle} platform script"
            );
        }
    }
    if let Some(actions) = value.get("actions") {
        for action in actions.as_array().context("Invalid environment actions")? {
            for field in ["name", "command"] {
                ensure!(
                    action[field].as_str().is_some_and(|v| !v.trim().is_empty()),
                    "Enter an action {field}"
                );
            }
            ensure!(
                action
                    .get("icon")
                    .is_none_or(|v| matches!(v.as_str(), Some("tool" | "run" | "debug" | "test"))),
                "Invalid action icon"
            );
            ensure!(
                action
                    .get("platform")
                    .is_none_or(|v| matches!(v.as_str(), Some("darwin" | "linux" | "win32"))),
                "Invalid action platform"
            );
        }
    }
    Ok(json!({"type": "success", "configPath": path, "environment": value}))
}

pub(super) fn read(path: &Path) -> Result<Value> {
    ensure!(path.is_absolute(), "Environment path must be absolute");
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(
                json!({"configPath": path, "exists": false, "environment": null, "revision": null}),
            );
        }
        Err(error) => return Err(error.into()),
    };
    let environment = parse(path, &bytes).unwrap_or_else(
        |error| json!({"type": "error", "configPath": path, "error": error.to_string()}),
    );
    Ok(
        json!({"configPath": path, "exists": true, "environment": environment, "revision": revision(&bytes)}),
    )
}

pub(super) fn list(workspace: &Path) -> Result<Value> {
    ensure!(workspace.is_absolute(), "Workspace path must be absolute");
    let mut environments = Vec::new();
    for ancestor in workspace.ancestors() {
        let directory = ancestor.join(".codex/environments");
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_file()
                && entry.path().extension().is_some_and(|ext| ext == "toml")
            {
                paths.push(entry.path());
            }
        }
        paths.sort();
        for path in paths {
            let data = read(&path)?;
            environments.push(data["environment"].clone());
        }
    }
    Ok(json!({"environments": environments}))
}

pub(super) fn save(params: &Value) -> Result<Value> {
    let path = Path::new(
        params["configPath"]
            .as_str()
            .context("Missing environment path")?,
    );
    ensure!(path.is_absolute(), "Environment path must be absolute");
    let directory = path.parent().context("Environment path has no parent")?;
    ensure!(
        path.extension().is_some_and(|ext| ext == "toml")
            && directory
                .file_name()
                .is_some_and(|name| name == "environments")
            && directory
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == ".codex"),
        "Expected a .codex/environments TOML file"
    );
    let raw = params["raw"]
        .as_str()
        .context("Missing environment contents")?;
    parse(path, raw.as_bytes())?;
    fs::create_dir_all(directory)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join(".edit.lock"))?;
    lock.lock()?;
    let current = match fs::read(path) {
        Ok(bytes) => Some(revision(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if current.as_deref() != params["expectedRevision"].as_str() {
        return Ok(json!({"type": "conflict", "configPath": path}));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(raw.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(json!({"type": "success", "configPath": path, "revision": revision(raw.as_bytes())}))
}
