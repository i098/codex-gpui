use super::{Service, ToolEvent, pipe::Cancel};
use crate::host::{
    Host as _, automation_schedule, check_automation_id, definition, device_id,
    ensure_unowned_automation,
};
use anyhow::{Context as _, Result, bail, ensure};
use rusqlite::{OpenFlags, OptionalExtension as _, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Arguments {
    mode: String,
    id: Option<String>,
    kind: Option<String>,
    name: Option<String>,
    prompt: Option<String>,
    rrule: Option<String>,
    status: Option<String>,
    destination: Option<String>,
    execution_environment: Option<String>,
    local_environment_config_path: Option<String>,
    project_id: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
    notification_policy: Option<String>,
    target_thread_id: Option<String>,
}

pub(super) async fn execute(
    service: &Service,
    value: &Value,
    thread: &str,
    cancel: &Cancel,
) -> Result<Value> {
    let args = Arguments::deserialize(value)?;
    cancel.check()?;
    match args.mode.as_str() {
        "view" => {
            only_id(value)?;
            let id = required(args.id.as_deref(), "id")?;
            let (_, original) = read(service, id)?.context("Automation does not exist in the app and could not be updated. It may have been deleted manually by the user.")?;
            Ok(json!({"mode": "view", "automation": definition(id, &original)?}))
        }
        "delete" => {
            only_id(value)?;
            remove(service, required(args.id.as_deref(), "id")?, cancel)
        }
        "create" | "update" | "suggested_create" | "suggested_update" => {
            validate(&args, value, thread)?;
            if args.mode.starts_with("suggested_") {
                return Ok(json!({"mode": args.mode, "saved": false}));
            }
            if args.mode == "create" {
                ensure!(
                    !has_dtstart(required(args.rrule.as_deref(), "rrule")?),
                    "Immediate automation creates cannot include DTSTART. Use suggested_create for an anchored schedule."
                );
                ensure!(
                    args.kind.as_deref() != Some("cron")
                        || args.execution_environment.as_deref() == Some("local"),
                    "New cron automations must use local execution"
                );
                if args.kind.as_deref() == Some("cron") {
                    ensure!(
                        args.destination
                            .as_deref()
                            .is_none_or(|destination| destination == "local")
                            && value.get("localEnvironmentConfigPath").is_none(),
                        "New cron automations do not accept worktree destinations or setup scripts"
                    );
                }
            }
            if args.mode == "update" && args.execution_environment.as_deref() == Some("worktree") {
                ensure!(
                    value.get("localEnvironmentConfigPath") == Some(&Value::Null),
                    "For safety, use suggested_update to review a worktree automation with a setup script, or set localEnvironmentConfigPath to null."
                );
            }
            save(service, &args, value, thread, cancel)
        }
        _ => bail!("Invalid automation mode"),
    }
}

pub(super) fn apply(service: &Service, value: &Value, thread: &str) -> Result<Value> {
    let mut args = Arguments::deserialize(value)?;
    ensure!(
        matches!(args.mode.as_str(), "suggested_create" | "suggested_update"),
        "Expected an automation proposal"
    );
    validate(&args, value, thread)?;
    args.mode = args.mode.strip_prefix("suggested_").unwrap().to_owned();
    save(service, &args, value, thread, &Cancel::new())
}

fn required<'a>(value: Option<&'a str>, name: &str) -> Result<&'a str> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("{name} is required"))
}

fn only_id(value: &Value) -> Result<()> {
    ensure!(
        value.as_object().is_some_and(|object| object
            .keys()
            .all(|key| matches!(key.as_str(), "mode" | "id"))),
        "This automation mode accepts only mode and id"
    );
    Ok(())
}

