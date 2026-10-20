//! `automation-run-now` for a local task, from the original main process (`main-CqEoQDOU.js`
//! `Fl`/`Ll`/`Vl`, `src-BJv5Mk7n.js` `oa` and the `automation_runs` writers, `src-CP7XXlpZ.js`
//! `Al`-style permission modes and `Pr` model choice).
//!
//! Per task folder: the user's saved config picks the sandbox and approvals, the model list picks
//! the model, a `pending:` run row opens in the original database, then the app-server gets
//! `thread/start`, `thread/name/set` and `turn/start`. The row moves to the thread id; any failure
//! archives it. `finish_run` is the original's turn-completed update. Nothing here
//! starts a run when the app-server refuses it, and no row is written without the original's
//! database.

use super::LocalHost;
use super::{automation_schedule, git};
use anyhow::{Context as _, Result, bail, ensure};
use rusqlite::{OpenFlags, OptionalExtension as _, params};
use serde_json::{Value, json};
use std::{fs, process::Command};

/// `N.automationModel`, the model the original falls back to.
const AUTOMATION_MODEL: &str = "gpt-6.1-sol";
/// `Tr` and `Er`: the model and effort for a task that names none, when the server lists it.
const DEFAULT_MODEL: &str = "gpt-5.3-codex";
const DEFAULT_EFFORT: &str = "medium";
const MODES: [&str; 5] = [
    "read-only",
    "auto",
    "granular",
    "guardian-approvals",
    "full-access",
];

/// What `Al` returns for a run: the turn's approvals and sandbox.
#[derive(Debug, PartialEq)]
pub(super) struct Permissions {
    pub approval_policy: Value,
    pub reviewer: String,
    pub sandbox_policy: Value,
}

impl Permissions {
    /// `thread/start`'s `sandbox` string for the sandbox policy.
    fn sandbox_mode(&self) -> &'static str {
        sandbox_mode(&self.sandbox_policy).unwrap_or("workspace-write")
    }
}

struct Mode {
    sandbox_policy: Value,
    approval_policy: Value,
    reviewer: String,
}

fn sandbox_mode(policy: &Value) -> Option<&'static str> {
    match policy["type"].as_str()? {
        "dangerFullAccess" => Some("danger-full-access"),
        "readOnly" => Some("read-only"),
        "workspaceWrite" => Some("workspace-write"),
        _ => None,
    }
}

fn guardian(reviewer: &str) -> bool {
    matches!(reviewer, "auto_review" | "guardian_subagent")
}

fn workspace(roots: &[String], saved: &Value, policy: Option<Value>, reviewer: &str) -> Mode {
    let extra = saved["writable_roots"].as_array().into_iter().flatten();
    let writable: Vec<Value> = roots
        .iter()
        .map(|r| json!(r))
        .chain(extra.cloned())
        .collect();
    Mode {
        sandbox_policy: json!({
            "type": "workspaceWrite", "writableRoots": writable,
            "excludeSlashTmp": saved["exclude_slash_tmp"].as_bool().unwrap_or(false),
            "excludeTmpdirEnvVar": saved["exclude_tmpdir_env_var"].as_bool().unwrap_or(false),
            "networkAccess": saved["network_access"].as_bool().unwrap_or(false),
        }),
        approval_policy: policy.unwrap_or_else(|| json!("on-request")),
        reviewer: reviewer.to_owned(),
    }
}

fn read_only(policy: Option<Value>, reviewer: &str) -> Mode {
    Mode {
        sandbox_policy: json!({"type": "readOnly", "networkAccess": false}),
        approval_policy: policy.unwrap_or_else(|| json!("on-request")),
        reviewer: reviewer.to_owned(),
    }
}

fn full_access(policy: Option<Value>, reviewer: &str) -> Mode {
    Mode {
        sandbox_policy: json!({"type": "dangerFullAccess"}),
        approval_policy: policy.unwrap_or_else(|| json!("never")),
        reviewer: reviewer.to_owned(),
    }
}

/// `features.<name>` as the original reads it: a dotted key, else the `features` table.
fn feature(config: &Value, name: &str) -> Option<bool> {
    config[format!("features.{name}")]
        .as_bool()
        .or_else(|| config["features"][name].as_bool())
}

/// `er`: the saved reviewer, `user` when it is unknown or its feature is off.
fn saved_reviewer(config: &Value) -> &str {
    match config["approvals_reviewer"].as_str() {
        Some("guardian_subagent") if feature(config, "guardian_approval") == Some(false) => "user",
        Some(r @ ("user" | "auto_review" | "guardian_subagent")) => r,
        _ => "user",
    }
}

