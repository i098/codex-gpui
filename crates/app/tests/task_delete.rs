use anyhow::{Result, ensure};
use app::host::{Host, LocalHost, Request, json_body};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::fs;

fn definition(id: &str, name: &str, updated: i64) -> String {
    format!(
        r#"version = 1
id = "{id}"
kind = "cron"
name = "{name}"
prompt = "Check the project"
status = "ACTIVE"
rrule = "FREQ=DAILY;BYHOUR=9"
cwds = ["/tmp/project"]
created_at = 10
updated_at = {updated}
"#
    )
}

fn delete(host: &LocalHost, id: &str) -> Result<Value> {
    json_body(&host.fetch(&Request::post("automation-delete", Some(json!({"id": id}))))?)
}

fn ids(host: &LocalHost) -> Result<Vec<String>> {
    let body = json_body(&host.fetch(&Request::post("list-automations", None))?)?;
    Ok(body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect())
}

#[test]
fn delete_removes_only_the_confirmed_local_task() -> Result<()> {
    let home = tempfile::tempdir()?;
    fs::create_dir_all(home.path().join("sqlite"))?;
    let db = Connection::open(home.path().join("sqlite/codex.db"))?;
    db.execute_batch(
        "CREATE TABLE automations (
        id TEXT PRIMARY KEY, name TEXT, prompt TEXT, next_run_at INTEGER, next_run_nominal_at INTEGER,
        last_run_at INTEGER, rrule TEXT, status TEXT, account_id TEXT, user_id TEXT, installation_id TEXT);
        CREATE TABLE automation_runs (thread_id TEXT PRIMARY KEY, automation_id TEXT);
        INSERT INTO automations VALUES ('first', 'First', 'p', 1, 1, 1, 'FREQ=DAILY;BYHOUR=9', 'ACTIVE', NULL, NULL, NULL);
        INSERT INTO automations VALUES ('second', 'Second', 'p', 1, 1, 1, 'FREQ=DAILY;BYHOUR=9', 'ACTIVE', NULL, NULL, NULL);
        INSERT INTO automations VALUES ('private', 'Other', 'p', 1, 1, 1, '', 'ACTIVE', 'account', 'user', 'installation');
        INSERT INTO automation_runs VALUES ('run-first', 'first'), ('run-second', 'second'), ('run-private', 'private');",
    )?;
    let host = LocalHost::new(home.path());
    host.write_automation("first", &definition("first", "First", 20))?;
    host.write_automation("second", &definition("second", "Second", 30))?;
    let private = home.path().join("automations/private");
    fs::create_dir_all(&private)?;
    fs::write(
        private.join("automation.toml"),
        definition("private", "Other", 40),
    )?;
    let outside = home.path().join("outside");
    fs::create_dir_all(&outside)?;
    fs::write(outside.join("automation.toml"), "keep")?;
    let count = |sql: &str| db.query_row(sql, [], |row| row.get::<_, i64>(0));

    let answer = delete(&host, "first")?;
    ensure!(answer["success"] == true && answer["status"] == "deleted");
    ensure!(answer["item"]["id"] == "first" && answer["item"]["name"] == "First");
    ensure!(!home.path().join("automations/first").exists());
    ensure!(ids(&host)? == ["second"]);
    ensure!(count("SELECT COUNT(*) FROM automations WHERE id = 'first'")? == 0);
    ensure!(count("SELECT COUNT(*) FROM automation_runs WHERE automation_id = 'first'")? == 0);
    ensure!(count("SELECT COUNT(*) FROM automation_runs WHERE automation_id = 'second'")? == 1);

    let again = delete(&host, "first")?;
    ensure!(again["success"] == true && again["status"] == "not_found" && again["item"].is_null());
    let foreign = delete(&host, "private")?;
    ensure!(foreign["status"] == "not_found");
    ensure!(private.join("automation.toml").exists());
    ensure!(count("SELECT COUNT(*) FROM automations WHERE id = 'private'")? == 1);
    ensure!(count("SELECT COUNT(*) FROM automation_runs WHERE automation_id = 'private'")? == 1);

    for id in ["", ".", "..", "../outside", "a/b", "a\\b"] {
        let refused = delete(&host, id)?;
        ensure!(
            refused["success"] == false && refused["status"] == "invalid_id",
            "{id}"
        );
    }
    ensure!(outside.join("automation.toml").exists());
    ensure!(
        host.fetch(&Request::post("automation-delete", Some(json!({}))))
            .is_err()
    );
    ensure!(
        host.fetch(&Request::post(
            "automation-delete",
            Some(json!({"id": "second", "hostId": "remote"}))
        ))
        .is_err()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, home.path().join("automations/link"))?;
        ensure!(delete(&host, "link").is_err());
        ensure!(outside.join("automation.toml").exists());
    }
    ensure!(ids(&host)? == ["second"]);
    Ok(())
}

