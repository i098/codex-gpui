use super::*;
use sha2::{Digest, Sha256};

const DIRECTORY: &str = ".codex/environments";
const SELECTIONS: &str = "local-env-selections-by-workspace";

#[derive(Deserialize, Serialize)]
pub(super) struct Environment {
    #[serde(default = "initial_version")]
    version: u64,
    name: String,
    pub setup: Scripts,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup: Option<Scripts>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    actions: Vec<Action>,
}

fn initial_version() -> u64 {
    1
}

#[derive(Deserialize, Serialize)]
pub(super) struct Scripts {
    script: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    darwin: Option<Script>,
    #[serde(skip_serializing_if = "Option::is_none")]
    linux: Option<Script>,
    #[serde(skip_serializing_if = "Option::is_none")]
    win32: Option<Script>,
}

#[derive(Deserialize, Serialize)]
struct Script {
    script: String,
}

#[derive(Deserialize, Serialize)]
struct Action {
    name: String,
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
}

impl Scripts {
    pub(super) fn current(&self) -> &str {
        let platform = if cfg!(target_os = "macos") {
            &self.darwin
        } else if cfg!(windows) {
            &self.win32
        } else {
            &self.linux
        };
        platform
            .as_ref()
            .map(|script| script.script.as_str())
            .filter(|script| !script.is_empty())
            .unwrap_or(&self.script)
    }
}

fn revision(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn trim(text: &mut String) {
    let leading = text.len() - text.trim_start().len();
    text.drain(..leading);
    text.truncate(text.trim_end().len());
}

fn parse(bytes: &[u8]) -> Result<Environment> {
    let text = std::str::from_utf8(bytes).context("Environment file is not UTF-8")?;
    let mut value: toml::Value = toml::from_str(text)?;
    if let Some(actions) = value.get_mut("actions").and_then(toml::Value::as_array_mut) {
        actions.retain(|action| {
            ["name", "command"].iter().all(|key| {
                action
                    .get(key)
                    .and_then(toml::Value::as_str)
                    .is_some_and(|text| !text.trim().is_empty())
            })
        });
        for action in actions {
            if !action
                .get("icon")
                .and_then(toml::Value::as_str)
                .is_some_and(|icon| ["tool", "run", "debug", "test"].contains(&icon))
            {
                action
                    .as_table_mut()
                    .context("action must be a table")?
                    .remove("icon");
            }
        }
    }
    let mut environment: Environment = value.try_into()?;
    ensure!(
        environment.version > 0,
        "environment version must be at least 1"
    );
    trim(&mut environment.name);
    for scripts in std::iter::once(&mut environment.setup).chain(environment.cleanup.iter_mut()) {
        trim(&mut scripts.script);
        for platform in [&mut scripts.darwin, &mut scripts.linux, &mut scripts.win32]
            .into_iter()
            .flatten()
        {
            trim(&mut platform.script);
        }
    }
    for action in &mut environment.actions {
        trim(&mut action.name);
        trim(&mut action.command);
        ensure!(
            action
                .platform
                .as_deref()
                .is_none_or(|platform| ["darwin", "linux", "win32"].contains(&platform)),
            "unknown action platform"
        );
    }
    Ok(environment)
}

pub(super) fn config(path: &Path) -> Result<Environment> {
    ensure!(path.is_absolute(), "Environment path must be absolute");
    parse(&fs::read(path)?)
        .with_context(|| format!("Invalid environment config at {}", path.display()))
}

pub(super) fn workspace_dir(path: &Path) -> &Path {
    let marked = |dir: &Path| dir.join(".codex").exists() || dir.join(".git").exists();
    let dir = path.parent().unwrap_or(path);
    let codex = dir.parent().filter(|codex| {
        dir.file_name() == Some("environments".as_ref())
            && codex.file_name() == Some(".codex".as_ref())
    });
    match codex {
        Some(_) if marked(dir) => dir,
        Some(codex) if marked(codex) => codex,
        Some(codex) => codex.parent().unwrap_or(codex),
        None => dir.ancestors().find(|dir| marked(dir)).unwrap_or(path),
    }
}

fn relative_cwd(path: &Path) -> &Path {
    let project = workspace_dir(path);
    let root = project
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .or_else(|| project.ancestors().last())
        .unwrap_or(project);
    let relative = project.strip_prefix(root).unwrap_or(project);
    if relative.as_os_str().is_empty() {
        Path::new(".")
    } else {
        relative
    }
}

fn parsed(path: &Path, result: Result<Environment>) -> Value {
    match result {
        Ok(environment) => json!({
            "type": "success", "configPath": path,
            "cwdRelativeToGitRoot": relative_cwd(path), "environment": environment
        }),
        Err(error) => json!({
            "type": "error", "configPath": path, "cwdRelativeToGitRoot": path,
            "error": {"message": format!("Failed to parse environment config at {}: {error:#}", path.display())}
        }),
    }
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
    Ok(json!({
        "configPath": path, "exists": true,
        "environment": parsed(path, parse(&bytes)), "revision": revision(&bytes)
    }))
}

pub(super) fn list(workspace: &Path) -> Result<Value> {
    ensure!(workspace.is_absolute(), "Workspace path must be absolute");
    let mut normalized = PathBuf::with_capacity(workspace.as_os_str().len());
    for component in workspace.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            component => normalized.push(component),
        }
    }
    ensure!(normalized.is_dir(), "Workspace must be a directory");
    let mut paths = Vec::new();
    for (distance, ancestor) in normalized.ancestors().take(50).enumerate() {
        match fs::read_dir(ancestor.join(DIRECTORY)) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    let path = entry.path();
                    let kind = entry.file_type()?;
                    if (kind.is_file() || kind.is_symlink())
                        && path.extension().is_some_and(|ext| ext == "toml")
                    {
                        let not_default = path
                            .file_name()
                            .is_none_or(|name| name != "environment.toml");
                        paths.push((not_default, distance, path));
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if ancestor.join(".git").exists() {
            break;
        }
    }
    paths.sort_unstable();
    let environments: Vec<_> = paths
        .into_iter()
        .map(|(_, _, path)| {
            let result = fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|bytes| parse(&bytes));
            parsed(&path, result)
        })
        .collect();
    Ok(json!({"environments": environments}))
}