fn saved_policy(config: &Value) -> Option<Value> {
    Some(config["approval_policy"].clone()).filter(|p| !p.is_null())
}

/// `Qn`: the saved `sandbox_mode` and `approval_policy` as they stand.
fn custom(roots: &[String], config: &Value) -> Mode {
    let (policy, reviewer) = (saved_policy(config), saved_reviewer(config));
    match config["sandbox_mode"].as_str() {
        Some("danger-full-access") => full_access(policy, reviewer),
        Some("workspace-write") => {
            workspace(roots, &config["sandbox_workspace_write"], policy, reviewer)
        }
        _ => read_only(policy, reviewer),
    }
}

/// `$n`: the named agent mode over `roots` and the saved config.
fn mode(name: &str, roots: &[String], config: &Value) -> Mode {
    let plain = config["approval_policy"].is_null() || config["approval_policy"] == "on-request";
    let granular = json!({"granular": {"sandbox_approval": false, "rules": false,
        "skill_approval": false, "request_permissions": true, "mcp_elicitations": true}});
    match name {
        "read-only" => read_only(None, "user"),
        "full-access" => full_access(None, "user"),
        "granular" => workspace(roots, &Value::Null, Some(granular), "user"),
        "guardian-approvals" => match config["sandbox_mode"].as_str() {
            Some("read-only") if plain => read_only(None, "guardian_subagent"),
            Some("workspace-write") if plain => workspace(
                roots,
                &config["sandbox_workspace_write"],
                None,
                "guardian_subagent",
            ),
            _ => workspace(roots, &Value::Null, None, "guardian_subagent"),
        },
        "custom" => custom(roots, config),
        _ => workspace(roots, &Value::Null, None, "user"),
    }
}

/// `Kn`: do the config requirements allow this sandbox, approval policy and reviewer?
fn allowed(requirements: &Value, sandbox: Option<&str>, policy: &Value, reviewer: &str) -> bool {
    if requirements.is_null() {
        return true;
    }
    if let (Some(modes), Some(sandbox)) = (requirements["allowedSandboxModes"].as_array(), sandbox)
        && !modes.iter().any(|m| m == sandbox)
    {
        return false;
    }
    if let Some(policies) = requirements["allowedApprovalPolicies"].as_array()
        && !policies.contains(policy)
    {
        return false;
    }
    if let Some(reviewers) = requirements["allowedApprovalsReviewers"].as_array() {
        let any_guardian = reviewers.iter().any(|r| r.as_str().is_some_and(guardian));
        if !reviewers.iter().any(|r| r == reviewer) && !(guardian(reviewer) && any_guardian) {
            return false;
        }
    }
    true
}

/// `Va`: the modes the requirements allow, plus `custom` when the saved config sets a sandbox or
/// approval policy that they also allow.
fn allowed_modes(requirements: &Value, config: &Value) -> Vec<&'static str> {
    let mut modes: Vec<&'static str> = MODES
        .into_iter()
        .filter(|name| {
            let m = mode(name, &[], config);
            let profile = if *name == "read-only" {
                ":read-only"
            } else if *name == "full-access" {
                ":danger-full-access"
            } else {
                ":workspace"
            };
            (requirements.is_null()
                || requirements["allowedPermissionProfiles"].is_null()
                || requirements["allowedPermissionProfiles"][profile] == true)
                && allowed(
                    requirements,
                    sandbox_mode(&m.sandbox_policy),
                    &m.approval_policy,
                    &m.reviewer,
                )
        })
        .collect();
    let set = !config["approval_policy"].is_null() || config["sandbox_mode"].is_string();
    let c = custom(&[], config);
    if set
        && requirements["allowedPermissionProfiles"].is_null()
        && allowed(
            requirements,
            sandbox_mode(&c.sandbox_policy),
            &c.approval_policy,
            &c.reviewer,
        )
    {
        modes.push("custom");
    }
    modes
}

