//! Desktop host services, separate from the `codex app-server` JSON-RPC connection.
//!
//! `ReplayHost` matches the complete recorded request, including HTTP headers and identity.
//! It returns the original wire response, including errors and binary responses.
//! `LocalHost` reads legacy definitions from `automations/<id>/automation.toml` and local
//! run metadata from `sqlite/codex.db` under `CODEX_HOME` (default: `~/.codex`).
//! It never migrates the original database or reads another account's automation rows.
//! Definition writes are atomic; scheduling and execution remain the original host's job.
//! `CloudHost` adds the original's `httpFetch` service for ChatGPT backend requests (usage,
//! beacons, hosted automations, suggested-task conversations): it consumes the renderer's
//! `X-OpenAI-Attach-*` headers, attaches the token that `codex app-server` holds
//! (`getAuthStatus` with `includeToken`) plus the account route from `account/read`, retries once
//! after a 401 with a refreshed token, and answers the original wire shapes, including errors.
//! The token never reaches logs, errors, or `Debug` output.
//! Local fetches support `list-automations`, `inbox-items`, and `paths-exist`; other endpoints return errors.
//! `Root` loads the automation list off the UI thread when the Scheduled route opens.
//! Consumers read the result through `Root::automations`; this service slice does not change the static screens.
//!
//! Storage evidence: Codex 26.1007.30912, `src-BJv5Mk7n.js` functions `jF`, `NF`, `QF`,
//! `Z$`, `Q$`, and `$$`. HTTP evidence: `main-CqEoQDOU.js` `performDesktopFetch`, `fetchHttp`,
//! `A_`, `j_`, `M_`; `bootstrap-CooIJpZm.js` `on`, `rn`, `Xt`; `src-BJv5Mk7n.js` `Kn`, `qn`,
//! `getAuthToken`. The checked-in recording retains the original oracle responses.

use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use bridge::Bridge;
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use ui::screens::sidebar::Project;
use url::Url;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "transport", content = "request", rename_all = "kebab-case")]
pub enum Request {
    Fetch {
        method: String,
        url: String,
        body: String,
    },
    Http(Value),
}

impl Request {
    pub fn post(endpoint: &str, params: Option<Value>) -> Self {
        Self::Fetch {
            method: "POST".into(),
            url: format!("vscode://codex/{endpoint}"),
            body: params.map_or_else(String::new, |p| p.to_string()),
        }
    }
}

pub trait Host: Send + Sync {
    /// Returns the original wire response. Call this off the UI thread for a local host.
    fn fetch(&self, request: &Request) -> Result<Value>;

    /// Reads the original sidebar project state off the UI thread.
    fn projects(&self) -> Result<Vec<Project>> {
        bail!("sidebar project state is unsupported by this host")
    }

    /// The signed-in ChatGPT identity the renderer puts on its backend requests; `None`
    /// without a ChatGPT sign-in. Call this off the UI thread for a cloud host.
    fn identity(&self) -> Option<Identity> {
        None
    }
}

/// What the webview client sends with a ChatGPT backend request: the account and user of
/// the session (`expectedIdentity`, `ChatGPT-Account-ID`) and the device id (`oai-did`).
#[derive(Clone, Debug, PartialEq)]
pub struct Identity {
    pub account_id: String,
    pub user_id: String,
    pub device_id: String,
}

/// The webview client's models read that starts a suggested task's guided conversation
/// (`conversation-actions` `J`: `query.getOrFetch` of the TPP models before the conversation
/// is prepared), as recorded in `refs/interactions/s035b.json`.
pub fn tpp_models(identity: &Identity) -> Request {
    Request::Http(json!({
        "url": "/tpp/models/?iim=false&include_icons=false",
        "method": "GET",
        "headers": {
            "x-openai-web-frontend": "codex_webview",
            "x-oai-mcp-form-version": "2",
            "x-openai-codex-window-type": "electron",
            "OAI-Language": "en-US",
            "oai-did": identity.device_id,
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Desktop-Surface": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
            "ChatGPT-Account-ID": identity.account_id,
        },
        "expectedIdentity": {"accountId": identity.account_id, "userId": identity.user_id},
        "returnErrorResponse": true,
    }))
}