pub(super) fn save(params: &Value) -> Result<Value> {
    let path = Path::new(
        params["configPath"]
            .as_str()
            .context("Missing environment path")?,
    );
    ensure!(path.is_absolute(), "Environment path must be absolute");
    let raw = params["raw"]
        .as_str()
        .context("Missing environment contents")?;
    parse(raw.as_bytes())?;
    let directory = path.parent().context("Environment path has no parent")?;
    fs::create_dir_all(directory)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join(".edit.lock"))?;
    lock.lock()?;
    let current = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if current.as_deref() != Some(raw.as_bytes()) {
        if current.as_deref().map(revision).as_deref() != params["expectedRevision"].as_str() {
            return Ok(json!({"type": "conflict", "configPath": path}));
        }
        let destination = if current.is_some() {
            fs::canonicalize(path)?
        } else {
            path.to_owned()
        };
        let mut builder = tempfile::Builder::new();
        #[cfg(unix)]
        if current.is_none() {
            use std::os::unix::fs::PermissionsExt as _;
            builder.permissions(fs::Permissions::from_mode(0o666));
        }
        let mut temporary = builder.tempfile_in(
            destination
                .parent()
                .context("Environment destination has no parent")?,
        )?;
        if current.is_some() {
            temporary
                .as_file()
                .set_permissions(fs::metadata(&destination)?.permissions())?;
        }
        temporary.write_all(raw.as_bytes())?;
        temporary.as_file().sync_all()?;
        if !persist(temporary, &destination, current.as_deref(), raw.as_bytes())? {
            return Ok(json!({"type": "conflict", "configPath": path}));
        }
    }
    Ok(json!({"type": "success", "configPath": path, "revision": revision(raw.as_bytes())}))
}

fn persist(
    file: tempfile::NamedTempFile,
    path: &Path,
    current: Option<&[u8]>,
    raw: &[u8],
) -> Result<bool> {
    if current.is_some() {
        match fs::read(path) {
            Ok(latest) if latest == raw => return Ok(true),
            Ok(latest) if current != Some(latest.as_slice()) => return Ok(false),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        }
        file.persist(path)?;
    } else if let Err(error) = file.persist_noclobber(path) {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            return match fs::read(path) {
                Ok(latest) => Ok(latest == raw),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error.into()),
            };
        }
        return Err(error.into());
    }
    Ok(true)
}

#[derive(Debug, PartialEq)]
pub(super) enum Selection {
    Unset,
    Disabled,
    Config(PathBuf),
}

fn workspace_key(workspace: &Path) -> Result<Option<String>> {
    if workspace.as_os_str().is_empty() || workspace == Path::new("/") {
        return Ok(None);
    }
    ensure!(workspace.is_absolute(), "Workspace path must be absolute");
    let normalized =
        normalized_project_path(workspace.to_str().context("Workspace path must be UTF-8")?);
    Ok(Some(format!("local:{normalized}")))
}