/// `Al`: the unattended run's approvals and sandbox. `agent_mode` is the mode the user last
/// picked on this host, which wins when the requirements allow it.
pub(super) fn permissions(
    roots: &[String],
    config: &Value,
    requirements: &Value,
    agent_mode: Option<&str>,
    projectless: bool,
) -> Result<Permissions> {
    let own = if config["sandbox_mode"].is_string() || !config["approval_policy"].is_null() {
        "custom"
    } else if guardian(saved_reviewer(config)) {
        "guardian-approvals"
    } else {
        "auto"
    };
    let wanted = agent_mode.unwrap_or(own);
    let modes = allowed_modes(requirements, config);
    let has = |name: &str| modes.contains(&name);
    let chosen = if has(wanted) {
        Some(wanted)
    } else {
        [own, "auto", "granular", "guardian-approvals", "read-only"]
            .into_iter()
            .find(|m| has(m))
    };
    let Some(chosen) = chosen else {
        bail!("The config requirements allow no agent mode for this run");
    };
    let picked = mode(chosen, roots, config);
    let never_allowed = requirements["allowedApprovalPolicies"]
        .as_array()
        .is_none_or(|p| p.contains(&json!("never")));
    let user_allowed = requirements["allowedApprovalsReviewers"]
        .as_array()
        .is_none_or(|r| r.contains(&json!("user")));
    let reviewer = if projectless && never_allowed && user_allowed {
        "user".to_owned()
    } else {
        picked.reviewer.clone()
    };
    let approval_policy =
        if never_allowed && (projectless || (chosen != "granular" && reviewer == "user")) {
            json!("never")
        } else {
            picked.approval_policy
        };
    let mut sandbox_policy = picked.sandbox_policy;
    if projectless
        && sandbox_policy["type"] == "workspaceWrite"
        && config["sandbox_mode"] == "workspace-write"
        && config["sandbox_workspace_write"]["network_access"] == true
    {
        sandbox_policy["networkAccess"] = json!(true);
    }
    Ok(Permissions {
        approval_policy,
        reviewer,
        sandbox_policy,
    })
}

/// `Pr`: the model and effort a run uses, given the server's `model/list` data.
pub(super) fn model_settings(item: &Value, models: &[Value]) -> (String, Value) {
    let find = |name: Option<&str>| name.and_then(|n| models.iter().find(|m| m["model"] == n));
    let effort = |model: &Value, wanted: Option<&str>| -> Value {
        let supported = model["supportedReasoningEfforts"].as_array();
        wanted
            .filter(|w| supported.is_some_and(|s| s.iter().any(|e| e["reasoningEffort"] == *w)))
            .map(Value::from)
            .or_else(|| Some(model["defaultReasoningEffort"].clone()).filter(|e| !e.is_null()))
            .or_else(|| supported?.first().map(|e| e["reasoningEffort"].clone()))
            .unwrap_or(Value::Null)
    };
    let (requested, wanted) = (item["model"].as_str(), item["reasoningEffort"].as_str());
    if let Some(m) = find(requested) {
        return (
            m["model"].as_str().unwrap_or_default().to_owned(),
            effort(m, wanted),
        );
    }
    if requested.is_none()
        && let Some(m) = find(Some(DEFAULT_MODEL))
    {
        return (
            DEFAULT_MODEL.to_owned(),
            effort(m, wanted.or(Some(DEFAULT_EFFORT))),
        );
    }
    match find(Some(AUTOMATION_MODEL))
        .or_else(|| models.iter().find(|m| m["isDefault"] == true))
        .or_else(|| models.first())
    {
        Some(m) => (
            m["model"].as_str().unwrap_or_default().to_owned(),
            effort(m, None),
        ),
        None => (
            requested.unwrap_or(DEFAULT_MODEL).to_owned(),
            json!(wanted.unwrap_or(DEFAULT_EFFORT)),
        ),
    }
}

fn git(dir: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// `aAe`: the git folders a run in `cwd` must write: its repository root and its common git dir.
fn git_roots(source: &str, cwd: &str) -> Vec<String> {
    let mut roots = Vec::new();
    let top = git(cwd, &["rev-parse", "--show-toplevel"]);
    if let Some(top) = top.as_ref().filter(|t| *t != cwd) {
        roots.push(top.clone());
    }
    let source_top = git(source, &["rev-parse", "--show-toplevel"]);
    if let Some(s) = source_top
        .as_ref()
        .filter(|s| *s != cwd && Some(*s) != top.as_ref())
    {
        roots.push(s.clone());
    }
    let base = top.as_deref().unwrap_or(source);
    if let Some(common) = git(base, &["rev-parse", "--git-common-dir"]) {
        let path = std::path::Path::new(&common);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::path::Path::new(source_top.as_deref().unwrap_or(source)).join(path)
        };
        roots.push(path.to_string_lossy().into_owned());
    }
    roots
}

