//! Desktop host services, separate from the `codex app-server` JSON-RPC connection.
//!
//! `ReplayHost` matches the complete recorded request, including HTTP headers and identity.
//! It returns the original wire response, including errors and binary responses.
//! `LocalHost` reads legacy definitions from `automations/<id>/automation.toml` and local
//! run metadata from `sqlite/codex.db` under `CODEX_HOME` (default: `~/.codex`).
//! It never migrates the original database or reads another account's automation rows.
//! Definition writes are atomic; scheduling and execution remain the original host's job.
//! Authenticated HTTP services (usage, beacons, hosted automations) are explicitly unsupported.
//! Local fetches support `list-automations`, `inbox-items`, and `paths-exist`; other endpoints return errors.
//! `Root` loads the automation list off the UI thread when the Scheduled route opens.
//! Consumers read the result through `Root::automations`; this service slice does not change the static screens.
//!
//! Storage evidence: Codex 26.1007.30912, `src-BJv5Mk7n.js` functions `jF`, `NF`, `QF`,
//! `Z$`, `Q$`, and `$$`. The checked-in recording retains the original oracle responses.

use anyhow::{Context as _, Result, bail, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Exchange {
    pub request: Request,
    pub response: Value,
}

pub struct ReplayHost {
    exchanges: Vec<Exchange>,
}

impl ReplayHost {
    pub fn from_recording(recording: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct Recording {
            exchanges: Vec<Exchange>,
        }
        Ok(Self {
            exchanges: serde_json::from_str::<Recording>(recording)?.exchanges,
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
        Ok(Self { exchanges })
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
            // The file viewer (`vscode://codex/read-file`): `{contents}` of a local text file.
            "read-file" => {
                let path = params["path"].as_str().context("read-file needs a path")?;
                json!({"contents": std::fs::read_to_string(path)?})
            }
            _ => bail!("unsupported local host endpoint: {endpoint}"),
        };
        Ok(
            json!({"type": "fetch-response", "responseType": "success", "status": 200,
            "headers": {"content-type": "application/json"}, "bodyJsonString": body.to_string()}),
        )
    }
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