/// The Scheduled page's read-only suggestion request.
pub fn suggested_automations() -> Request {
    Request::Http(json!({
        "url": "/suggested_automations",
        "method": "GET",
        "headers": {
            "OAI-Language": "en-US",
            "X-OpenAI-Attach-Auth": "1",
            "X-OpenAI-Attach-Integrity-State": "1",
            "originator": "Codex Desktop",
            "X-OpenAI-Task-Suggestions-Surface": "scheduled_page",
        },
        "returnErrorResponse": true,
    }))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Exchange {
    pub request: Request,
    pub response: Value,
}

pub struct ReplayHost {
    exchanges: Vec<Exchange>,
    project_snapshot: Option<ProjectSnapshot>,
}

impl ReplayHost {
    pub fn from_recording(recording: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct Recording {
            exchanges: Vec<Exchange>,
            #[serde(default)]
            project_snapshot: Option<ProjectSnapshot>,
        }
        let recording: Recording = serde_json::from_str(recording)?;
        Ok(Self {
            exchanges: recording.exchanges,
            project_snapshot: recording.project_snapshot,
        })
    }

    pub fn from_oracle(oracle: &str) -> Result<Self> {
        let oracle: serde_json::Map<String, Value> = serde_json::from_str(oracle)?;
        let mut exchanges = Vec::new();
        for (key, response) in oracle {
            let request = if let Some(key) = key.strip_prefix("fetch ") {
                let mut parts = key.splitn(3, ' ');
                Request::Fetch {
                    method: parts.next().context("missing method")?.into(),
                    url: parts.next().context("missing URL")?.into(),
                    body: serde_json::from_str(parts.next().context("missing body")?)?,
                }
            } else if let Some(key) = key.strip_prefix("svc httpFetch.fetch ") {
                let args: Vec<Value> = serde_json::from_str(key)?;
                Request::Http(args.get(1).context("missing HTTP request")?.clone())
            } else {
                continue;
            };
            exchanges.push(Exchange { request, response });
        }
        Ok(Self {
            exchanges,
            project_snapshot: None,
        })
    }
}

impl Host for ReplayHost {
    fn fetch(&self, request: &Request) -> Result<Value> {
        self.exchanges
            .iter()
            .find(|e| e.request == *request)
            .map(|e| e.response.clone())
            .context("no recorded host response for this exact request")
    }

    fn projects(&self) -> Result<Vec<Project>> {
        self.project_snapshot
            .as_ref()
            .context("no recorded sidebar project snapshot")?
            .rows()
    }

    /// The identity of the first recorded request that carries one.
    fn identity(&self) -> Option<Identity> {
        self.exchanges.iter().find_map(|e| {
            let Request::Http(http) = &e.request else {
                return None;
            };
            Some(Identity {
                account_id: http["expectedIdentity"]["accountId"].as_str()?.to_owned(),
                user_id: http["expectedIdentity"]["userId"].as_str()?.to_owned(),
                device_id: http["headers"]["oai-did"].as_str()?.to_owned(),
            })
        })
    }
}

#[derive(Deserialize)]
struct ProjectSnapshot {
    state: Value,
    codex_home: PathBuf,
    workspace_roots: Vec<String>,
    existing_paths: Option<Vec<String>>,
}

impl ProjectSnapshot {
    fn rows(&self) -> Result<Vec<Project>> {
        let existing = self.existing_paths.as_ref().map(|paths| {
            paths
                .iter()
                .map(|path| {
                    normalized_project_path(path)
                        .trim_end_matches('/')
                        .to_owned()
                })
                .collect::<HashSet<_>>()
        });
        project_rows(&self.state, &self.codex_home, |path, normalized| {
            Ok(existing.as_ref().map(|existing| {
                self.workspace_roots.iter().any(|root| root == path)
                    && existing.contains(normalized.trim_end_matches('/'))
            }))
        })
    }
}

fn project_rows(
    state: &Value,
    codex_home: &Path,
    mut exists: impl FnMut(&str, &str) -> Result<Option<bool>>,
) -> Result<Vec<Project>> {
    let Some(projects) = state["local-projects"].as_object() else {
        return Ok(Vec::new());
    };
    let managed_root = normalized_project_path(&codex_home.join("worktrees").to_string_lossy());
    let mut rows = Vec::with_capacity(projects.len());
    for stored in projects.values() {
        let (Some(id), Some(name), Some(roots)) = (
            stored["id"].as_str(),
            stored["name"].as_str(),
            stored["rootPaths"].as_array(),
        ) else {
            continue;
        };
        if !stored["createdAt"].is_number()
            || !stored["updatedAt"].is_number()
            || roots.iter().any(|root| !root.is_string())
        {
            continue;
        }
        let root = roots.first().and_then(Value::as_str);
        let normalized = root.map(normalized_project_path);
        let worktree = root
            .zip(normalized.as_deref())
            .is_some_and(|(path, normalized)| {
                is_worktree_path(normalized, &managed_root, is_windows_path(path))
            });
        let missing = match root.zip(normalized.as_deref()) {
            Some((path, normalized)) => exists(path, normalized)?.is_some_and(|exists| !exists),
            None => false,
        };
        let label = if name.trim().is_empty() {
            id
        } else {
            name.trim()
        };
        let expanded_key = format!("sidebar-project-expanded-v1-codex:{id}");
        rows.push(Project {
            id: id.to_owned().into(),
            label: label.to_owned().into(),
            new_chat_label: if worktree {
                "New chat in this worktree".into()
            } else {
                format!("Start new chat in {label}").into()
            },
            worktree,
            missing,
            expanded: state["electron-persisted-atom-state"][&expanded_key] != false,
        });
    }
    if let Some(order) = state["project-order"].as_array() {
        let ranks: HashMap<_, _> = order
            .iter()
            .enumerate()
            .filter_map(|(rank, id)| id.as_str().map(|id| (id, rank)))
            .collect();
        rows.sort_by_key(|project| {
            ranks
                .get(project.id.as_ref())
                .copied()
                .unwrap_or(usize::MAX)
        });
    }
    Ok(rows)
}

fn is_windows_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with("//")
        || path.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
}