#[test]
fn delete_works_without_the_original_database() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    host.write_automation("only", &definition("only", "Only", 20))?;
    ensure!(delete(&host, "only")?["status"] == "deleted");
    ensure!(ids(&host)?.is_empty());
    ensure!(delete(&host, "only")?["status"] == "not_found");
    Ok(())
}

#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

/// The row's `Scheduled task actions` button of the first and second row.
#[cfg(target_os = "macos")]
const ROW_ACTIONS: [(f32, f32); 2] = [(310., 190.), (310., 239.)];

#[cfg(target_os = "macos")]
fn open_dialog(fx: &mut common::Fx, row: usize) -> Result<()> {
    fx.click(ROW_ACTIONS[row])?;
    // The original's menu hangs under the button: right edges 1 px apart, 2 px below it
    // (captured: button x 296..324 y 178..206 of row 1, menu right 323, top 208).
    let corner = fx.read(|r, cx| r.scheduled().and_then(|s| s.read(cx).menu_corner()));
    let corner = corner.map(|p| (f32::from(p.x), f32::from(p.y)));
    ensure!(
        corner == Some((323., 208. + 49. * row as f32)),
        "menu corner {corner:?}"
    );
    if let Ok(output) = std::env::var("CGX_TASK_DELETE_SHOTS") {
        fs::create_dir_all(&output)?;
        let path = format!("{output}/menu.png");
        if !std::path::Path::new(&path).exists() {
            fx.shot()?.save(path)?;
        }
    }
    fx.keys("enter")?;
    ensure!(
        fx.read(|r, cx| {
            r.dialog()
                .is_some_and(|d| d.read(cx).kind() == ui::dialog::Kind::DeleteTask)
        }),
        "Delete did not open its confirmation"
    );
    Ok(())
}

/// Whether task `id`'s `Scheduled task actions` button has focus.
#[cfg(target_os = "macos")]
fn trigger_focused(fx: &mut common::Fx, id: &str) -> Result<bool> {
    let sidebar = fx.read(|r, _| r.scheduled().cloned());
    let handle = sidebar.and_then(|s| fx.read(|_, cx| s.read(cx).trigger_focus(id)));
    let Some(handle) = handle else {
        return Ok(false);
    };
    fx.cx
        .update_window(fx.window, |_, window, _| handle.is_focused(window))
}

