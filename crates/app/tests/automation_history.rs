#![cfg(target_os = "macos")]

mod common;

use anyhow::Result;
use app::host::LocalHost;
use common::*;
use gpui::Focusable as _;
use rusqlite::Connection;
use serde_json::json;
use std::{fs, sync::Arc};
use ui::screens::automations::OpenAutomation;

#[test]
fn proxy() {
    proxy_child();
}

fn history_keys(fx: &mut Fx, keys: &str) -> Result<()> {
    let focus = fx.read(|root, cx| root.run_history(cx).unwrap().focus_handle(cx));
    fx.cx
        .update_window(fx.window, |_, window, cx| window.focus(&focus, cx))?;
    fx.keys(keys)
}

#[test]
fn previous_runs_confirm_cancel_and_persist_partial_results() -> Result<()> {
    let home = tempfile::tempdir()?;
    fs::create_dir_all(home.path().join("sqlite"))?;
    let db = Connection::open(home.path().join("sqlite/codex.db"))?;
    db.execute_batch("CREATE TABLE automations (id TEXT PRIMARY KEY, name TEXT, prompt TEXT,
        next_run_at INTEGER, next_run_nominal_at INTEGER, last_run_at INTEGER, rrule TEXT,
        status TEXT, account_id TEXT, user_id TEXT, installation_id TEXT);
        CREATE TABLE automation_runs (thread_id TEXT PRIMARY KEY, automation_id TEXT,
        status TEXT, read_at INTEGER, thread_title TEXT, source_cwd TEXT, inbox_title TEXT,
        inbox_summary TEXT, archived_assistant_message TEXT, archived_user_message TEXT,
        archived_reason TEXT, created_at INTEGER, updated_at INTEGER);
        INSERT INTO automations (id,name,prompt,status,rrule) VALUES ('owned-review','Review changes','Check the project','ACTIVE','FREQ=DAILY;BYHOUR=9');")?;
    for (thread, status, created) in [
        (GREETING, "PENDING_REVIEW", 4),
        (SUBTRACT, "ACCEPTED", 3),
        (EXPLAIN, "IN_PROGRESS", 2),
        ("already-archived", "ARCHIVED", 1),
    ] {
        db.execute(
            "INSERT INTO automation_runs (thread_id,automation_id,status,thread_title,created_at)
            VALUES (?,'owned-review',?,'Review changes',?)",
            (thread, status, created),
        )?;
    }
    let host = Arc::new(LocalHost::new(home.path()));
    host.write_automation("owned-review", "version = 1\nid = \"owned-review\"\nname = \"Review changes\"\nprompt = \"Check the project\"\nstatus = \"ACTIVE\"\nrrule = \"FREQ=DAILY;BYHOUR=9\"\ncwds = []\ncreated_at = 1792083600000\nupdated_at = 1792083600000\n")?;
    let mut lines = script(None, |_| {})?;
    for attempt in 0..3 {
        for (index, thread) in [GREETING, SUBTRACT].into_iter().enumerate() {
            let id = 90000 + attempt * 10 + index;
            lines.push(json!({"dir":"send","msg":{"id":id,"method":"thread/archive","params":{"threadId":thread}}}));
            lines.push(if index == 0 {
                json!({"dir":"recv","msg":{"id":id,"result":{}}})
            } else {
                json!({"dir":"recv","msg":{"id":id,"error":{"code":-32000,"message":"owned refusal"}}})
            });
        }
    }
    for index in 0..2 {
        let id = 91000 + index;
        lines.push(json!({"dir":"send","msg":{"id":id,"method":"thread/unarchive","params":{"threadId":GREETING}}}));
        lines.push(if index == 0 {
            json!({"dir":"recv","msg":{"id":id,"result":{}}})
        } else {
            json!({"dir":"recv","msg":{"id":id,"error":{"code":-32000,"message":"owned rollback refusal"}}})
        });
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.action(Box::new(ui::OpenScheduled))?;
    fx.wait("task list", |fx| {
        fx.read(|root, _| matches!(root.automations(), Some(Ok(list)) if list["items"][0]["id"] == "owned-review"))
    })?;
    fx.action(Box::new(OpenAutomation {
        id: "owned-review".into(),
    }))?;
    fx.wait("stored history", |fx| {
        fx.read(|root, cx| {
            root.run_history(cx)
                .is_some_and(|h| h.read(cx).items().len() == 4)
        })
    })?;
    let status = |thread| {
        db.query_row(
            "SELECT status FROM automation_runs WHERE thread_id=?",
            [thread],
            |r| r.get::<_, String>(0),
        )
    };
    let archives = |fx: &mut Fx| -> Result<Vec<serde_json::Value>> {
        Ok(fx
            .sent()?
            .into_iter()
            .filter(|r| r["method"] == "thread/archive")
            .collect())
    };
    let shots = std::env::var_os("CGX_ARCHIVE_SHOTS").map(std::path::PathBuf::from);
    history_keys(&mut fx, "space down")?;
    if let Some(dir) = &shots {
        fs::create_dir_all(dir)?;
        fx.shot()?.save(dir.join("native-menu.png"))?;
    }
    fx.keys("enter")?;
    assert_eq!(
        fx.read(|root, cx| root.dialog().unwrap().read(cx).kind()),
        ui::dialog::Kind::ArchiveRuns { count: 2 }
    );
    if let Some(dir) = &shots {
        fx.shot()?.save(dir.join("native-confirm.png"))?;
    }
    fx.keys("escape")?;
    assert!(fx.read(|root, _| root.dialog().is_none()));
    assert!(archives(&mut fx)?.is_empty());
    assert_eq!(status(GREETING)?, "PENDING_REVIEW");
    history_keys(&mut fx, "space down enter")?;
    fx.keys("enter")?;
    assert!(fx.read(|root, _| root.dialog().is_none()));
    assert!(archives(&mut fx)?.is_empty());
    db.execute_batch("CREATE TRIGGER refuse_read BEFORE UPDATE ON automation_runs BEGIN SELECT RAISE(ABORT,'owned read refusal'); END;")?;
    history_keys(&mut fx, "space enter")?;
    fx.wait("read error toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .contains(&"Could not mark all runs as read".into())
        })
    })?;
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM automation_runs WHERE read_at IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        0
    );
    db.execute_batch("DROP TRIGGER refuse_read;")?;
    history_keys(&mut fx, "space enter")?;
    fx.wait("read state reload", |fx| {
        fx.read(|root, cx| {
            root.run_history(cx)
                .unwrap()
                .read(cx)
                .items()
                .iter()
                .all(|r| !r["readAt"].is_null())
        })
    })?;
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM automation_runs WHERE read_at IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )?,
        4
    );
    db.execute_batch("CREATE TRIGGER refuse_archive BEFORE UPDATE OF status ON automation_runs WHEN NEW.status = 'ARCHIVED' BEGIN SELECT RAISE(ABORT,'owned archive refusal'); END;")?;
    for failures in 1..=2 {
        history_keys(&mut fx, "space down enter")?;
        fx.keys("tab enter")?;
        fx.wait("archive persistence refusal", |fx| {
            fx.read(|root, cx| {
                root.toasts(cx)
                    .iter()
                    .filter(|title| title.as_ref() == "Archived 0; could not archive 2")
                    .count()
                    == failures
            })
        })?;
        assert_eq!(status(GREETING)?, "PENDING_REVIEW");
        assert_eq!(
            fx.read(|root, _| root.recents().iter().any(|id| id == GREETING)),
            failures == 1
        );
    }
    db.execute_batch("DROP TRIGGER refuse_archive;")?;
    history_keys(&mut fx, "space down enter")?;
    fx.keys("tab")?;
    fx.keys("enter")?;
    fx.wait("partial archive toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .contains(&"Archived 1; could not archive 1".into())
        })
    })?;
    assert_eq!(status(GREETING)?, "ARCHIVED");
    assert_eq!(status(SUBTRACT)?, "ACCEPTED");
    assert_eq!(status(EXPLAIN)?, "IN_PROGRESS");
    assert_eq!(
        archives(&mut fx)?,
        vec![
            json!({"method":"thread/archive","params":{"threadId":GREETING}}),
            json!({"method":"thread/archive","params":{"threadId":SUBTRACT}}),
            json!({"method":"thread/archive","params":{"threadId":GREETING}}),
            json!({"method":"thread/archive","params":{"threadId":SUBTRACT}}),
            json!({"method":"thread/archive","params":{"threadId":GREETING}}),
            json!({"method":"thread/archive","params":{"threadId":SUBTRACT}})
        ]
    );
    if let Some(dir) = &shots {
        fx.shot()?.save(dir.join("native-partial.png"))?;
    }
    println!(
        "Owned results: Cancel and Escape write nothing. Read failures preserve unread state. Archive write and rollback failures keep stored rows."
    );
    Ok(())
}