fn normalized_project_path(path: &str) -> String {
    let windows = is_windows_path(path);
    let path = if path.contains('\\') {
        Cow::Owned(path.replace('\\', "/"))
    } else {
        Cow::Borrowed(path)
    };
    let mut normalized = PathBuf::with_capacity(path.len());
    for component in Path::new(path.as_ref()).components() {
        match component {
            Component::ParentDir if normalized.file_name().is_some_and(|name| name != "..") => {
                normalized.pop();
            }
            Component::ParentDir if !normalized.has_root() => normalized.push(".."),
            Component::ParentDir | Component::CurDir => {}
            component => normalized.push(component),
        }
    }
    let mut normalized = normalized
        .into_os_string()
        .into_string()
        .expect("project paths are UTF-8");
    if windows {
        normalized.make_ascii_lowercase();
    }
    normalized
}

fn is_worktree_path(path: &str, managed_root: &str, windows: bool) -> bool {
    if path
        .strip_prefix(managed_root)
        .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('/'))
    {
        return true;
    }
    let workspace_root = if windows {
        "/openai/codex/workspaces"
    } else {
        "/OpenAI/Codex/workspaces"
    };
    path.match_indices("/worktrees").any(|(index, _)| {
        let tail = &path[index + "/worktrees".len()..];
        if !tail.is_empty() && !tail.starts_with('/') {
            return false;
        }
        let prefix = &path[..index];
        prefix.ends_with("/.codex")
            || prefix.ends_with("/.codex-workspaces")
            || prefix.ends_with(workspace_root)
            || prefix
                .rsplit_once("/instances/")
                .is_some_and(|(root, instance)| {
                    !instance.is_empty()
                        && !instance.contains('/')
                        && (root.ends_with("/.codex-workspaces") || root.ends_with(workspace_root))
                })
    })
}

/// Decodes a JSON response without turning HTTP errors or binary bodies into empty data.
pub fn json_body(response: &Value) -> Result<Value> {
    let response = response.get("value").unwrap_or(response);
    if response["responseType"] == "error" {
        bail!("host error {}: {}", response["status"], response["error"]);
    }
    let response = response.pointer("/response/__response").unwrap_or(response);
    let status = response["status"].as_u64().context("missing host status")?;
    ensure!((200..300).contains(&status), "host HTTP status {status}");
    let body = response
        .get("bodyJsonString")
        .or_else(|| response.get("body"))
        .and_then(Value::as_str)
        .context("host response has no JSON body")?;
    Ok(serde_json::from_str(body)?)
}

pub struct LocalHost {
    home: PathBuf,
}