fn selection_value(value: &Value) -> Result<Selection> {
    if value.is_null() {
        return Ok(Selection::Disabled);
    }
    Ok(Selection::Config(
        value
            .as_str()
            .context("Environment selection must be a path or null")?
            .into(),
    ))
}

fn same_selection(left: &Selection, right: &Selection) -> bool {
    match (left, right) {
        (Selection::Config(left), Selection::Config(right)) => left
            .to_str()
            .zip(right.to_str())
            .is_some_and(|(left, right)| {
                normalized_project_path(left) == normalized_project_path(right)
            }),
        _ => left == right,
    }
}

pub(super) fn selection(host: &LocalHost, workspace: &Path) -> Result<Selection> {
    let Some(key) = workspace_key(workspace)? else {
        return Ok(Selection::Unset);
    };
    let raw = match fs::read(host.home.join(".codex-global-state.json")) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Selection::Unset),
        Err(error) => return Err(error.into()),
    };
    let state: Value = serde_json::from_slice(&raw)?;
    let Some(value) = state
        .pointer("/electron-persisted-atom-state/local-env-selections-by-workspace")
        .filter(|value| !value.is_null())
    else {
        return Ok(Selection::Unset);
    };
    let selections = value
        .as_object()
        .context("Environment selections must be an object")?;
    if let Some(value) = selections.get(&key) {
        return selection_value(value);
    }
    let mut selected = Selection::Unset;
    for (candidate, value) in selections {
        let Some(candidate) = candidate.strip_prefix("local:") else {
            continue;
        };
        if workspace_key(Path::new(candidate))
            .ok()
            .flatten()
            .as_deref()
            != Some(key.as_str())
        {
            continue;
        }
        let value = selection_value(value)?;
        if selected != Selection::Unset && !same_selection(&selected, &value) {
            return Ok(Selection::Disabled);
        }
        selected = value;
    }
    Ok(selected)
}

