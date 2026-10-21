use super::LocalHost;
use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
};
use ui::screens::connections::{Connection, SshDraft};

const SAVED: &str = "codex-managed-remote-connections";
const AUTO_CONNECT: &str = "remote-connection-auto-connect-by-host-id";
const SETUP_COMPLETE: &str = "codex-mobile-has-connected-device";

pub struct SavedConnections {
    pub rows: Vec<Connection>,
    pub setup_complete: bool,
}

impl LocalHost {
    pub fn connections(&self) -> Result<Vec<Connection>> {
        Ok(self.saved_connections()?.rows)
    }

    pub fn saved_connections(&self) -> Result<SavedConnections> {
        let state = match fs::read(self.home.join(".codex-global-state.json")) {
            Ok(bytes) => serde_json::from_slice::<Value>(&bytes)?,
            Err(error) if error.kind() == ErrorKind::NotFound => json!({}),
            Err(error) => return Err(error).context("read remote connections"),
        };
        let mut rows: Vec<Connection> = match state.get(SAVED) {
            Some(value) => {
                serde_json::from_value(value.clone()).context("invalid saved remote connections")?
            }
            None => Vec::new(),
        };
        for row in &mut rows {
            row.auto_connect = state[AUTO_CONNECT][&row.host_id].as_bool().unwrap_or(false);
            row.color = state["remote-host-globe-color-by-host-id"][&row.host_id]
                .as_str()
                .map(str::to_owned);
        }
        rows.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        Ok(SavedConnections {
            rows,
            setup_complete: state[SETUP_COMPLETE].as_bool().unwrap_or(false),
        })
    }

    pub fn record_mobile_setup(&self) -> Result<()> {
        self.update_global_state(|state| {
            state[SETUP_COMPLETE] = json!(true);
            Ok(true)
        })
    }

    pub fn save_connection(&self, draft: &SshDraft) -> Result<Connection> {
        let rows = self.connections()?;
        draft.validate(&rows).map_err(anyhow::Error::msg)?;
        let connection = Connection {
            host_id: if draft.alias {
                let alias = url::form_urlencoded::byte_serialize(draft.hostname.trim().as_bytes())
                    .collect::<String>()
                    .replace('+', "%20");
                format!("remote-ssh-discovered:{alias}")
            } else {
                draft
                    .host_id
                    .clone()
                    .unwrap_or_else(|| format!("remote-ssh-codex-managed:{}", crate::uuid_v4()))
            },
            display_name: draft.display_name.trim().into(),
            source: if draft.alias {
                "discovered"
            } else {
                "codex-managed"
            }
            .into(),
            alias: draft.alias.then(|| draft.hostname.trim().into()),
            hostname: (!draft.alias).then(|| draft.hostname.trim().into()),
            ssh_port: (!draft.alias)
                .then(|| draft.port.trim().parse::<u16>().ok())
                .flatten(),
            identity: (!draft.alias && draft.use_identity).then(|| draft.identity.trim().into()),
            color: match &draft.color {
                Some(color) => color.clone(),
                None => rows
                    .iter()
                    .find(|row| Some(&row.host_id) == draft.host_id.as_ref())
                    .and_then(|row| row.color.clone()),
            },
            ..Default::default()
        };
        self.update_global_state(|state| {
            if state[SAVED].is_null() {
                state[SAVED] = json!([]);
            }
            let saved = state[SAVED]
                .as_array_mut()
                .context("Invalid saved remote connections")?;
            let mut value = saved
                .iter()
                .find(|row| {
                    row["hostId"].as_str() == draft.host_id.as_deref()
                        || row["hostId"] == connection.host_id
                })
                .cloned()
                .unwrap_or_else(|| json!({}));
            let object = value
                .as_object_mut()
                .context("Invalid saved remote connection")?;
            object.extend(
                serde_json::to_value(&connection)?
                    .as_object()
                    .context("Invalid connection")?
                    .clone(),
            );
            saved.retain(|row| {
                row["hostId"].as_str() != draft.host_id.as_deref()
                    && row["hostId"] != connection.host_id
            });
            saved.push(value);
            if let Some(color) = &draft.color {
                let key = "remote-host-globe-color-by-host-id";
                if state[key].is_null() {
                    state[key] = json!({});
                }
                let colors = state[key]
                    .as_object_mut()
                    .context("Invalid remote connection colors")?;
                if let Some(color) = color {
                    colors.insert(connection.host_id.clone(), json!(color));
                } else {
                    colors.remove(&connection.host_id);
                }
                if colors.is_empty() {
                    state
                        .as_object_mut()
                        .context("Invalid global state")?
                        .remove(key);
                }
            }
            Ok(true)
        })?;
        Ok(connection)
    }