impl LocalHost {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    pub fn from_env() -> Self {
        Self::new(
            std::env::var_os("CODEX_HOME")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".codex")
                }),
        )
    }

    fn database(&self, flags: OpenFlags) -> Result<Option<Connection>> {
        let path = self.home.join("sqlite/codex.db");
        match fs::metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
            Ok(_) => Ok(Some(Connection::open_with_flags(path, flags)?)),
        }
    }

    fn automations(&self) -> Result<Value> {
        let directory = self.home.join("automations");
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({"items": []})),
            Err(e) => return Err(e.into()),
        };
        let db = self.database(OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut items = Vec::new();
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path().join("automation.toml");
            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e).with_context(|| path.display().to_string()),
            };
            // The original skips malformed definitions and directory/id mismatches.
            let Ok(mut item) = definition(&id, &text) else {
                continue;
            };
            if item["status"] == "DELETED" {
                continue;
            }
            if let Some(db) = &db {
                let cached = db
                    .query_row(
                        "SELECT next_run_at, next_run_nominal_at, last_run_at, rrule, status,
                     account_id, user_id, installation_id FROM automations WHERE id = ?",
                        [&id],
                        |row| {
                            Ok((
                                row.get::<_, Option<i64>>(0)?,
                                row.get::<_, Option<i64>>(1)?,
                                row.get::<_, Option<i64>>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, Option<String>>(5)?,
                                row.get::<_, Option<String>>(6)?,
                                row.get::<_, Option<String>>(7)?,
                            ))
                        },
                    )
                    .optional()?;
                if let Some((next, nominal, last, rrule, status, account, user, installation)) =
                    cached
                {
                    if account.is_some() || user.is_some() || installation.is_some() {
                        continue;
                    }
                    item["lastRunAt"] = json!(last);
                    if item["status"] == "ACTIVE"
                        && item["status"] == status
                        && item["rrule"] == rrule
                    {
                        item["nextRunAt"] = json!(next);
                        item["nextRunNominalAt"] = json!(nominal);
                    }
                }
            }
            items.push(item);
        }
        items.sort_by_key(|v| std::cmp::Reverse(v["updatedAt"].as_i64().unwrap_or_default()));
        Ok(json!({"items": items}))
    }

    /// Saves a validated original-format definition, not an automation scheduling command.
    /// Existing schedule metadata remains in the original database for its scheduler to reconcile.
    pub fn write_automation(&self, id: &str, contents: &str) -> Result<()> {
        definition(id, contents)?;
        if let Some(db) = self.database(OpenFlags::SQLITE_OPEN_READ_ONLY)? {
            let owned: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM automations WHERE id = ? AND
                 (account_id IS NOT NULL OR user_id IS NOT NULL OR installation_id IS NOT NULL))",
                [id],
                |row| row.get(0),
            )?;
            ensure!(
                !owned,
                "cannot write an account-owned automation through LocalHost"
            );
        }
        let root = self.home.join("automations");
        fs::create_dir_all(&root)?;
        let root = root.canonicalize()?;
        ensure!(
            root.starts_with(self.home.canonicalize()?),
            "automations root escapes CODEX_HOME"
        );
        let directory = root.join(id);
        fs::create_dir_all(&directory)?;
        ensure!(
            directory.canonicalize()?.starts_with(&root),
            "automation directory escapes CODEX_HOME"
        );
        let mut file = tempfile::NamedTempFile::new_in(&directory)?;
        file.write_all(contents.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(directory.join("automation.toml"))?;
        Ok(())
    }

    /// Applies the original inbox read-state update, restricted to local, unowned automations.
    pub fn mark_inbox_read(&self, thread_id: &str, read_at: Option<i64>) -> Result<bool> {
        let Some(db) = self.database(OpenFlags::SQLITE_OPEN_READ_WRITE)? else {
            return Ok(false);
        };
        Ok(db.execute(
            "UPDATE automation_runs SET read_at = ? WHERE thread_id = ? AND automation_id IN
             (SELECT id FROM automations WHERE account_id IS NULL AND user_id IS NULL AND installation_id IS NULL)",
            params![read_at, thread_id],
        )? > 0)
    }

    fn inbox(&self, params: &Value) -> Result<Value> {
        let limit = match params.get("limit").filter(|v| !v.is_null()) {
            Some(limit) => limit
                .as_i64()
                .filter(|n| *n >= 0)
                .context("invalid inbox limit")?,
            None => 200,
        };
        let thread = params
            .get("threadId")
            .filter(|v| !v.is_null())
            .map(|v| v.as_str().context("invalid inbox threadId"))
            .transpose()?;
        let Some(db) = self.database(OpenFlags::SQLITE_OPEN_READ_ONLY)? else {
            return Ok(
                json!({"items": [], "unreadRunCounts": {"total": 0, "automationIds": [], "unreadRuns": []}}),
            );
        };
        let mut query = db.prepare(
            "SELECT r.thread_id, r.automation_id, a.name,
             COALESCE(a.name, NULLIF(r.inbox_title, ''), r.thread_title),
             COALESCE(NULLIF(r.inbox_summary, ''), r.archived_assistant_message, r.archived_user_message, a.prompt),
             r.archived_assistant_message, r.archived_user_message, r.archived_reason,
             r.source_cwd, r.read_at, r.created_at, r.status
             FROM automation_runs r JOIN automations a ON a.id = r.automation_id
             WHERE a.account_id IS NULL AND a.user_id IS NULL AND a.installation_id IS NULL
             AND r.status IN ('IN_PROGRESS', 'PENDING_REVIEW', 'ACCEPTED', 'ARCHIVED')
             AND (?1 IS NULL OR r.thread_id = ?1)
             ORDER BY r.status = 'IN_PROGRESS' DESC, r.status = 'PENDING_REVIEW' DESC, r.created_at DESC LIMIT ?2")?;
        let items = query.query_map(params![thread, limit], |r| Ok(json!({
            "id": r.get::<_, String>(0)?, "threadId": r.get::<_, String>(0)?,
            "automationId": r.get::<_, String>(1)?, "automationName": r.get::<_, Option<String>>(2)?,
            "title": r.get::<_, Option<String>>(3)?, "description": r.get::<_, Option<String>>(4)?,
            "archivedAssistantMessage": r.get::<_, Option<String>>(5)?,
            "archivedUserMessage": r.get::<_, Option<String>>(6)?, "archivedReason": r.get::<_, Option<String>>(7)?,
            "sourceCwd": r.get::<_, Option<String>>(8)?, "readAt": r.get::<_, Option<i64>>(9)?,
            "createdAt": r.get::<_, i64>(10)?, "status": r.get::<_, String>(11)?
        })))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut unread = db.prepare(
            "SELECT r.automation_id, r.thread_id FROM automation_runs r JOIN automations a ON a.id = r.automation_id
             WHERE r.read_at IS NULL AND r.status IN ('PENDING_REVIEW', 'ACCEPTED')
             AND a.account_id IS NULL AND a.user_id IS NULL AND a.installation_id IS NULL")?;
        let runs = unread.query_map([], |r| Ok(json!({"automationId": r.get::<_, String>(0)?, "threadId": r.get::<_, String>(1)?})))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut ids = Vec::new();
        for run in &runs {
            if !ids.contains(&run["automationId"]) {
                ids.push(run["automationId"].clone());
            }
        }
        Ok(
            json!({"items": items, "unreadRunCounts": {"total": runs.len(), "automationIds": ids, "unreadRuns": runs}}),
        )
    }
}

impl Host for LocalHost {
    fn projects(&self) -> Result<Vec<Project>> {
        let path = self.home.join(".codex-global-state.json");
        let contents = match fs::read(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("read original sidebar project state"),
        };
        let state: Value =
            serde_json::from_slice(&contents).context("decode original sidebar project state")?;
        ensure!(
            state.is_object(),
            "original sidebar project state must be an object"
        );
        project_rows(&state, &self.home, |path, _| {
            Ok(Some(Path::new(path).try_exists()?))
        })
    }

    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Fetch { method, url, body } = request else {
            bail!("authenticated remote HTTP is unsupported by LocalHost");
        };
        ensure!(method == "POST", "unsupported host method: {method}");
        let endpoint = url
            .strip_prefix("vscode://codex/")
            .context("unsupported host URL")?;
        let params: Value = if body.is_empty() {
            json!({})
        } else {
            serde_json::from_str(body)?
        };
        ensure!(params.is_object(), "host parameters must be an object");
        ensure!(
            params.get("hostId").is_none_or(|id| id == "local"),
            "remote host is unsupported by LocalHost"
        );
        let body = match endpoint {
            "list-automations" => self.automations()?,
            "inbox-items" => self.inbox(&params)?,
            "paths-exist" => {
                let paths: Vec<String> =
                    serde_json::from_value(params.get("paths").cloned().unwrap_or(json!([])))?;
                let mut existing = Vec::new();
                for path in paths {
                    if Path::new(&path).try_exists()? {
                        existing.push(path);
                    }
                }
                json!({"existingPaths": existing})
            }
            _ => bail!("unsupported local host endpoint: {endpoint}"),
        };
        Ok(
            json!({"type": "fetch-response", "responseType": "success", "status": 200,
            "headers": {"content-type": "application/json"}, "bodyJsonString": body.to_string()}),
        )
    }
}