impl LocalHost {
    /// The first root folder of the original sidebar project `id` (`local-projects` in
    /// `.codex-global-state.json`), like the original's `cl` then `en(project.roots)`.
    fn project_root(&self, id: &str) -> Result<String> {
        self.global_state()["local-projects"]
            .as_object()
            .into_iter()
            .flat_map(|projects| projects.values())
            .find(|project| project["id"] == id)
            .and_then(|project| project["rootPaths"][0].as_str())
            .map(str::to_owned)
            .context("Scheduled run skipped: no folders configured")
    }

    fn global_state(&self) -> Value {
        fs::read(self.home.join(".codex-global-state.json"))
            .ok()
            .and_then(|contents| serde_json::from_slice(&contents).ok())
            .unwrap_or(Value::Null)
    }

    /// The agent mode the user last picked on this host (`jl`) [INFERENCE: the original reads
    /// `agent-mode-by-host-id` from its global state; the persisted-atom table is assumed].
    fn agent_mode(&self) -> Option<String> {
        let state = self.global_state();
        let mode =
            state["electron-persisted-atom-state"]["agent-mode-by-host-id"]["local"].as_str()?;
        [
            "read-only",
            "auto",
            "granular",
            "guardian-approvals",
            "full-access",
            "custom",
        ]
        .contains(&mode)
        .then(|| mode.to_owned())
    }