    pub fn delete_connection(&self, host_id: &str) -> Result<()> {
        self.update_global_state(|state| {
            let rows = state[SAVED]
                .as_array_mut()
                .context("Invalid saved remote connections")?;
            let count = rows.len();
            rows.retain(|row| row["hostId"] != host_id);
            ensure!(rows.len() != count, "Remote connection not found");
            if let Some(auto) = state[AUTO_CONNECT].as_object_mut() {
                auto.remove(host_id);
            }
            Ok(true)
        })
    }

    pub fn set_connection_auto_connect(&self, host_id: &str, enabled: bool) -> Result<()> {
        self.update_global_state(|state| {
            if state[AUTO_CONNECT].is_null() {
                state[AUTO_CONNECT] = json!({});
            }
            let values = state[AUTO_CONNECT]
                .as_object_mut()
                .context("invalid remote connection preferences")?;
            values.insert(host_id.to_owned(), json!(enabled));
            Ok(true)
        })
    }
}

pub fn discover_ssh(home: &Path) -> Result<Vec<String>> {
    let root = home.join(".ssh");
    let mut visited = HashSet::new();
    let mut aliases = HashSet::new();
    read_ssh_config(
        &root.join("config"),
        &root,
        home,
        &mut visited,
        &mut aliases,
    )?;
    let mut aliases: Vec<_> = aliases.into_iter().collect();
    aliases.sort();
    Ok(aliases)
}