/// The sign-in the cloud transport attaches: the ChatGPT token and the workspace route.
#[derive(Clone)]
pub struct Credentials {
    pub token: String,
    /// `account/read.workspaceRouting.backendOrigin`; `None` keeps the configured base URL.
    pub backend_origin: Option<String>,
    /// `accountRoutingOverride` unless it is `NO_CONSTRAINT`.
    pub routing_override: Option<String>,
}

/// Loads the sign-in; `true` asks for a refreshed token. `None` means no ChatGPT sign-in.
pub type CredentialSource = Box<dyn Fn(bool) -> Result<Option<Credentials>> + Send + Sync>;

/// The token and workspace route that `codex app-server` holds, read like the original's
/// `getAuthToken` (`getAuthStatus` with `includeToken`) and `getBackendRequestAuth`
/// (`account/read`). API-key and signed-out accounts yield `None`.
pub fn app_server_credentials(bridge: Arc<Bridge>) -> CredentialSource {
    Box::new(move |refresh| {
        pollster::block_on(async {
            let params = json!({"includeToken": true, "refreshToken": refresh});
            let status: Value = bridge.request("getAuthStatus", params).await?;
            if !matches!(
                status["authMethod"].as_str(),
                Some("chatgpt" | "chatgptAuthTokens")
            ) {
                return Ok(None);
            }
            let Some(token) = status["authToken"].as_str() else {
                return Ok(None);
            };
            let account: Value = bridge
                .request("account/read", json!({"refreshToken": false}))
                .await?;
            let routing = &account["workspaceRouting"];
            Ok(Some(Credentials {
                token: token.to_owned(),
                backend_origin: routing["backendOrigin"].as_str().map(str::to_owned),
                routing_override: routing["accountRoutingOverride"]
                    .as_str()
                    .filter(|o| *o != "NO_CONSTRAINT")
                    .map(str::to_owned),
            }))
        })
    })
}

type Headers = Vec<(String, String)>;
/// An `httpFetch` failure the original reports as an error response: `(status, message)`.
type Refusal = (u16, String);

/// `LocalHost` plus the original's `httpFetch` service for the ChatGPT backend.
pub struct CloudHost {
    local: LocalHost,
    credentials: CredentialSource,
    cached: Mutex<Option<Credentials>>,
    base_url: String,
    // ponytail: in memory; the original persists it in global state across runs.
    integrity_state: Mutex<Option<String>>,
    // ponytail: a fresh `oai-did` per process; the original persists the device id.
    device_id: String,
    agent: ureq::Agent,
}

impl CloudHost {
    /// `base_url` is the ChatGPT backend root, like `https://chatgpt.com/backend-api`.
    pub fn new(
        local: LocalHost,
        credentials: CredentialSource,
        base_url: impl Into<String>,
    ) -> Self {
        // No overall deadline, like `fetchHttp`: a suggested-task conversation start streams
        // its answer, and the reply is only complete once the stream closes.
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .build()
            .into();
        Self {
            local,
            credentials,
            cached: Mutex::new(None),
            base_url: base_url.into(),
            integrity_state: Mutex::new(None),
            device_id: device_id(),
            agent,
        }
    }