fn validate(args: &Arguments, value: &Value, thread: &str) -> Result<()> {
    required(args.name.as_deref(), "name")?;
    required(args.prompt.as_deref(), "prompt")?;
    required(args.rrule.as_deref(), "rrule")?;
    ensure!(
        matches!(args.status.as_deref(), Some("ACTIVE" | "PAUSED")),
        "status must be ACTIVE or PAUSED"
    );
    if args.mode.ends_with("update") {
        check_automation_id(required(args.id.as_deref(), "id")?)?;
    } else {
        ensure!(
            value.get("id").is_none(),
            "Create does not accept an automation id"
        );
    }
    ensure!(
        args.notification_policy
            .as_deref()
            .is_none_or(|policy| policy == "failed_runs_only"),
        "Invalid notificationPolicy"
    );
    match args.kind.as_deref() {
        Some("cron") => {
            ensure!(
                value.get("projectId").is_some(),
                "projectId is required; use null for projectless tasks"
            );
            required(args.model.as_deref(), "model")?;
            ensure!(
                matches!(
                    args.reasoning_effort.as_deref(),
                    Some(
                        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
                    )
                ),
                "Invalid reasoningEffort"
            );
            ensure!(
                matches!(
                    args.execution_environment.as_deref(),
                    Some("local" | "worktree")
                ),
                "Invalid executionEnvironment"
            );
            ensure!(
                args.destination
                    .as_deref()
                    .is_none_or(|destination| matches!(destination, "local" | "worktree")),
                "Invalid cron destination"
            );
            ensure!(
                value.get("targetThreadId").is_none(),
                "Cron tasks do not accept targetThreadId"
            );
        }
        Some("heartbeat") => {
            ensure!(
                ![
                    "projectId",
                    "model",
                    "reasoningEffort",
                    "executionEnvironment",
                    "localEnvironmentConfigPath"
                ]
                .iter()
                .any(|key| value.get(*key).is_some()),
                "Heartbeat tasks do not accept cron fields"
            );
            ensure!(
                args.destination
                    .as_deref()
                    .is_none_or(|destination| matches!(destination, "local" | "thread")),
                "Invalid heartbeat destination"
            );
            ensure!(
                args.target_thread_id.is_some() || args.destination.as_deref() == Some("thread"),
                "Missing targetThreadId or destination=thread"
            );
            let target = args.target_thread_id.as_deref().unwrap_or(thread);
            ensure!(uuid(target), "Invalid targetThreadId UUID");
        }
        _ => bail!("kind must be cron or heartbeat"),
    }
    Ok(())
}

fn has_dtstart(rule: &str) -> bool {
    rule.split([';', '\n']).any(|part| {
        let part = part.trim_start();
        part.get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("DTSTART"))
            && part
                .as_bytes()
                .get(7)
                .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
    })
}

fn uuid(value: &str) -> bool {
    if matches!(
        value,
        "00000000-0000-0000-0000-000000000000" | "ffffffff-ffff-ffff-ffff-ffffffffffff"
    ) {
        return true;
    }
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
        && matches!(value.as_bytes()[14], b'1'..=b'8')
        && matches!(
            value.as_bytes()[19],
            b'8' | b'9' | b'a' | b'b' | b'A' | b'B'
        )
}

