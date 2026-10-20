use super::{Fx, ROOT, script};
use anyhow::{Context as _, Result};
use app::host::LocalHost;
use rusqlite::{Connection, params};
use serde_json::Value;
use std::{fs, path::Path, sync::Arc};

pub fn run_store(home: &Path) -> Result<Connection> {
    fs::create_dir_all(home.join("sqlite"))?;
    let db = Connection::open(home.join("sqlite/codex.db"))?;
    db.execute_batch(
        "CREATE TABLE automations (id TEXT PRIMARY KEY, name TEXT, prompt TEXT,
        next_run_at INTEGER, next_run_nominal_at INTEGER, last_run_at INTEGER, rrule TEXT,
        status TEXT, account_id TEXT, user_id TEXT, installation_id TEXT);
        CREATE TABLE automation_runs (thread_id TEXT PRIMARY KEY, automation_id TEXT,
        status TEXT, read_at INTEGER, thread_title TEXT, source_cwd TEXT, inbox_title TEXT,
        inbox_summary TEXT, archived_assistant_message TEXT, archived_user_message TEXT,
        archived_reason TEXT, created_at INTEGER, updated_at INTEGER);",
    )?;
    Ok(db)
}

pub fn management(theme: &'static str, size: (f32, f32)) -> Result<(Fx, Connection)> {
    #[cfg(target_os = "macos")]
    super::install_panic_hook();
    let fixture: Value = serde_json::from_slice(&fs::read(format!(
        "{ROOT}/tools/parity/fixtures/automation-history.json"
    ))?)?;
    let reference: Value =
        serde_json::from_slice(&fs::read(format!("{ROOT}/refs/interactions/s037.json"))?)?;
    let id = fixture["taskId"]
        .as_str()
        .context("history fixture task id")?;
    let task = reference["fixture"]["items"]
        .as_array()
        .context("recorded tasks")?
        .iter()
        .find(|task| task["id"] == id)
        .context("recorded history task")?;
    let home = tempfile::tempdir()?;
    let db = run_store(home.path())?;
    db.execute(
        "INSERT INTO automations (id,name,prompt,status,rrule,next_run_at,next_run_nominal_at)
         VALUES (?,?,?,?,?,?,?)",
        params![
            id,
            task["name"].as_str(),
            task["prompt"].as_str(),
            task["status"].as_str(),
            task["rrule"].as_str(),
            task["nextRunAt"].as_i64(),
            task["nextRunNominalAt"].as_i64()
        ],
    )?;
    let rows = fixture["inbox"]["items"]
        .as_array()
        .context("recorded run rows")?;
    for row in rows {
        db.execute(
            "INSERT INTO automation_runs
             (thread_id,automation_id,status,read_at,thread_title,source_cwd,inbox_summary,
              archived_assistant_message,archived_user_message,archived_reason,created_at,updated_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
            params![row["threadId"].as_str(), id, row["status"].as_str(), row["readAt"].as_i64(),
                row["title"].as_str(), row["sourceCwd"].as_str(), row["description"].as_str(),
                row["archivedAssistantMessage"].as_str(), row["archivedUserMessage"].as_str(),
                row["archivedReason"].as_str(), row["createdAt"].as_i64(), row["createdAt"].as_i64()],
        )?;
    }
    let host = Arc::new(LocalHost::new(home.path()));
    host.write_automation(id, &format!(
        "version = 1\nid = {id:?}\nname = {}\nprompt = {}\nstatus = {}\nkind = \"cron\"\nrrule = {}\ncwds = {}\ncreated_at = {}\nupdated_at = {}\n",
        task["name"], task["prompt"], task["status"], task["rrule"], task["cwds"], task["createdAt"], task["updatedAt"],
    ))?;
    let mut fx = Fx::open(theme, size, script(None, |_| {})?)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx._host_home = home;
    fx.action(Box::new(ui::OpenScheduled))?;
    fx.wait("recorded task definition", |fx| {
        fx.read(|root, _| root.automations().is_some_and(|list| list.is_ok()))
    })?;
    fx.action(Box::new(
        ui::screens::automations::OpenAutomationManagement {
            id: Some(id.into()),
        },
    ))?;
    fx.wait("only the latest run marked read", |fx| {
        fx.read(|root, cx| {
            root.run_history(cx).is_some_and(|history| {
                let history = history.read(cx);
                history.items().len() == rows.len() && !history.items()[0]["readAt"].is_null()
            })
        })
    })?;
    Ok((fx, db))
}