    /// `$CODEX_API_BASE_URL`, else the production backend, like the original's `$t`.
    pub fn from_env(credentials: CredentialSource) -> Self {
        let base = std::env::var("CODEX_API_BASE_URL")
            .ok()
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| "https://chatgpt.com/backend-api".into());
        Self::new(LocalHost::from_env(), credentials, base)
    }

    /// Cached sign-in, reloaded on `refresh`. Failures carry the original's messages only.
    fn credentials(&self, refresh: bool) -> std::result::Result<Credentials, Refusal> {
        let mut cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        if !refresh && let Some(credentials) = cached.as_ref() {
            return Ok(credentials.clone());
        }
        let loaded = (self.credentials)(refresh).map_err(|_| {
            if refresh {
                (401, "Failed to refresh authentication token".to_owned())
            } else {
                (432, "Failed to retrieve authentication token".to_owned())
            }
        })?;
        cached.clone_from(&loaded);
        loaded.ok_or_else(|| (500, "No authenticated workspace is available".to_owned()))
    }

    /// The original's `fetchHttp`: wire problems become error responses, not `Err`.
    fn http(&self, request: &Value) -> Result<Value> {
        let url = request["url"].as_str().context("HTTP request has no url")?;
        let value = match self.send(url, request) {
            Ok(value) => value,
            Err((status, error)) => json!({
                "type": "fetch-response", "responseType": "error",
                "status": status, "error": error, "responseStatus": null
            }),
        };
        Ok(json!({"value": value, "fn": false, "calls": []}))
    }

    /// The original's `performDesktopFetch` for one request, with its single 401 retry.
    fn send(&self, url: &str, request: &Value) -> std::result::Result<Value, Refusal> {
        let method = request["method"]
            .as_str()
            .unwrap_or("GET")
            .to_ascii_uppercase();
        let body = request["body"].as_str().map(str::to_owned);
        let mut headers: Headers = request["headers"]
            .as_object()
            .map(|h| {
                h.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        let resolved = if url.starts_with("http://") || url.starts_with("https://") {
            url.to_owned()
        } else {
            format!(
                "{}/{}",
                self.base_url.trim_end_matches('/'),
                url.trim_start_matches('/')
            )
        };
        let target = Url::parse(&resolved).map_err(|e| (400, format!("Invalid URL: {e}")))?;
        let attach_auth = take_flag(&mut headers, "X-OpenAI-Attach-Auth");
        let surface = take_flag(&mut headers, "X-OpenAI-Attach-Desktop-Surface");
        let device_check = take_flag(&mut headers, "X-OpenAI-Attach-DeviceCheck-Token");
        let integrity = take_flag(&mut headers, "X-OpenAI-Attach-Integrity-State");
        let expected_account = take_header(&mut headers, "X-OpenAI-Expected-Account-Id");
        take_header(&mut headers, "X-OpenAI-Account-Routing-Override");
        let allowed = auth_allowed(&target, &self.base_url);
        let attach_auth = (attach_auth || (allowed && infers_auth(target.path())))
            && !has_header(&headers, "Authorization");
        let refusals = [
            (
                attach_auth && !allowed,
                "Refusing to attach authentication to non-OpenAI URL",
            ),
            (
                surface && !allowed,
                "Refusing to attach desktop surface to non-OpenAI URL",
            ),
            (
                device_check,
                "DeviceCheck tokens are unavailable on this host",
            ),
            (
                integrity && !allowed,
                "Refusing to attach integrity state to non-OpenAI URL",
            ),
        ];
        if let Some((_, message)) = refusals.into_iter().find(|(refused, _)| *refused) {
            return Err((400, message.into()));
        }
        if let Some(body) = &body
            && !has_header(&headers, "Content-Type")
            && body.trim_start().starts_with(['{', '['])
            && serde_json::from_str::<Value>(body).is_ok()
        {
            set_header(&mut headers, "Content-Type", "application/json");
        }
        if surface {
            if !has_header(&headers, "originator") {
                set_header(&mut headers, "originator", "Codex Desktop");
            }
            if !has_header(&headers, "User-Agent") {
                set_header(&mut headers, "User-Agent", user_agent());
            }
        }
        let return_error_response = request["returnErrorResponse"] == true;
        let expected_identity = request.get("expectedIdentity").filter(|v| v.is_object());
        let mut retry = request["retry"] != "never";
        let mut credentials = attach_auth.then(|| self.credentials(false)).transpose()?;
        loop {
            let mut headers = headers.clone();
            let mut target = target.clone();
            if let Some(credentials) = &credentials {
                let claims = claims(&credentials.token);
                if let Some(expected) = &expected_account
                    && claims.account_id.as_deref() != Some(expected)
                {
                    return Err((
                        409,
                        "The authenticated account changed before the request could be sent."
                            .into(),
                    ));
                }
                if let Some(identity) = expected_identity
                    && (identity["accountId"].as_str() != claims.principal_account.as_deref()
                        || identity["userId"].as_str() != claims.user_id.as_deref())
                {
                    return Err((499, "Authenticated principal changed".into()));
                }
                set_header(
                    &mut headers,
                    "Authorization",
                    format!("Bearer {}", credentials.token),
                );
                if let Some(account) = &claims.account_id {
                    set_header(&mut headers, "ChatGPT-Account-Id", account);
                }
                if claims.fedramp {
                    set_header(&mut headers, "X-OpenAI-Fedramp", "true");
                } else {
                    take_header(&mut headers, "X-OpenAI-Fedramp");
                }
                if let Some(origin) = &credentials.backend_origin {
                    let query = target.query().map(|q| format!("?{q}")).unwrap_or_default();
                    target = Url::parse(&format!("{origin}{}{query}", target.path()))
                        .map_err(|e| (400, format!("Invalid workspace route: {e}")))?;
                }
                if let Some(route) = &credentials.routing_override {
                    set_header(&mut headers, "X-OpenAI-Account-Routing-Override", route);
                }
            }
            let state = integrity
                .then(|| {
                    self.integrity_state
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone()
                })
                .flatten();
            if let Some(state) = &state {
                set_header(&mut headers, "X-OAI-IS", state);
            }
            let mut builder = ureq::http::Request::builder()
                .method(method.as_str())
                .uri(target.as_str());
            for (name, value) in &headers {
                builder = builder.header(name.as_str(), value.as_str());
            }
            let bad = |e: ureq::http::Error| (400, e.to_string());
            let response = match &body {
                Some(body) => self.agent.run(builder.body(body.clone()).map_err(bad)?),
                None => self.agent.run(builder.body(()).map_err(bad)?),
            }
            .map_err(|e| (500, e.to_string()))?;
            if integrity {
                self.store_integrity_update(&response, state.as_deref());
            }
            if response.status() == 401 && credentials.is_some() && retry {
                retry = false;
                credentials = Some(self.credentials(true)?);
                continue;
            }
            return wire(response, return_error_response);
        }
    }

    /// Keeps `X-OAI-IS-Update` only when the state it answers is the one this host sent.
    fn store_integrity_update(
        &self,
        response: &ureq::http::Response<ureq::Body>,
        sent: Option<&str>,
    ) {
        let Some(next) = response
            .headers()
            .get("x-oai-is-update")
            .and_then(|v| v.to_str().ok())
            .filter(|s| valid_integrity_state(s))
        else {
            return;
        };
        let mut current = self
            .integrity_state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if current.as_deref().filter(|s| valid_integrity_state(s)) == sent {
            *current = Some(next.to_owned());
        }
    }
}

impl Host for CloudHost {
    fn projects(&self) -> Result<Vec<Project>> {
        self.local.projects()
    }

    fn fetch(&self, request: &Request) -> Result<Value> {
        match request {
            Request::Http(http) => self.http(http),
            Request::Fetch { .. } => self.local.fetch(request),
        }
    }

    fn identity(&self) -> Option<Identity> {
        let claims = claims(&self.credentials(false).ok()?.token);
        Some(Identity {
            account_id: claims.principal_account?,
            user_id: claims.user_id?,
            device_id: self.device_id.clone(),
        })
    }
}

/// A random v4 UUID from the standard library's hasher keys.
fn device_id() -> String {
    use std::hash::{BuildHasher as _, RandomState};
    let a = (RandomState::new().hash_one(0u8) & !0xf000) | 0x4000;
    let b = (RandomState::new().hash_one(1u8) & !(0xc << 60)) | (0x8 << 60);
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0xffff,
        b >> 48,
        b & 0xffff_ffff_ffff
    )
}