#[cfg(target_os = "macos")]
#[test]
fn confirmed_delete_removes_the_task_and_cancel_keeps_it() -> Result<()> {
    use common::{Fx, RAIL_SCHEDULED};
    use std::{os::unix::fs::PermissionsExt, sync::Arc};
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    host.write_automation("alpha", &definition("alpha", "Alpha task", 30))?;
    host.write_automation("beta", &definition("beta", "Beta task", 20))?;
    let alpha = home.path().join("automations/alpha");
    let beta = home.path().join("automations/beta");
    let mut fx = Fx::new()?;
    fx.cx.update(|cx| {
        fx.root
            .update(cx, |root, cx| root.set_host(Arc::new(host), cx))
    });
    fx.click(RAIL_SCHEDULED)?;
    let listed = |fx: &mut Fx, wanted: usize| {
        fx.wait("task list", |fx| {
            fx.read(|r, _| {
                r.automations().is_some_and(|list| {
                    list.as_ref()
                        .is_ok_and(|list| list["items"].as_array().unwrap().len() == wanted)
                })
            })
        })
    };
    listed(&mut fx, 2)?;

    // The confirmation names the clicked row's task and focuses `Cancel`.
    open_dialog(&mut fx, 0)?;
    ensure!(fx.read(|r, cx| {
        let dialog = r.dialog().unwrap().read(cx);
        dialog.task() == "alpha"
            && dialog.query() == "Alpha task"
            && dialog.focused_control() == "Cancel"
    }));
    if let Ok(output) = std::env::var("CGX_TASK_DELETE_SHOTS") {
        fs::create_dir_all(&output)?;
        fx.shot()?.save(format!("{output}/confirm.png"))?;
    }
    // Escape cancels and returns focus to that row's button.
    fx.keys("escape")?;
    ensure!(fx.read(|r, _| r.dialog().is_none()) && alpha.join("automation.toml").exists());
    ensure!(
        trigger_focused(&mut fx, "alpha")?,
        "focus must return to the row button"
    );
    open_dialog(&mut fx, 0)?;
    ensure!(fx.read(|r, cx| {
        r.dialog().is_some_and(|d| {
            d.read(cx).task() == "alpha" && d.read(cx).focused_control() == "Cancel"
        })
    }));
    // Cancel keeps the task as well.
    fx.keys("enter")?;
    ensure!(fx.read(|r, _| r.dialog().is_none()) && alpha.join("automation.toml").exists());
    listed(&mut fx, 2)?;

    // The second row asks about its own task; confirming deletes exactly that one.
    open_dialog(&mut fx, 1)?;
    ensure!(fx.read(|r, cx| r.dialog().unwrap().read(cx).task() == "beta"));
    fx.keys("escape")?;
    open_dialog(&mut fx, 0)?;
    fx.keys("tab")?;
    ensure!(
        fx.read(|r, cx| r.dialog().unwrap().read(cx).focused_control() == "Delete scheduled task")
    );
    fx.keys("enter")?;
    fx.wait("deleted task gone", |fx| {
        fx.read(|r, _| r.dialog().is_none()) && !alpha.exists()
    })?;
    listed(&mut fx, 1)?;
    ensure!(!alpha.exists() && beta.join("automation.toml").exists());

    // A failed removal reports itself, keeps the task, and can be retried.
    fx.click((150., 190.))?;
    fx.wait("task page", |fx| {
        fx.route() == app::Route::Automation("beta".into())
    })?;
    fs::set_permissions(&beta, fs::Permissions::from_mode(0o555))?;
    open_dialog(&mut fx, 0)?;
    fx.keys("tab enter")?;
    fx.wait("failure toast", |fx| {
        fx.read(|r, cx| r.toasts(cx) == ["Could not delete scheduled task"])
    })?;
    ensure!(fx.read(|r, _| r.dialog().is_none()));
    ensure!(beta.join("automation.toml").exists());
    ensure!(fx.route() == app::Route::Automation("beta".into()));
    fs::set_permissions(&beta, fs::Permissions::from_mode(0o755))?;
    open_dialog(&mut fx, 0)?;
    fx.keys("tab enter")?;
    fx.wait("retry deletes", |fx| fx.route() == app::Route::Scheduled)?;
    listed(&mut fx, 0)?;
    ensure!(!beta.exists());
    println!(
        "Native task delete: Escape and Cancel kept the task, confirm deleted only the named task, a failed removal toasted and kept it, the retry deleted it and closed its page."
    );
    Ok(())
}
