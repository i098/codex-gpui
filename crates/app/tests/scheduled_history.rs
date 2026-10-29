#![cfg(target_os = "macos")]

mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host, LocalHost, Request, json_body};
use common::{Fx, GREETING, SUBTRACT};
use gpui::Entity;
use rusqlite::{Connection, params};
use serde_json::json;
use std::{fs, sync::Arc};
use ui::screens::automations::{
    OpenAutomation,
    history::{History, HistoryAction, Operation},
};

#[test]
fn proxy() {
    common::proxy_child();
}

fn page(fx: &Fx) -> Entity<History> {
    fx.read(|root, cx| {
        root.scheduled_sidebar()
            .unwrap()
            .read(cx)
            .history
            .clone()
            .unwrap()
    })
}

fn operation(fx: &mut Fx, operation: Operation) -> Result<()> {
    fx.action(Box::new(HistoryAction(page(fx), operation)))
}

#[test]
fn native_history_reads_confirms_retries_skips_and_reports_partial_archive() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let saved = json_body(&host.fetch(&Request::post("automation-create", Some(json!({
        "name": "Hourly review", "prompt": "Review local changes", "rrule": "FREQ=HOURLY;INTERVAL=1;BYMINUTE=0", "cwds": [home.path()]
    }))))?)?;
    let automation = saved["item"]["id"].as_str().context("task ID")?.to_owned();
    fs::create_dir_all(home.path().join("sqlite"))?;
    let db = Connection::open(home.path().join("sqlite/codex.db"))?;
    db.execute_batch("CREATE TABLE automations (
        id TEXT PRIMARY KEY, name TEXT, prompt TEXT, next_run_at INTEGER, next_run_nominal_at INTEGER,
        last_run_at INTEGER, rrule TEXT, status TEXT, account_id TEXT, user_id TEXT, installation_id TEXT);
        CREATE TABLE automation_runs (
        thread_id TEXT PRIMARY KEY, automation_id TEXT, status TEXT, read_at INTEGER,
        thread_title TEXT, source_cwd TEXT, inbox_title TEXT, inbox_summary TEXT,
        archived_assistant_message TEXT, archived_user_message TEXT, archived_reason TEXT,
        created_at INTEGER, updated_at INTEGER);")?;
    db.execute("INSERT INTO automations (id, name, prompt, rrule, status, next_run_at, next_run_nominal_at) VALUES (?, 'Hourly review', 'Review local changes', 'FREQ=HOURLY;INTERVAL=1;BYMINUTE=0', 'ACTIVE', ?, ?)",
        params![automation, saved["item"]["nextRunAt"].as_i64(), saved["item"]["nextRunNominalAt"].as_i64()])?;
    for (id, status, created) in [
        (GREETING, "PENDING_REVIEW", 5),
        (SUBTRACT, "ACCEPTED", 4),
        ("active-chat", "PENDING_REVIEW", 3),
        ("running", "IN_PROGRESS", 2),
        ("archived", "ARCHIVED", 1),
    ] {
        let created = saved["item"]["createdAt"].as_i64().unwrap() - (6 - created) * 3_600_000;
        db.execute("INSERT INTO automation_runs (thread_id, automation_id, status, created_at, thread_title) VALUES (?, ?, ?, ?, ?)", params![id, automation, status, created, id])?;
    }
    let mut lines = common::script(None, |_| {})?;
    let template = lines
        .iter()
        .find_map(|line| line["msg"]["result"].get("thread").cloned())
        .context("recorded thread")?;
    for (index, (id, active)) in [(GREETING, false), (SUBTRACT, false), ("active-chat", true)]
        .into_iter()
        .cycle()
        .take(6)
        .enumerate()
    {
        let mut thread = template.clone();
        thread["id"] = json!(id);
        thread["status"] = json!({"type": if active {"active"} else {"idle"}});
        let request = 90_000 + index;
        lines.push(json!({"dir":"send","msg":{"id":request,"method":"thread/read","params":{"threadId":id,"includeTurns":false}}}));
        lines.push(json!({"dir":"recv","msg":{"id":request,"result":{"thread":thread}}}));
        if !active {
            let request = 91_000 + index;
            lines.push(json!({"dir":"send","msg":{"id":request,"method":"thread/archive","params":{"threadId":id}}}));
            lines.push(if id == SUBTRACT {
                json!({"dir":"recv","msg":{"id":request,"error":{"code":-32000,"message":"archive refused"}}})
            } else {
                json!({"dir":"recv","msg":{"id":request,"result":{}}})
            });
        }
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(Arc::new(host), cx));
    fx.click(common::RAIL_SCHEDULED)?;
    fx.wait("stored task", |fx| {
        fx.read(|root, _| root.automations().is_some_and(Result::is_ok))
    })?;
    fx.action(Box::new(OpenAutomation {
        id: automation.clone(),
    }))?;
    fx.wait("stored run history", |fx| {
        fx.read(|_, cx| page(fx).read(cx).runs.len() == 5 && !page(fx).read(cx).busy)
    })?;
    operation(&mut fx, Operation::MarkRead)?;
    fx.wait("read markers saved", |fx| {
        fx.read(|_, cx| {
            let history = page(fx);
            let history = history.read(cx);
            !history.busy
                && history
                    .runs
                    .iter()
                    .filter(|run| run.actionable())
                    .all(|run| run.read_at.is_some())
        })
    })?;
    for id in ["running", "archived"] {
        ensure!(
            db.query_row(
                "SELECT read_at FROM automation_runs WHERE thread_id=?",
                [id],
                |row| row.get::<_, Option<i64>>(0)
            )?
            .is_none()
        );
    }
    db.execute_batch(
        "CREATE TRIGGER fail_archive BEFORE UPDATE OF status ON automation_runs
         WHEN NEW.status = 'ARCHIVED'
         BEGIN SELECT RAISE(FAIL, 'injected run-store update failure'); END;",
    )?;
    operation(&mut fx, Operation::Archive)?;
    ensure!(fx.cx.pending_prompt().is_some(), "archive must ask first");
    ensure!(
        !fx.sent()?
            .iter()
            .any(|request| request["method"] == "thread/archive")
    );
    fx.cx.simulate_prompt_answer("Cancel");
    fx.cx.run_until_parked();
    ensure!(
        !fx.sent()?
            .iter()
            .any(|request| request["method"] == "thread/archive")
    );
    operation(&mut fx, Operation::Archive)?;
    fx.cx.simulate_prompt_answer("Archive all");
    fx.wait("archive succeeded but run-store update failed", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|text| text == "Archived 0; could not archive 2")
        })
    })?;
    fx.wait("failed archive history refreshed", |fx| {
        fx.read(|_, cx| !page(fx).read(cx).busy)
    })?;
    ensure!(
        db.query_row(
            "SELECT status FROM automation_runs WHERE thread_id=?",
            [GREETING],
            |row| row.get::<_, String>(0)
        )? == "PENDING_REVIEW"
    );
    ensure!(fx.sent()?.iter().any(|request| {
        request["method"] == "thread/archive" && request["params"]["threadId"] == GREETING
    }));
    db.execute_batch("DROP TRIGGER fail_archive")?;
    operation(&mut fx, Operation::Archive)?;
    fx.cx.simulate_prompt_answer("Archive all");
    fx.wait("partial archive result", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|text| text == "Archived 1; could not archive 1")
        })
    })?;
    fx.wait("stored history refreshed", |fx| {
        fx.read(|_, cx| {
            let history = page(fx);
            let history = history.read(cx);
            !history.busy
                && history
                    .runs
                    .iter()
                    .any(|run| run.thread_id == GREETING && run.status == "ARCHIVED")
        })
    })?;
    for id in [SUBTRACT, "active-chat"] {
        ensure!(
            db.query_row(
                "SELECT status FROM automation_runs WHERE thread_id=?",
                [id],
                |row| row.get::<_, String>(0)
            )? != "ARCHIVED"
        );
    }
    let archived: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|request| request["method"] == "thread/archive")
        .map(|request| request["params"]["threadId"].clone())
        .collect();
    ensure!(
        archived.len() == 4
            && archived.iter().filter(|id| **id == json!(GREETING)).count() == 2
            && archived.iter().filter(|id| **id == json!(SUBTRACT)).count() == 2,
        "{archived:?}"
    );
    operation(&mut fx, Operation::View(SUBTRACT.into()))?;
    ensure!(fx.route() == app::Route::Thread(SUBTRACT.into()));
    Ok(())
}