/// The recorded wire shape: `__response` for deliverable responses, else the error object.
fn wire(
    mut response: ureq::http::Response<ureq::Body>,
    return_error_response: bool,
) -> std::result::Result<Value, Refusal> {
    let status = response.status().as_u16();
    let text = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_none_or(|t| {
            ["json", "text", "xml", "javascript"]
                .iter()
                .any(|k| t.contains(k))
        });
    let mut headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    headers.sort();
    let bytes = response
        .body_mut()
        .read_to_vec()
        .map_err(|e| (500, e.to_string()))?;
    if (200..300).contains(&status) || status == 304 || return_error_response {
        let mut wire = json!({"status": status, "statusText": "", "headers": headers});
        if text {
            wire["body"] = String::from_utf8_lossy(&bytes).into_owned().into();
        } else {
            wire["base64"] = STANDARD.encode(&bytes).into();
        }
        return Ok(json!({"response": {"__response": wire}}));
    }
    let error = if bytes.is_empty() {
        format!("Request failed with status {status}")
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok(json!({
        "type": "fetch-response", "responseType": "error",
        "status": status, "error": error, "responseStatus": status
    }))
}

/// The token's `https://api.openai.com/auth` claims (`Kn` and `qn` in the original).
struct Claims {
    /// `chatgpt_account_id`: the `ChatGPT-Account-Id` header and expected-account check.
    account_id: Option<String>,
    /// `chatgpt_account_id`, else `account_id`: the authenticated principal.
    principal_account: Option<String>,
    /// `chatgpt_user_id`, else `user_id`.
    user_id: Option<String>,
    fedramp: bool,
}

fn claims(token: &str) -> Claims {
    let json = token
        .split('.')
        .nth(1)
        .and_then(|payload| URL_SAFE_NO_PAD.decode(payload).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or_default();
    let auth = &json["https://api.openai.com/auth"];
    let text = |key: &str| auth[key].as_str().map(str::to_owned);
    Claims {
        account_id: text("chatgpt_account_id"),
        principal_account: text("chatgpt_account_id").or_else(|| text("account_id")),
        user_id: text("chatgpt_user_id").or_else(|| text("user_id")),
        fedramp: auth["chatgpt_account_is_fedramp"] == true,
    }
}

/// `isDesktopAuthAllowedUrl`: OpenAI hosts, the configured backend, and local development.
fn auth_allowed(url: &Url, base_url: &str) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    if host == "localhost" && matches!(url.port(), None | Some(8000)) {
        return true;
    }
    if host == "status.openai.com" {
        return false;
    }
    host == "openai.com"
        || host.ends_with(".openai.com")
        || Url::parse(base_url).is_ok_and(|base| base.origin() == url.origin())
        || host == "chatgpt.com"
        || (host.ends_with(".chatgpt.com") && !host.starts_with("ab."))
}

/// `shouldInferCodexApiAuth`: Codex API paths are authenticated without the attach header.
fn infers_auth(path: &str) -> bool {
    let path = path.trim_end_matches('/');
    [
        "/wham",
        "/api/wham",
        "/backend-api/wham",
        "/backend-api/estuary/content",
    ]
    .iter()
    .any(|root| path == *root || path.starts_with(&format!("{root}/")))
}

fn valid_integrity_state(state: &str) -> bool {
    let parts: Vec<&str> = state.split('.').collect();
    state.len() <= 2048
        && parts.len() == 4
        && parts[0] == "ois1"
        && parts[1..].iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

fn user_agent() -> String {
    let os = match std::env::consts::OS {
        "macos" => "Mac OS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    };
    let version = env!("CARGO_PKG_VERSION");
    format!("Codex Desktop/{version} ({os}; {})", std::env::consts::ARCH)
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

fn take_header(headers: &mut Headers, name: &str) -> Option<String> {
    let index = headers
        .iter()
        .position(|(k, _)| k.eq_ignore_ascii_case(name))?;
    Some(headers.remove(index).1)
}

/// The original's `pc`: present and not `0`/`false`.
fn take_flag(headers: &mut Headers, name: &str) -> bool {
    take_header(headers, name).is_some_and(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
}

fn set_header(headers: &mut Headers, name: &str, value: impl Into<String>) {
    take_header(headers, name);
    headers.push((name.to_owned(), value.into()));
}

fn definition(id: &str, contents: &str) -> Result<Value> {
    ensure!(
        !id.is_empty() && id != "." && id != ".." && !id.contains(['/', '\\', '\0']),
        "invalid automation id"
    );
    let raw: Value = toml::from_str(contents)?;
    ensure!(
        raw["id"] == id,
        "automation id does not match its directory"
    );
    ensure!(
        raw.get("version").is_none_or(|v| v == 1),
        "unsupported automation version"
    );
    for key in ["name", "prompt", "status"] {
        ensure!(raw[key].is_string(), "missing automation {key}");
    }
    ensure!(
        ["ACTIVE", "PAUSED", "DELETED"]
            .iter()
            .any(|s| raw["status"] == *s),
        "invalid automation status"
    );
    for key in ["created_at", "updated_at"] {
        ensure!(raw[key].is_i64(), "invalid automation {key}");
    }
    for key in [
        "rrule",
        "model",
        "local_environment_config_path",
        "plugin_template_id",
        "target_thread_id",
    ] {
        ensure!(
            raw.get(key).is_none_or(Value::is_string),
            "invalid automation {key}"
        );
    }
    for (key, allowed) in [
        (
            "reasoning_effort",
            &[
                "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
            ][..],
        ),
        ("execution_environment", &["worktree", "local"][..]),
        ("notification_policy", &["failed_runs_only"][..]),
    ] {
        ensure!(
            raw.get(key)
                .is_none_or(|v| v.as_str().is_some_and(|s| allowed.contains(&s))),
            "invalid automation {key}"
        );
    }
    ensure!(
        raw.get("auto_archive").is_none_or(Value::is_boolean),
        "invalid auto_archive"
    );
    ensure!(
        raw.get("target")
            .is_none_or(|v| v.is_object() && v["type"].is_string()),
        "invalid automation target"
    );
    let trimmed = |key: &str| {
        raw[key]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let mut item = json!({"id": id, "name": raw["name"], "prompt": raw["prompt"], "status": raw["status"],
        "rrule": trimmed("rrule").unwrap_or_else(|| "FREQ=HOURLY;INTERVAL=24;BYMINUTE=0".into()),
        "model": trimmed("model"), "reasoningEffort": raw["reasoning_effort"],
        "createdAt": raw["created_at"], "updatedAt": raw["updated_at"],
        "nextRunAt": null, "nextRunNominalAt": null, "lastRunAt": null});
    if let Some(policy) = raw.get("notification_policy") {
        item["notificationPolicy"] = policy.clone();
    }
    if raw["kind"] == "heartbeat" {
        ensure!(
            trimmed("target_thread_id").is_some(),
            "heartbeat requires a target thread"
        );
        item["kind"] = json!("heartbeat");
        item["targetThreadId"] = raw["target_thread_id"].clone();
    } else {
        ensure!(
            raw.get("kind").is_none_or(|k| k == "cron"),
            "invalid automation kind"
        );
        let cwds: Vec<String> = serde_json::from_value(raw["cwds"].clone())?;
        item["kind"] = json!("cron");
        item["cwds"] = json!(cwds);
        item["pluginTemplateId"] = json!(trimmed("plugin_template_id"));
        item["autoArchive"] = raw.get("auto_archive").cloned().unwrap_or(json!(false));
        item["executionEnvironment"] =
            json!(raw["execution_environment"].as_str().unwrap_or("worktree"));
        item["localEnvironmentConfigPath"] = json!(trimmed("local_environment_config_path"));
        item["target"] = match raw["target"]["type"].as_str() {
            Some("project") => {
                ensure!(
                    raw["target"]["project_id"].is_string(),
                    "missing project id"
                );
                json!({"type": "project", "projectId": raw["target"]["project_id"]})
            }
            Some("projectless") => json!({"type": "projectless"}),
            None => Value::Null,
            _ => bail!("invalid automation target"),
        };
    }
    Ok(item)
}