pub(super) fn select(host: &LocalHost, workspace: &Path, path: Option<&Path>) -> Result<()> {
    let key =
        workspace_key(workspace)?.context("Environment selection needs a non-root workspace")?;
    let value = match path {
        Some(path) => {
            config(path)?;
            Value::String(
                path.to_str()
                    .context("Environment path must be UTF-8")?
                    .into(),
            )
        }
        None => Value::Null,
    };
    host.update_global_state(|state| {
        if state["electron-persisted-atom-state"].is_null() {
            state["electron-persisted-atom-state"] = json!({});
        }
        let atoms = state["electron-persisted-atom-state"]
            .as_object_mut()
            .context("Persisted atoms must be an object")?;
        let selections = atoms.entry(SELECTIONS).or_insert_with(|| json!({}));
        if selections.is_null() {
            *selections = json!({});
        }
        let selections = selections
            .as_object_mut()
            .context("Environment selections must be an object")?;
        if selections.get(&key) == Some(&value) {
            return Ok(false);
        }
        selections.insert(key, value);
        Ok(true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = "name=' Project '\n[setup]\nscript=' echo setup '\n";

    #[test]
    fn listing_keeps_parse_errors_and_stops_at_the_git_root() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let project = directory.path().join("project");
        let nested = project.join("pkg");
        fs::create_dir_all(project.join(".git"))?;
        fs::create_dir_all(nested.join(DIRECTORY))?;
        fs::create_dir_all(directory.path().join(DIRECTORY))?;
        fs::write(directory.path().join(DIRECTORY).join("outside.toml"), RAW)?;
        let root = project.join(DIRECTORY).join("z.toml");
        fs::create_dir_all(root.parent().unwrap())?;
        fs::write(&root, RAW)?;
        let default = nested.join(DIRECTORY).join("environment.toml");
        fs::write(
            &default,
            format!(
                "{RAW}[[actions]]\nname=''\ncommand='discard'\n[[actions]]\nname=' Run '\ncommand=' run '\nicon='unknown'\n"
            ),
        )?;
        let invalid = nested.join(DIRECTORY).join("invalid.toml");
        fs::write(&invalid, "not valid TOML")?;
        let result = list(&nested.join("../pkg"))?;
        let rows = result["environments"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["configPath"], json!(default));
        assert_eq!(rows[0]["cwdRelativeToGitRoot"], "pkg");
        assert_eq!(rows[0]["environment"]["version"], 1);
        assert_eq!(rows[0]["environment"]["name"], "Project");
        assert_eq!(rows[0]["environment"]["setup"]["script"], "echo setup");
        assert_eq!(
            rows[0]["environment"]["actions"],
            json!([{"name": "Run", "command": "run"}])
        );
        assert_eq!(rows[1]["type"], "error");
        assert_eq!(rows[1]["cwdRelativeToGitRoot"], json!(invalid));
        assert!(
            rows[1]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Failed to parse")
        );
        assert_eq!(rows[2]["configPath"], json!(root));
        assert_eq!(rows[2]["cwdRelativeToGitRoot"], ".");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn saves_preserve_symlinks_permissions_and_external_edits() -> Result<()> {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let directory = tempfile::tempdir()?;
        let workspace = directory.path().join("old\\app");
        fs::create_dir_all(workspace.join(DIRECTORY))?;
        let target = directory.path().join("target.toml");
        fs::write(&target, RAW)?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640))?;
        let path = workspace.join(DIRECTORY).join("environment.toml");
        symlink(&target, &path)?;
        let before = read(&path)?;
        assert!(before["revision"].as_str().unwrap().starts_with("sha256:"));
        let updated = RAW.replace("Project", "Updated");
        let saved = save(
            &json!({"configPath": path, "expectedRevision": before["revision"], "raw": updated}),
        )?;
        assert_eq!(saved["type"], "success");
        assert_eq!(saved["revision"], read(&path)?["revision"]);
        assert!(fs::symlink_metadata(&path)?.file_type().is_symlink());
        assert_eq!(fs::metadata(&target)?.permissions().mode() & 0o777, 0o640);
        assert_eq!(fs::read_to_string(&target)?, updated);
        assert_eq!(
            save(
                &json!({"configPath": path, "expectedRevision": before["revision"], "raw": updated})
            )?["type"],
            "success"
        );
        fs::write(&target, RAW.replace("Project", "External"))?;
        assert_eq!(
            save(
                &json!({"configPath": path, "expectedRevision": saved["revision"], "raw": updated})
            )?["type"],
            "conflict"
        );
        assert!(fs::read_to_string(&target)?.contains("External"));
        assert_eq!(
            list(&workspace)?["environments"][0]["configPath"],
            json!(path)
        );
        assert!(!directory.path().join("old/app").exists());
        Ok(())
    }

    #[test]
    fn selections_keep_disabled_unset_aliases_and_unrelated_state_distinct() -> Result<()> {
        let home = tempfile::tempdir()?;
        let host = LocalHost::new(home.path());
        let workspace = home.path().join("project");
        let path = workspace.join(DIRECTORY).join("environment.toml");
        save(&json!({"configPath": path, "raw": RAW, "expectedRevision": null}))?;
        let state_path = home.path().join(".codex-global-state.json");
        fs::write(
            &state_path,
            r#"{"keep":true,"electron-persisted-atom-state":{"keep":"value"}}"#,
        )?;
        assert_eq!(selection(&host, &workspace)?, Selection::Unset);
        select(&host, &workspace, Some(&path))?;
        assert_eq!(
            selection(&host, &workspace)?,
            Selection::Config(path.clone())
        );
        select(&host, &workspace, None)?;
        assert_eq!(selection(&host, &workspace)?, Selection::Disabled);
        assert_eq!(
            selection(&host, &workspace.join("other"))?,
            Selection::Unset
        );
        let mut state: Value = serde_json::from_slice(&fs::read(&state_path)?)?;
        assert_eq!(state["keep"], true);
        assert_eq!(state["electron-persisted-atom-state"]["keep"], "value");
        let first = format!("local:{}/./", workspace.display());
        let second = format!("local:{}/../project", workspace.display());
        let mut aliases = serde_json::Map::new();
        aliases.insert(first, json!(path));
        aliases.insert("local:relative/path".into(), Value::Null);
        state["electron-persisted-atom-state"][SELECTIONS] = Value::Object(aliases);
        fs::write(&state_path, serde_json::to_vec(&state)?)?;
        assert_eq!(
            selection(&host, &workspace)?,
            Selection::Config(path.clone())
        );
        state["electron-persisted-atom-state"][SELECTIONS][second] = Value::Null;
        fs::write(&state_path, serde_json::to_vec(&state)?)?;
        assert_eq!(selection(&host, &workspace)?, Selection::Disabled);
        let exact = workspace_key(&workspace)?.unwrap();
        state["electron-persisted-atom-state"][SELECTIONS][exact] = json!(path);
        fs::write(&state_path, serde_json::to_vec(&state)?)?;
        assert_eq!(selection(&host, &workspace)?, Selection::Config(path));
        assert_eq!(selection(&host, Path::new("/"))?, Selection::Unset);
        Ok(())
    }
}