fn read(service: &Service, key: &str) -> Result<Option<(toml::Table, String)>> {
    check_automation_id(key)?;
    if let Some(db) = service.storage.database(OpenFlags::SQLITE_OPEN_READ_ONLY)? {
        ensure_unowned_automation(&db, key)?;
    }
    let home = service.storage.home.canonicalize()?;
    let root = service.storage.home.join("automations");
    match root.join(key).symlink_metadata() {
        Ok(metadata) => ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Automation directory must not be a symlink"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let path = root.join(key).join("automation.toml");
    let path = match path.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let root = root.canonicalize()?;
    ensure!(
        root.starts_with(home) && path.starts_with(root),
        "Automation path escapes CODEX_HOME"
    );
    let original = fs::read_to_string(path)?;
    let item = definition(key, &original)?;
    if item["status"] == "DELETED" {
        return Ok(None);
    }
    Ok(Some((toml::from_str(&original)?, original)))
}

fn lock(service: &Service) -> Result<fs::File> {
    fs::create_dir_all(&service.storage.home)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(service.storage.home.join(".automation-tools.lock"))?;
    lock.lock()?;
    Ok(lock)
}

fn save(
    service: &Service,
    args: &Arguments,
    value: &Value,
    thread: &str,
    cancel: &Cancel,
) -> Result<Value> {
    let _lock = lock(service)?;
    cancel.check()?;
    let key = if args.mode == "create" {
        device_id()
    } else {
        required(args.id.as_deref(), "id")?.to_owned()
    };
    let previous = read(service, &key)?;
    if args.mode != "create" {
        ensure!(
            previous.is_some(),
            "Automation does not exist in the app and could not be updated. It may have been deleted manually by the user."
        );
    }
    let now = chrono::Utc::now().timestamp_millis();
    let mut raw = previous
        .as_ref()
        .map(|(raw, _)| raw.clone())
        .unwrap_or_default();
    raw.insert("version".into(), 1.into());
    raw.insert("id".into(), key.clone().into());
    for (key, text) in [
        ("kind", args.kind.as_deref()),
        ("name", args.name.as_deref()),
        ("prompt", args.prompt.as_deref()),
        ("rrule", args.rrule.as_deref()),
        ("status", args.status.as_deref()),
    ] {
        raw.insert(key.into(), required(text, key)?.to_owned().into());
    }
    raw.entry("created_at").or_insert(now.into());
    raw.insert("updated_at".into(), now.into());
    if value.get("notificationPolicy").is_some() {
        raw.remove("notification_policy");
        if let Some(policy) = &args.notification_policy {
            raw.insert("notification_policy".into(), policy.clone().into());
        }
    }
    if args.kind.as_deref() == Some("heartbeat") {
        for key in [
            "cwds",
            "target",
            "model",
            "reasoning_effort",
            "execution_environment",
            "local_environment_config_path",
            "auto_archive",
            "plugin_template_id",
        ] {
            raw.remove(key);
        }
        raw.insert(
            "target_thread_id".into(),
            args.target_thread_id
                .as_deref()
                .unwrap_or(thread)
                .to_owned()
                .into(),
        );
    } else {
        raw.remove("target_thread_id");
        let (target, cwds) = if let Some(project) = &args.project_id {
            let project = service
                .storage
                .projects()?
                .into_iter()
                .find(|item| item.id.as_ref() == project)
                .context("Automation project no longer exists")?;
            let root = project.root.context("Automation project has no folder")?;
            (
                toml::Table::from_iter([
                    ("type".into(), "project".into()),
                    ("project_id".into(), project.id.to_string().into()),
                ]),
                vec![toml::Value::String(root.to_string())],
            )
        } else {
            ensure!(
                args.execution_environment.as_deref() != Some("worktree"),
                "Worktree automations require a project with a folder"
            );
            (
                toml::Table::from_iter([("type".into(), "projectless".into())]),
                Vec::new(),
            )
        };
        raw.insert("target".into(), target.into());
        raw.insert("cwds".into(), cwds.into());
        raw.insert(
            "model".into(),
            required(args.model.as_deref(), "model")?.to_owned().into(),
        );
        raw.insert(
            "reasoning_effort".into(),
            required(args.reasoning_effort.as_deref(), "reasoningEffort")?
                .to_owned()
                .into(),
        );
        raw.insert(
            "execution_environment".into(),
            required(
                args.execution_environment.as_deref(),
                "executionEnvironment",
            )?
            .to_owned()
            .into(),
        );
        if value.get("localEnvironmentConfigPath").is_some() {
            raw.remove("local_environment_config_path");
            if let Some(path) = &args.local_environment_config_path {
                raw.insert("local_environment_config_path".into(), path.clone().into());
            }
        }
    }
    let contents = toml::to_string(&raw)?;
    let item = definition(&key, &contents)?;
    let root = service.storage.home.join("automations");
    fs::create_dir_all(&root)?;
    ensure!(
        root.canonicalize()?
            .starts_with(service.storage.home.canonicalize()?),
        "Automation path escapes CODEX_HOME"
    );
    let schedule = if item["status"] == "ACTIVE" {
        Some(
            automation_schedule::next_run(
                required(item["rrule"].as_str(), "rrule")?,
                required(item["kind"].as_str(), "kind")?,
                &key,
                now,
                rrule::Tz::LOCAL,
                &service.storage.jitter_salt(),
            )
            .context(automation_schedule::NO_FUTURE_RUN)?,
        )
    } else {
        None
    };
    let mut db = service
        .storage
        .database(OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let transaction = db
        .as_mut()
        .map(|db| db.transaction_with_behavior(TransactionBehavior::Immediate))
        .transpose()?;
    if let Some(transaction) = &transaction {
        ensure_unowned_automation(transaction, &key)?;
        persist(transaction, &item, schedule)?;
    }
    cancel.check()?;
    service.storage.write_automation(&key, &contents)?;
    if let Some(transaction) = transaction
        && let Err(error) = transaction.commit()
    {
        if let Some((_, original)) = previous {
            service
                .storage
                .write_automation(&key, &original)
                .context("Restore automation after database commit failed")?;
        } else {
            fs::remove_file(
                service
                    .storage
                    .home
                    .join("automations")
                    .join(&key)
                    .join("automation.toml"),
            )?;
        }
        return Err(error.into());
    }
    service.notify(ToolEvent::AutomationsChanged);
    Ok(json!({"automationId": key, "mode": args.mode, "status": item["status"]}))
}

fn persist(db: &rusqlite::Connection, item: &Value, schedule: Option<(i64, i64)>) -> Result<()> {
    let key = required(item["id"].as_str(), "id")?;
    let last: Option<i64> = db
        .query_row(
            "SELECT last_run_at FROM automations WHERE id = ?",
            [key],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    db.execute("INSERT INTO automations (id,name,prompt,status,next_run_at,next_run_nominal_at,last_run_at,cwds,target_type,project_id,rrule,model,reasoning_effort,created_at,updated_at,kind,target_thread_id,execution_environment,local_environment_config_path,plugin_template_id,notification_policy,auto_archive)
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)
        ON CONFLICT(id) DO UPDATE SET name=excluded.name,prompt=excluded.prompt,status=excluded.status,next_run_at=excluded.next_run_at,next_run_nominal_at=excluded.next_run_nominal_at,cwds=excluded.cwds,target_type=excluded.target_type,project_id=excluded.project_id,rrule=excluded.rrule,model=excluded.model,reasoning_effort=excluded.reasoning_effort,updated_at=excluded.updated_at,kind=excluded.kind,target_thread_id=excluded.target_thread_id,execution_environment=excluded.execution_environment,local_environment_config_path=excluded.local_environment_config_path,notification_policy=excluded.notification_policy", params![key,item["name"].as_str(),item["prompt"].as_str(),item["status"].as_str(),schedule.map(|(next,_)|next),schedule.map(|(_,nominal)|nominal),last,item.get("cwds").map_or_else(||"[]".into(),Value::to_string),item["target"]["type"].as_str(),item["target"]["projectId"].as_str(),item["rrule"].as_str(),item["model"].as_str(),item["reasoningEffort"].as_str(),item["createdAt"].as_i64(),item["updatedAt"].as_i64(),item["kind"].as_str(),item["targetThreadId"].as_str(),item["executionEnvironment"].as_str(),item["localEnvironmentConfigPath"].as_str(),item["pluginTemplateId"].as_str(),item["notificationPolicy"].as_str(),item["autoArchive"].as_bool().unwrap_or(false)])?;
    Ok(())
}

fn remove(service: &Service, key: &str, cancel: &Cancel) -> Result<Value> {
    let _lock = lock(service)?;
    let previous = read(service, key)?;
    let mut db = service
        .storage
        .database(OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let transaction = db
        .as_mut()
        .map(|db| db.transaction_with_behavior(TransactionBehavior::Immediate))
        .transpose()?;
    if let Some(transaction) = &transaction {
        ensure_unowned_automation(transaction, key)?;
        transaction.execute("UPDATE automations SET status='DELETED',next_run_at=NULL,next_run_nominal_at=NULL,updated_at=? WHERE id=?", params![chrono::Utc::now().timestamp_millis(),key])?;
    }
    cancel.check()?;
    if let Some((raw, _)) = &previous {
        let mut raw = raw.clone();
        raw.insert("status".into(), "DELETED".into());
        service
            .storage
            .write_automation(key, &toml::to_string(&raw)?)?;
    }
    if let Some(transaction) = transaction
        && let Err(error) = transaction.commit()
    {
        if let Some((_, original)) = &previous {
            service
                .storage
                .write_automation(key, original)
                .context("Restore automation after delete commit failed")?;
        }
        return Err(error.into());
    }
    if previous.is_some() {
        let directory = service.storage.home.join("automations").join(key);
        ensure!(
            !directory.symlink_metadata()?.file_type().is_symlink(),
            "Automation directory must not be a symlink"
        );
        fs::remove_dir_all(directory)
            .context("Automation still exists after the app tried to delete it")?;
    }
    service.notify(ToolEvent::AutomationsChanged);
    Ok(
        json!({"automationId": key, "mode": "delete", "deleteStatus": if previous.is_some() {"deleted"} else {"not_found"}}),
    )
}