fn words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escape = false;
    for character in line.chars() {
        if escape {
            word.push(character);
            escape = false;
            continue;
        }
        if character == '\\' {
            escape = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else {
                word.push(character);
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == '#' {
            break;
        } else if character.is_whitespace() || character == '=' {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(character);
        }
    }
    if escape {
        word.push('\\');
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

fn read_ssh_config(
    path: &Path,
    root: &Path,
    home: &Path,
    visited: &mut HashSet<PathBuf>,
    aliases: &mut HashSet<String>,
) -> Result<()> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).context("read SSH config"),
    };
    if !visited.insert(path.canonicalize()?) {
        return Ok(());
    }
    for line in source.lines() {
        let parts = words(line);
        let Some((name, values)) = parts.split_first() else {
            continue;
        };
        if name.eq_ignore_ascii_case("Host") {
            if let Some(alias) = values
                .iter()
                .find(|v| !v.contains(['!', '*', '?', '[', ']']))
                && !alias.eq_ignore_ascii_case("colima")
            {
                aliases.insert(alias.clone());
            }
        } else if name.eq_ignore_ascii_case("Include") {
            for pattern in values {
                let path = if let Some(rest) = pattern.strip_prefix("~/") {
                    home.join(rest)
                } else {
                    let path = PathBuf::from(pattern);
                    if path.is_absolute() {
                        path
                    } else {
                        root.join(path)
                    }
                };
                for entry in glob::glob(path.to_str().context("SSH include path is not UTF-8")?)? {
                    let path = entry?;
                    if path.metadata()?.is_file() {
                        read_ssh_config(&path, root, home, visited, aliases)?;
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn ssh_command(connection: &Connection) -> Result<Command> {
    ensure!(
        connection.source != "remote-control",
        "Remote-control connections require an authorized device transport"
    );
    if connection.source == "wsl" {
        let distro = connection
            .hostname
            .as_deref()
            .context("WSL distribution is missing")?;
        let mut command = Command::new("wsl.exe");
        command.args([
            "--distribution",
            distro,
            "--exec",
            "sh",
            "-lc",
            "exec codex app-server --stdio",
        ]);
        return Ok(command);
    }
    let mut command = Command::new("ssh");
    command.args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15"]);
    let target = if let Some(alias) = &connection.alias {
        alias
    } else {
        if let Some(port) = connection.ssh_port {
            command.arg("-p").arg(port.to_string());
        }
        if let Some(identity) = &connection.identity {
            command.arg("-i").arg(identity);
        }
        connection
            .hostname
            .as_ref()
            .context("SSH hostname is missing")?
    };
    ensure!(
        !target.is_empty() && !target.starts_with('-') && !target.contains(['\n', '\r', '\0']),
        "Invalid SSH target"
    );
    command
        .arg("--")
        .arg(target)
        .arg("exec sh -lc 'exec codex app-server --stdio'");
    Ok(command)
}

pub fn discover_wsl() -> Result<Vec<Connection>> {
    if !cfg!(target_os = "windows") {
        return Ok(Vec::new());
    }
    let output = match Command::new("wsl.exe").args(["--list", "--quiet"]).output() {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).context("Could not discover WSL connections"),
    };
    if !output.status.success() {
        bail!("Could not discover WSL connections");
    }
    let words: Vec<u16> = output
        .stdout
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| u16::from_le_bytes(*bytes))
        .collect();
    Ok(String::from_utf16_lossy(&words)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| Connection {
            host_id: format!("wsl:{name}"),
            display_name: format!("WSL: {name}"),
            source: "wsl".into(),
            hostname: Some(name.into()),
            ..Default::default()
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ssh_discovery_follows_includes_without_cycles_or_patterns() {
        let temp = tempfile::tempdir().unwrap();
        let ssh = temp.path().join(".ssh");
        fs::create_dir(&ssh).unwrap();
        fs::write(
            ssh.join("config"),
            "Include more-*\nHost * !nope 'build host' other\nHost colima\n",
        )
        .unwrap();
        fs::write(
            ssh.join("more-hosts"),
            "Host deploy\nInclude config\nHost github.com\n",
        )
        .unwrap();
        assert_eq!(
            discover_ssh(temp.path()).unwrap(),
            vec!["build host", "deploy", "github.com"]
        );
    }
    #[test]
    fn saved_connection_updates_preserve_unrelated_global_state() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join(".codex-global-state.json"),
            "{\"unrelated\":7}",
        )
        .unwrap();
        let host = LocalHost::new(temp.path());
        let draft = SshDraft {
            display_name: "Build".into(),
            hostname: "user@example.invalid".into(),
            port: "2222".into(),
            color: Some(Some("#0285ff".into())),
            ..Default::default()
        };
        let saved = host.save_connection(&draft).unwrap();
        host.set_connection_auto_connect(&saved.host_id, true)
            .unwrap();
        let rows = host.connections().unwrap();
        assert_eq!(rows[0].ssh_port, Some(2222));
        assert_eq!(rows[0].color.as_deref(), Some("#0285ff"));
        let path = temp.path().join(".codex-global-state.json");
        let mut state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        state[SAVED][0]["connectionAnalyticsId"] = json!("retained");
        fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
        let mut edit = draft.clone();
        edit.host_id = Some(saved.host_id.clone());
        edit.display_name = "Renamed".into();
        edit.color = None;
        host.save_connection(&edit).unwrap();
        let state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(state[SAVED][0]["connectionAnalyticsId"], "retained");
        assert!(rows[0].auto_connect);
        assert_eq!(
            host.connections().unwrap()[0].color.as_deref(),
            Some("#0285ff")
        );
        edit.color = Some(None);
        host.save_connection(&edit).unwrap();
        let reset = host.connections().unwrap();
        assert_eq!(reset[0].color, None);
        assert!(reset[0].auto_connect);
        host.delete_connection(&saved.host_id).unwrap();
        assert!(host.connections().unwrap().is_empty());
        let state: Value = serde_json::from_slice(
            &fs::read(temp.path().join(".codex-global-state.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(state["unrelated"], 7);
    }

    #[test]
    fn stored_ssh_options_cannot_become_command_arguments() {
        let connection = Connection {
            source: "discovered".into(),
            alias: Some("-oProxyCommand=bad".into()),
            ..Default::default()
        };
        assert!(ssh_command(&connection).is_err());
    }
}