    /// `ia`: opens an `IN_PROGRESS` row. `None` without the original database.
    fn begin_run(&self, automation: &str, id: &str, name: &str, cwd: &str) -> Result<()> {
        let Some(db) = self.database(OpenFlags::SQLITE_OPEN_READ_WRITE)? else {
            return Ok(());
        };
        let now = chrono::Utc::now().timestamp_millis();
        db.execute(
            "INSERT OR IGNORE INTO automation_runs (thread_id, automation_id, status, thread_title,
             source_cwd, created_at, updated_at) VALUES (?, ?, 'IN_PROGRESS', ?, ?, ?, ?)",
            params![id, automation, name, cwd, now, now],
        )?;
        Ok(())
    }

    /// `pa`, else `ia`: the pending row takes the thread id.
    fn name_run(
        &self,
        automation: &str,
        pending: &str,
        thread: &str,
        name: &str,
        cwd: &str,
    ) -> Result<()> {
        let Some(db) = self.database(OpenFlags::SQLITE_OPEN_READ_WRITE)? else {
            return Ok(());
        };
        let moved = db.execute(
            "UPDATE automation_runs SET thread_id = ?, updated_at = ? WHERE thread_id = ?",
            params![thread, chrono::Utc::now().timestamp_millis(), pending],
        )?;
        if moved == 0 {
            self.begin_run(automation, thread, name, cwd)?;
        }
        Ok(())
    }

    /// `ca`: a run that could not start is archived, like the original's `auto` archive.
    fn archive_run(&self, id: &str) {
        let Ok(Some(db)) = self.database(OpenFlags::SQLITE_OPEN_READ_WRITE) else {
            return;
        };
        let _ = db.execute(
            "UPDATE automation_runs SET status = 'ARCHIVED', updated_at = ?,
             archived_reason = COALESCE(archived_reason, 'auto') WHERE thread_id = ?",
            params![chrono::Utc::now().timestamp_millis(), id],
        );
    }

    /// The run salt for the schedule jitter (`ZF`): the stored one, else a new one that is kept.
    fn jitter_salt(&self) -> String {
        let path = self.home.join("automations/.run-jitter-salt");
        let stored = fs::read_to_string(&path)
            .map(|s| s.trim().to_owned())
            .unwrap_or_default();
        if !stored.is_empty() {
            return stored;
        }
        let salt = super::device_id();
        let _ = fs::write(&path, format!("{salt}\n"));
        salt
    }

    /// What the original does first on a run now (`Fl`, `Il`, `Vn`): the next scheduled run is
    /// kept when it is still ahead, else recalculated after the run's own nominal time, and the
    /// run itself becomes the task's last run. A paused task keeps its times [INFERENCE: `QF`
    /// is assumed to leave them alone]. Like the original, a failed write is not an error.
    fn record_run_now(&self, item: &Value, now: i64) {
        let Ok(Some(db)) = self.database(OpenFlags::SQLITE_OPEN_READ_WRITE) else {
            return;
        };
        let (next, nominal) = (
            item["nextRunAt"].as_i64(),
            item["nextRunNominalAt"].as_i64(),
        );
        let (next, nominal) = if item["status"] != "ACTIVE" {
            (next, nominal)
        } else if let Some((n, m)) = next.zip(nominal).filter(|(n, m)| *n > now && *m > now) {
            (Some(n), Some(m))
        } else if next.is_none() && nominal.is_some() && item["lastRunAt"].is_i64() {
            (None, nominal)
        } else {
            let rule = item["rrule"].as_str().unwrap_or_default();
            let one_shot = rule.split(';').any(|part| part == "COUNT=1");
            let after = next.filter(|n| *n <= now).and(nominal);
            let scheduled = (!(one_shot && next.is_some_and(|n| n <= now)))
                .then(|| {
                    automation_schedule::next_run_after(
                        rule,
                        "cron",
                        item["id"].as_str().unwrap_or_default(),
                        now,
                        after,
                        rrule::Tz::LOCAL,
                        &self.jitter_salt(),
                    )
                })
                .flatten();
            match scheduled {
                Some((n, m)) => (Some(n), Some(m)),
                None => (None, nominal),
            }
        };
        let _ = db.execute(
            "INSERT INTO automations
             (id, name, prompt, status, next_run_at, next_run_nominal_at, last_run_at, cwds,
              target_type, project_id, rrule, model, reasoning_effort, created_at, updated_at,
              kind, target_thread_id, execution_environment, local_environment_config_path,
              plugin_template_id, notification_policy, auto_archive)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                     ?16, ?17, ?18, ?19, ?20, ?21, ?22)
             ON CONFLICT(id) DO UPDATE SET next_run_at=excluded.next_run_at,
              next_run_nominal_at=excluded.next_run_nominal_at, last_run_at=excluded.last_run_at",
            params![
                item["id"].as_str(),
                item["name"].as_str(),
                item["prompt"].as_str(),
                item["status"].as_str(),
                next,
                nominal,
                now,
                item.get("cwds")
                    .map_or_else(|| "[]".to_owned(), Value::to_string),
                item["target"]["type"].as_str(),
                item["target"]["projectId"].as_str(),
                item["rrule"].as_str(),
                item["model"].as_str(),
                item["reasoningEffort"].as_str(),
                item["createdAt"].as_i64(),
                item["updatedAt"].as_i64(),
                item["kind"].as_str(),
                item["targetThreadId"].as_str(),
                item["executionEnvironment"].as_str(),
                item["localEnvironmentConfigPath"].as_str(),
                item["pluginTemplateId"].as_str(),
                item["notificationPolicy"].as_str(),
                item["autoArchive"].as_bool().unwrap_or(false)
            ],
        );
    }

    /// Finish the initial automation turn before the notification policy reads the run.
    pub fn finish_run(&self, thread_id: &str, turn: &Value) -> Result<bool> {
        let tracked = self
            .bridge
            .as_ref()
            .and_then(|bridge| bridge.automation_turn(thread_id));
        let finish_unrecorded = || {
            if let (Some(bridge), Some(tracked)) = (&self.bridge, &tracked)
                && pollster::block_on(tracked.matches_turn(turn["id"].as_str().unwrap_or_default()))
            {
                tracked.mark_completed();
                bridge.release_automation_turn(thread_id, tracked);
            }
            Ok(false)
        };
        let Some(db) = self.database(OpenFlags::SQLITE_OPEN_READ_WRITE)? else {
            return finish_unrecorded();
        };
        let run: Option<(String, String)> = db
            .query_row(
                "SELECT r.automation_id, r.status FROM automation_runs r JOIN automations a ON a.id = r.automation_id
                 WHERE r.thread_id = ? AND r.status IN ('IN_PROGRESS', 'PENDING_REVIEW')
                 AND a.account_id IS NULL AND a.user_id IS NULL AND a.installation_id IS NULL",
                [thread_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((automation, status)) = run else {
            return finish_unrecorded();
        };
        if let Some(tracked) = &tracked
            && !pollster::block_on(tracked.matches_turn(turn["id"].as_str().unwrap_or_default()))
        {
            return Ok(false);
        }
        if status == "PENDING_REVIEW" && tracked.is_none() {
            return Ok(false);
        }
        let tasks = self.automations()?;
        let task = tasks["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|task| task["id"] == automation);
        let muted = tracked.as_ref().map_or_else(
            || task.is_some_and(|task| task["notificationPolicy"] == "failed_runs_only"),
            |tracked| tracked.muted,
        );
        let completed = turn["status"] == "completed";
        let now = chrono::Utc::now().timestamp_millis();
        let changed = db.execute(
            "UPDATE automation_runs SET status = 'PENDING_REVIEW',
             read_at = CASE WHEN ?1 THEN CASE WHEN ?2 THEN COALESCE(read_at, ?3) ELSE NULL END ELSE read_at END,
             updated_at = ?3 WHERE thread_id = ?4 AND status IN ('IN_PROGRESS', 'PENDING_REVIEW')",
            params![muted, completed, now, thread_id],
        )? > 0;
        if let (Some(bridge), Some(tracked)) = (&self.bridge, tracked) {
            if !changed || !tracked.mark_completed() {
                return Ok(false);
            }
            let heartbeat = tasks["items"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|task| task["kind"] == "heartbeat" && task["targetThreadId"] == thread_id);
            let archived = if completed && tracked.auto_archive && !heartbeat {
                let home = self.home.clone();
                pollster::block_on(bridge.archive_automation_turn(
                    thread_id,
                    &tracked,
                    || Self::mark_run_archived(&db, thread_id),
                    {
                        let thread_id = thread_id.to_owned();
                        move || Self::restore_run_row(&home, &thread_id)
                    },
                ))
            } else {
                Ok(())
            };
            if let Err(error) = archived {
                eprintln!("codex-gpui: failed to archive automation run: {error:#}");
            }
            if !pollster::block_on(tracked.needs_restore()) {
                bridge.release_automation_turn(thread_id, &tracked);
            }
        }
        Ok(changed)
    }

    fn mark_run_archived(db: &rusqlite::Connection, thread_id: &str) -> Result<()> {
        ensure!(
            db.execute(
                "UPDATE automation_runs SET status = 'ARCHIVED', updated_at = ?,
             archived_reason = COALESCE(archived_reason, 'auto')
             WHERE thread_id = ? AND status = 'PENDING_REVIEW'",
                params![chrono::Utc::now().timestamp_millis(), thread_id],
            )? > 0,
            "Automation run changed during archive"
        );
        Ok(())
    }

    fn restore_run_row(home: &std::path::Path, thread_id: &str) -> Result<()> {
        let db = rusqlite::Connection::open_with_flags(
            home.join("sqlite/codex.db"),
            OpenFlags::SQLITE_OPEN_READ_WRITE,
        )?;
        let now = chrono::Utc::now().timestamp_millis();
        db.execute(
            "UPDATE automation_runs SET status = 'ACCEPTED', read_at = COALESCE(read_at, ?),
             updated_at = ?, archived_reason = NULL WHERE thread_id = ? AND status = 'ARCHIVED'",
            params![now, now, thread_id],
        )?;
        Ok(())
    }

    /// The original's `automation-run-now` for a task: one app-server thread per task folder, in
    /// a new worktree of the folder's repository for the original default environment, titled
    /// with the task name, then a turn with the original's run preamble and the prompt. A run
    /// that cannot start fails with the reason. The original also runs projectless and heartbeat
    /// tasks; those need workspace directories and renderer thread state, which this host does
    /// not have, so they fail here instead of running somewhere else.
    pub(super) fn automation_run_now(&self, params: &Value) -> Result<Value> {
        let id = params["id"].as_str().context("missing automation id")?;
        let item = self
            .automations()?
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| items.iter().find(|item| item["id"] == id))
            .cloned()
            .context("Automation not found.")?;
        let prompt = item["prompt"].as_str().unwrap_or_default();
        ensure!(
            !ui::screens::automations::event_triggered(prompt),
            "Automations triggered by events can’t be run manually"
        );
        ensure!(
            item["kind"] == "cron",
            "Heartbeat tasks cannot be run from this app yet"
        );
        ensure!(
            matches!(
                item["executionEnvironment"].as_str(),
                Some("local" | "worktree")
            ),
            "Unknown automation execution environment"
        );
        let worktree_env = item["executionEnvironment"] == "worktree";
        let local_environment = item["localEnvironmentConfigPath"]
            .as_str()
            .map(std::path::PathBuf::from);
        let cwds: Vec<String> = match item["target"]["type"].as_str() {
            None => item["cwds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|cwd| cwd.as_str().map(str::to_owned))
                .collect(),
            Some("project") => {
                vec![self.project_root(item["target"]["projectId"].as_str().unwrap_or_default())?]
            }
            Some(_) => bail!("Projectless tasks cannot be run from this app yet"),
        };
        ensure!(
            !cwds.is_empty(),
            "Scheduled run skipped: no folders configured"
        );
        let bridge = self
            .bridge
            .as_ref()
            .context("The app server is not connected")?;
        self.record_run_now(&item, chrono::Utc::now().timestamp_millis());
        let name = item["name"].as_str().unwrap_or_default();
        let last = item["lastRunAt"].as_i64().map_or_else(
            || "never".to_owned(),
            |ms| {
                let at = chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default();
                format!(
                    "{} ({ms})",
                    at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                )
            },
        );
        let text = format!(
            "Automation: {name}\n\nAutomation ID: {id}\nAutomation memory: $CODEX_HOME/automations/{id}/memory.md\nLast run: {last}\n\n{prompt}"
        );
        let memory = self
            .home
            .join("automations")
            .join(id)
            .to_string_lossy()
            .into_owned();
        let agent_mode = self.agent_mode();
        let (models, requirements) = pollster::block_on(async {
            // The original falls back to the task's own model when the list cannot be read.
            let models: Option<Vec<Value>> = bridge
                .request::<_, Value>(
                    "model/list",
                    json!({"includeHidden": true, "cursor": null, "limit": 100}),
                )
                .await
                .ok()
                .and_then(|list| list["data"].as_array().cloned());
            let requirements: Value = bridge
                .request::<_, Value>("configRequirements/read", json!({}))
                .await?;
            Ok::<_, anyhow::Error>((models, requirements["requirements"].clone()))
        })?;
        let (model, effort) = match &models {
            Some(models) => model_settings(&item, models),
            None => (
                item["model"]
                    .as_str()
                    .unwrap_or(AUTOMATION_MODEL)
                    .to_owned(),
                json!(item["reasoningEffort"].as_str().unwrap_or(DEFAULT_EFFORT)),
            ),
        };
        let mut first_error = None;
        for cwd in &cwds {
            let pending = format!("pending:{}", super::device_id());
            let started = self.begin_run(id, &pending, name, cwd).and_then(|()| {
                // The original's default: the run happens in a fresh worktree of the folder's
                // repository, or in the folder itself when it is no repository.
                let worktree = if worktree_env {
                    git::create_worktree(
                        &self.home.join("worktrees"),
                        std::path::Path::new(cwd),
                        None,
                        local_environment.as_deref(),
                    )?
                } else {
                    None
                };
                let run_cwd = worktree.as_ref().map_or_else(
                    || cwd.clone(),
                    |w| w.workspace_root.to_string_lossy().into_owned(),
                );
                let started = pollster::block_on(async {
                    let saved: Value = bridge
                        .request("config/read", json!({"includeLayers": false, "cwd": cwd}))
                        .await?;
                    let config = saved["config"].clone();
                    let first = permissions(
                        std::slice::from_ref(cwd),
                        &config,
                        &requirements,
                        agent_mode.as_deref(),
                        false,
                    )?;
                    let thread: Value = bridge
                        .request(
                            "thread/start",
                            json!({"cwd": run_cwd, "model": model, "approvalPolicy": first.approval_policy,
                                "approvalsReviewer": first.reviewer, "sandbox": first.sandbox_mode(),
                                "threadSource": "automation"}),
                        )
                        .await?;
                    let thread_id = thread["thread"]["id"]
                        .as_str()
                        .context("thread/start returned no thread")?;
                    Ok::<_, anyhow::Error>((config, thread_id.to_owned()))
                });
                let (config, thread_id) = match started {
                    Ok(started) => started,
                    Err(error) => {
                        // No thread owns the worktree yet, so the helper may remove it.
                        if let Some(worktree) = &worktree {
                            let _ = git::remove_unstarted_worktree(&self.home.join("worktrees"), worktree);
                        }
                        return Err(error);
                    }
                };
                if let Some(worktree) = &worktree {
                    // The original only logs a failure to record the owner.
                    let _ = git::set_owner_thread(&worktree.git_root, &thread_id);
                }
                // From here the row follows the thread, so a failure archives that row.
                let _ = self.name_run(id, &pending, &thread_id, name, cwd);
                (|| {
                    let mut roots = vec![cwd.clone(), run_cwd.clone(), memory.clone()];
                    roots.extend(git_roots(cwd, &run_cwd));
                    let mut seen = std::collections::HashSet::new();
                    roots.retain(|root| seen.insert(root.clone()));
                    let turn = permissions(&roots, &config, &requirements, agent_mode.as_deref(), false)?;
                    pollster::block_on(async {
                        // The title is cosmetic: the original only logs a failure to set it.
                        let _ = bridge
                            .request::<_, Value>(
                                "thread/name/set",
                                json!({ "threadId": thread_id, "name": name }),
                            )
                            .await;
                        bridge
                            .start_automation_turn(
                                json!({"threadId": thread_id, "cwd": run_cwd, "model": model, "effort": effort,
                                    "approvalPolicy": turn.approval_policy, "approvalsReviewer": turn.reviewer,
                                    "sandboxPolicy": turn.sandbox_policy,
                                    "input": [{ "type": "text", "text": text, "text_elements": [] }]}),
                                item["notificationPolicy"] == "failed_runs_only",
                                item["kind"] == "cron" && item["autoArchive"] == true,
                            )
                            .await
                    })
                    .map(drop)
                })()
                .inspect_err(|_| self.archive_run(&thread_id))
            });
            if let Err(error) = started {
                self.archive_run(&pending);
                first_error.get_or_insert(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(json!({ "success": true })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Vec<String> {
        vec!["/p".to_owned()]
    }

    #[test]
    fn an_unconfigured_run_is_workspace_write_that_never_asks() {
        let p = permissions(&roots(), &json!({}), &Value::Null, None, false).unwrap();
        assert_eq!(p.approval_policy, "never");
        assert_eq!(p.sandbox_policy["type"], "workspaceWrite");
        assert_eq!(p.sandbox_policy["writableRoots"], json!(["/p"]));
    }

    #[test]
    fn requirements_that_forbid_never_keep_the_mode_policy() {
        let req = json!({"allowedApprovalPolicies": ["on-request"]});
        let p = permissions(&roots(), &json!({}), &req, None, false).unwrap();
        assert_eq!(p.approval_policy, "on-request");
    }

    #[test]
    fn saved_sandbox_and_policy_are_the_custom_mode() {
        let cfg = json!({"sandbox_mode": "read-only", "approval_policy": "untrusted"});
        let p = permissions(&roots(), &cfg, &Value::Null, None, false).unwrap();
        assert_eq!(p.sandbox_policy["type"], "readOnly");
        // An unattended run never asks, so the original overrides the saved policy ...
        assert_eq!(p.approval_policy, "never");
        // ... unless the requirements do not allow `never`.
        let req = json!({"allowedApprovalPolicies": ["untrusted"]});
        let p = permissions(&roots(), &cfg, &req, None, false).unwrap();
        assert_eq!(p.approval_policy, "untrusted");
        assert_eq!(p.sandbox_mode(), "read-only");
    }

    #[test]
    fn a_picked_mode_the_requirements_forbid_falls_back() {
        let req = json!({"allowedSandboxModes": ["read-only"]});
        let p = permissions(&roots(), &json!({}), &req, Some("full-access"), false).unwrap();
        assert_eq!(p.sandbox_policy["type"], "readOnly");
        let none = json!({"allowedSandboxModes": []});
        assert!(permissions(&roots(), &json!({}), &none, None, false).is_err());
    }

    fn models() -> Vec<Value> {
        vec![
            json!({"model": "a", "isDefault": true, "defaultReasoningEffort": "low",
                "supportedReasoningEfforts": [{"reasoningEffort": "low"}, {"reasoningEffort": "high"}]}),
            json!({"model": DEFAULT_MODEL, "supportedReasoningEfforts": [{"reasoningEffort": "medium"}]}),
        ]
    }

    #[test]
    fn a_listed_model_keeps_a_supported_effort_and_drops_an_unsupported_one() {
        let task = |m: &str, e: &str| json!({"model": m, "reasoningEffort": e});
        assert_eq!(
            model_settings(&task("a", "high"), &models()),
            ("a".into(), json!("high"))
        );
        assert_eq!(
            model_settings(&task("a", "max"), &models()),
            ("a".into(), json!("low"))
        );
    }

    #[test]
    fn an_unlisted_or_unnamed_model_falls_back_like_the_original() {
        let unnamed = json!({});
        assert_eq!(
            model_settings(&unnamed, &models()),
            (DEFAULT_MODEL.into(), json!("medium"))
        );
        let gone = json!({"model": "retired"});
        assert_eq!(model_settings(&gone, &models()), ("a".into(), json!("low")));
        assert_eq!(
            model_settings(&gone, &[]),
            ("retired".into(), json!("medium"))
        );
    }
}
