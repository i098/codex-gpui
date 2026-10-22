use super::LocalHost;
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write as _,
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

pub const DIRECTORY: &str = ".codex/environments";
pub const DEFAULT_FILE: &str = "environment.toml";
const MAX_ANCESTORS: usize = 50;
const SELECTIONS: &str = "local-env-selections-by-workspace";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Environment {
    #[serde(default = "initial_version")]
    pub version: u64,
    pub name: String,
    pub setup: Lifecycle,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup: Option<Lifecycle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<EnvironmentAction>,
}

fn initial_version() -> u64 {
    1
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Lifecycle {
    pub script: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub darwin: Option<PlatformScript>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linux: Option<PlatformScript>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub win32: Option<PlatformScript>,
}

impl Lifecycle {
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PlatformScript {
    pub script: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct EnvironmentAction {
    pub name: String,
    pub command: String,
    pub icon: Option<String>,
    pub platform: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ParsedEnvironment {
    #[serde(rename_all = "camelCase")]
    Success {
        config_path: PathBuf,
        cwd_relative_to_git_root: PathBuf,
        environment: Box<Environment>,
    },
    #[serde(rename_all = "camelCase")]
    Error {
        config_path: PathBuf,
        cwd_relative_to_git_root: PathBuf,
        error: ParseError,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ParseError {
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentConfig {
    pub config_path: PathBuf,
    pub exists: bool,
    pub revision: Option<String>,
    pub environment: Option<ParsedEnvironment>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SaveResult {
    Success,
    Conflict,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    Unset,
    Disabled,
    Config(PathBuf),
}

fn normalized(path: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "environment path must be absolute");
    Ok(
        super::normalized_project_path(path.to_str().context("environment path must be UTF-8")?)
            .into(),
    )
}

fn filesystem_path(path: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "environment path must be absolute");
    let mut result = PathBuf::with_capacity(path.as_os_str().len());
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            component => result.push(component),
        }
    }
    Ok(result)
}

/// Read project environments, including ancestors up to the nearest Git root.
pub fn list(workspace: &Path) -> Result<Vec<ParsedEnvironment>> {
    let workspace = filesystem_path(workspace)?;
    ensure!(workspace.is_dir(), "workspace must be a directory");
    let mut files = Vec::new();
    for (distance, root) in workspace.ancestors().take(MAX_ANCESTORS).enumerate() {
        if let Ok(entries) = fs::read_dir(root.join(DIRECTORY)) {
            for entry in entries {
                let path = entry?.path();
                if path
                    .extension()
                    .is_some_and(|extension| extension == "toml")
                {
                    let not_default = path.file_name().is_none_or(|name| name != DEFAULT_FILE);
                    files.push((not_default, distance, path));
                }
            }
        }
        if marker(root, ".git") {
            break;
        }
    }
    files.sort_unstable();
    Ok(files
        .into_iter()
        .map(|(_, _, path)| {
            let result = fs::read_to_string(&path)
                .context("read environment configuration")
                .and_then(|raw| parse(&raw));
            parsed_result(&path, result)
        })
        .collect())
}

fn parsed_result(path: &Path, result: Result<Environment>) -> ParsedEnvironment {
    match result {
        Ok(environment) => ParsedEnvironment::Success {
            config_path: path.into(),
            cwd_relative_to_git_root: relative_cwd(path),
            environment: Box::new(environment),
        },
        Err(error) => ParsedEnvironment::Error {
            config_path: path.into(),
            cwd_relative_to_git_root: path.into(),
            error: ParseError {
                message: format!(
                    "Failed to parse environment config at {}: {error:#}",
                    path.display()
                ),
            },
        },
    }
}

fn parse(raw: &str) -> Result<Environment> {
    parse_value(toml::from_str(raw)?)
}

pub(super) fn parse_value(mut value: toml::Value) -> Result<Environment> {
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
    for lifecycle in std::iter::once(&mut environment.setup).chain(environment.cleanup.iter_mut()) {
        trim(&mut lifecycle.script);
        for platform in [
            &mut lifecycle.darwin,
            &mut lifecycle.linux,
            &mut lifecycle.win32,
        ]
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

fn trim(text: &mut String) {
    let leading = text.len() - text.trim_start().len();
    text.drain(..leading);
    text.truncate(text.trim_end().len());
}

fn marker(root: &Path, name: &str) -> bool {
    fs::metadata(root.join(name)).is_ok_and(|metadata| metadata.is_dir() || metadata.is_file())
}

fn relative_cwd(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(path);
    let cwd = if parent
        .file_name()
        .is_some_and(|name| name == "environments")
        && parent
            .parent()
            .is_some_and(|root| root.file_name().is_some_and(|name| name == ".codex"))
    {
        let codex = parent.parent().expect("checked parent");
        if marker(parent, ".codex") || marker(parent, ".git") {
            parent
        } else if marker(codex, ".codex") || marker(codex, ".git") {
            codex
        } else {
            codex.parent().unwrap_or(codex)
        }
    } else {
        parent
            .ancestors()
            .find(|root| marker(root, ".codex") || marker(root, ".git"))
            .unwrap_or(path)
    };
    let git_root = cwd
        .ancestors()
        .find(|root| marker(root, ".git"))
        .or_else(|| cwd.ancestors().last())
        .unwrap_or(cwd);
    let relative = cwd.strip_prefix(git_root).unwrap_or(cwd);
    if relative.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        relative.into()
    }
}

/// Read the file and revision without replacing malformed configuration with defaults.
pub fn read_config(path: &Path) -> Result<EnvironmentConfig> {
    ensure!(path.is_absolute(), "environment path must be absolute");
    match fs::read_to_string(path) {
        Ok(raw) => Ok(EnvironmentConfig {
            config_path: path.to_owned(),
            exists: true,
            revision: Some(revision(&raw)),
            environment: Some(parsed_result(path, parse(&raw))),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(EnvironmentConfig {
            config_path: path.to_owned(),
            exists: false,
            revision: None,
            environment: None,
        }),
        Err(error) => Err(error).context("read local environment"),
    }
}

fn revision(raw: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(raw.as_bytes()))
}

/// Save only the revision the editor read, or accept identical content.
pub fn save_config(path: &Path, expected_revision: Option<&str>, raw: &str) -> Result<SaveResult> {
    // ponytail: native saves serialize; use per-file locks if concurrent saves become common.
    static WRITERS: Mutex<()> = Mutex::new(());
    let _guard = WRITERS.lock().unwrap_or_else(|error| error.into_inner());
    ensure!(path.is_absolute(), "environment path must be absolute");
    let parent = path.parent().context("environment path has no parent")?;
    fs::create_dir_all(parent)?;
    let current = match fs::read_to_string(path) {
        Ok(current) => Some(current),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("read environment before save"),
    };
    if current.as_deref() == Some(raw) {
        return Ok(SaveResult::Success);
    }
    match (&current, expected_revision) {
        (None, None) => {}
        (Some(current), Some(expected)) if revision(current) == expected => {}
        _ => return Ok(SaveResult::Conflict),
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
    let mut file = builder.tempfile_in(
        destination
            .parent()
            .context("environment destination has no parent")?,
    )?;
    if current.is_some() {
        file.as_file()
            .set_permissions(fs::metadata(&destination)?.permissions())?;
    }
    file.write_all(raw.as_bytes())?;
    file.as_file().sync_all()?;
    persist_config(file, &destination, current.as_deref(), raw)
}

fn persist_config(
    file: tempfile::NamedTempFile,
    destination: &Path,
    current: Option<&str>,
    raw: &str,
) -> Result<SaveResult> {
    if current.is_some() {
        // External editors do not take the native lock, so recheck after the disk flush.
        match fs::read_to_string(destination) {
            Ok(latest) if latest == raw => return Ok(SaveResult::Success),
            Ok(latest) if current != Some(latest.as_str()) => return Ok(SaveResult::Conflict),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SaveResult::Conflict);
            }
            Err(error) => return Err(error).context("recheck environment before replacing it"),
        }
        file.persist(destination)?;
    } else if let Err(error) = file.persist_noclobber(destination) {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            return match fs::read_to_string(destination) {
                Ok(current) => Ok(if current == raw {
                    SaveResult::Success
                } else {
                    SaveResult::Conflict
                }),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Ok(SaveResult::Conflict)
                }
                Err(error) => Err(error).context("read concurrently created environment"),
            };
        }
        return Err(error).context("create local environment");
    }
    Ok(SaveResult::Success)
}

fn workspace_key(workspace: &Path) -> Result<Option<String>> {
    if workspace.as_os_str().is_empty() || workspace == Path::new("/") {
        return Ok(None);
    }
    let root = normalized(workspace)?;
    Ok(Some(format!("local:{}", root.display())))
}

fn selection_value(value: &Value) -> Result<Selection> {
    if value.is_null() {
        return Ok(Selection::Disabled);
    }
    let path = value
        .as_str()
        .context("environment selection must be a path or null")?;
    Ok(Selection::Config(PathBuf::from(path)))
}

fn same_selection(left: &Selection, right: &Selection) -> bool {
    match (left, right) {
        (Selection::Config(left), Selection::Config(right)) => left
            .to_str()
            .zip(right.to_str())
            .is_some_and(|(left, right)| {
                super::normalized_project_path(left) == super::normalized_project_path(right)
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
        Err(error) => return Err(error).context("read environment selection"),
    };
    let state: Value = serde_json::from_slice(&raw)?;
    let Some(value) =
        state.pointer("/electron-persisted-atom-state/local-env-selections-by-workspace")
    else {
        return Ok(Selection::Unset);
    };
    if value.is_null() {
        return Ok(Selection::Unset);
    }
    let selections = value
        .as_object()
        .context("environment selections must be an object")?;
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

pub(super) fn select(host: &LocalHost, workspace: &Path, config: Option<&Path>) -> Result<()> {
    let key =
        workspace_key(workspace)?.context("environment selection needs a non-root workspace")?;
    let value = match config {
        Some(path) => {
            let config = read_config(path)?;
            ensure!(
                matches!(config.environment, Some(ParsedEnvironment::Success { .. })),
                "selected environment is not valid"
            );
            Value::String(
                config
                    .config_path
                    .to_str()
                    .context("environment path must be UTF-8")?
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
            .context("persisted atoms must be an object")?;
        let selections = atoms.entry(SELECTIONS).or_insert_with(|| json!({}));
        if selections.is_null() {
            *selections = json!({});
        }
        let selections = selections
            .as_object_mut()
            .context("environment selections must be an object")?;
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
    use crate::host::Host;

    #[test]
    fn edits_retain_invalid_files_and_reject_stale_revisions() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let workspace = directory.path().join("project");
        fs::create_dir_all(workspace.join(".git"))?;
        fs::create_dir_all(workspace.join("src"))?;
        let host = LocalHost::new(directory.path().join("home"));
        let config = workspace.join(DIRECTORY).join(DEFAULT_FILE);
        let raw = "name=' Project '\n[setup]\nscript=' echo first '\n[[actions]]\nname=''\ncommand='ignore'\n[[actions]]\nname='Run'\ncommand='run'\nicon='unknown'\n";
        assert_eq!(
            host.save_local_environment_config(&config, None, raw)?,
            SaveResult::Success
        );
        let read = host.local_environment_config(&config)?;
        let Some(ParsedEnvironment::Success { environment, .. }) = read.environment else {
            panic!("valid environment")
        };
        assert_eq!(environment.name, "Project");
        assert_eq!(environment.setup.script, "echo first");
        assert_eq!(environment.actions.len(), 1);
        assert_eq!(environment.actions[0].icon, None);
        fs::write(&config, "name='External edit'\n[setup]\nscript=''\n")?;
        assert_eq!(
            host.save_local_environment_config(&config, read.revision.as_deref(), raw)?,
            SaveResult::Conflict
        );
        assert!(fs::read_to_string(&config)?.contains("External edit"));
        let invalid = workspace.join(DIRECTORY).join("invalid.toml");
        fs::write(&invalid, "this is not TOML")?;
        let outside = directory.path().join(DIRECTORY).join(DEFAULT_FILE);
        host.save_local_environment_config(&outside, None, raw)?;
        let listed = host.local_environments(&workspace.join("src/../src"))?;
        assert_eq!(listed.len(), 2);
        assert!(
            matches!(&listed[0], ParsedEnvironment::Success { config_path, .. } if config_path == &config)
        );
        assert!(
            matches!(&listed[1], ParsedEnvironment::Error { config_path, .. } if config_path == &invalid)
        );
        let nested = workspace.join("pkg").join(DIRECTORY).join(DEFAULT_FILE);
        host.save_local_environment_config(&nested, None, raw)?;
        let nested_list = host.local_environments(&workspace.join("pkg"))?;
        assert!(
            matches!(&nested_list[0], ParsedEnvironment::Success { cwd_relative_to_git_root, .. } if cwd_relative_to_git_root == Path::new("pkg"))
        );
        assert!(
            matches!(&nested_list[1], ParsedEnvironment::Success { cwd_relative_to_git_root, .. } if cwd_relative_to_git_root == Path::new("."))
        );
        Ok(())
    }

    #[test]
    fn selection_keeps_disabled_distinct_and_preserves_other_state() -> Result<()> {
        let home = tempfile::tempdir()?;
        let workspace = home.path().join("project");
        let config = workspace.join(DIRECTORY).join(DEFAULT_FILE);
        let state_file = home.path().join(".codex-global-state.json");
        fs::write(
            &state_file,
            r#"{"unrelated":true,"electron-persisted-atom-state":{"keep":"old","local-env-selections-by-workspace":null}}"#,
        )?;
        let host = LocalHost::new(home.path());
        assert_eq!(
            host.local_environment_selection(&workspace)?,
            Selection::Unset
        );
        host.save_local_environment_config(&config, None, "name='Project'\n[setup]\nscript=''\n")?;
        host.select_local_environment(&workspace, Some(&config))?;
        assert_eq!(
            host.local_environment_selection(&workspace)?,
            Selection::Config(config.clone())
        );
        let state: Value = serde_json::from_slice(&fs::read(&state_file)?)?;
        assert_eq!(state["unrelated"], true);
        assert_eq!(state["electron-persisted-atom-state"]["keep"], "old");
        host.select_local_environment(&workspace, None)?;
        assert_eq!(
            host.local_environment_selection(&workspace)?,
            Selection::Disabled
        );
        assert_eq!(
            host.local_environment_selection(&workspace.join("other"))?,
            Selection::Unset
        );
        let mut state: Value = serde_json::from_slice(&fs::read(&state_file)?)?;
        let first_alias = format!("local:{}/./", workspace.display());
        let second_alias = format!("local:{}/../project", workspace.display());
        let mut aliases = serde_json::Map::new();
        aliases.insert(first_alias, json!(config));
        aliases.insert("local:rel/dir".into(), Value::Null);
        state["electron-persisted-atom-state"][SELECTIONS] = Value::Object(aliases);
        fs::write(&state_file, serde_json::to_vec(&state)?)?;
        assert_eq!(
            host.local_environment_selection(&workspace)?,
            Selection::Config(config.clone())
        );
        state["electron-persisted-atom-state"][SELECTIONS][second_alias] = Value::Null;
        fs::write(&state_file, serde_json::to_vec(&state)?)?;
        assert_eq!(
            host.local_environment_selection(&workspace)?,
            Selection::Disabled
        );
        let exact = workspace_key(&workspace)?.expect("workspace key");
        state["electron-persisted-atom-state"][SELECTIONS][exact] = json!(config);
        fs::write(&state_file, serde_json::to_vec(&state)?)?;
        assert_eq!(
            host.local_environment_selection(&workspace)?,
            Selection::Config(config)
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn saves_preserve_symlinks_and_file_permissions() -> Result<()> {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let directory = tempfile::tempdir()?;
        let target = directory.path().join("target.toml");
        let link = directory.path().join("environment.toml");
        fs::write(&target, "name='Old'\n[setup]\nscript=''\n")?;
        let fresh = directory.path().join("fresh.toml");
        save_config(&fresh, None, "name='New'\n[setup]\nscript=''\n")?;
        assert_eq!(
            fs::metadata(&fresh)?.permissions().mode() & 0o777,
            fs::metadata(&target)?.permissions().mode() & 0o777
        );
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640))?;
        symlink(&target, &link)?;
        let snapshot = read_config(&link)?;
        let updated = "name='New'\n[setup]\nscript=''\n";
        assert_eq!(
            save_config(&link, snapshot.revision.as_deref(), updated)?,
            SaveResult::Success
        );
        assert!(fs::symlink_metadata(&link)?.file_type().is_symlink());
        assert_eq!(fs::read_to_string(&target)?, updated);
        assert_eq!(fs::metadata(&target)?.permissions().mode() & 0o777, 0o640);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn file_operations_preserve_literal_backslashes() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let workspace = directory.path().join("old\\app");
        let config = workspace.join(DIRECTORY).join(DEFAULT_FILE);
        let host = LocalHost::new(directory.path().join("home"));
        host.save_local_environment_config(
            &config,
            None,
            "name='Literal path'\n[setup]\nscript=''\n",
        )?;
        assert!(config.is_file());
        assert!(!directory.path().join("old/app").exists());
        assert_eq!(host.local_environment_config(&config)?.config_path, config);
        assert_eq!(host.local_environments(&workspace)?.len(), 1);
        host.select_local_environment(&workspace, Some(&config))?;
        assert_eq!(
            host.local_environment_selection(&workspace)?,
            Selection::Config(config)
        );
        Ok(())
    }
}
