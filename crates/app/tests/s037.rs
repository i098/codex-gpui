//! S037 `autos-list`: AM-014 (click a task row selects it and opens its thread) and AM-015
//! (row hover shows the list fill and the `Scheduled task actions` button), replayed from
//! `refs/interactions/s037.json` (the original on the route1 Linux stack, two fixture tasks).

#![cfg(target_os = "macos")]

use anyhow::Result;
use app::host::{Host, Request};
use serde_json::{Value, json};

const RECORDING: &str = include_str!("../../../refs/interactions/s037.json");

/// The desktop host of the recording: `list-automations` answers the fixture tasks.
struct Fixture(Value);

impl Host for Fixture {
    fn fetch(&self, request: &Request) -> Result<Value> {
        anyhow::ensure!(
            *request == Request::post("list-automations", None),
            "unexpected host request {request:?}"
        );
        Ok(
            json!({"type": "fetch-response", "responseType": "success", "status": 200,
            "headers": {"content-type": "application/json"}, "bodyJsonString": self.0.to_string()}),
        )
    }
}

#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

#[cfg(target_os = "macos")]
#[test]
fn task_row_hover_and_click_match_original() -> Result<()> {
    use anyhow::{Context as _, ensure};
    use app::Route;
    use common::{Fx, RAIL_SCHEDULED};
    use std::{sync::Arc, time::Duration};

    let recording: Value = serde_json::from_str(RECORDING)?;
    let mut fx = Fx::new()?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(Arc::new(Fixture(recording["fixture"].clone())), cx)
    });
    fx.click(RAIL_SCHEDULED)?;
    fx.wait("host automation list", |fx| {
        fx.read(|root, _| root.automations().is_some())
    })?;
    ensure!(fx.route() == Route::Scheduled, "route {:?}", fx.route());
    let dom = recording["result"]["dom"].as_array().context("dom")?;
    let check = |fx: &mut Fx, entry: &Value| -> Result<()> {
        let image = fx.shot()?;
        let scale = image.width() as f32 / 1280.;
        for sample in entry["pixels"].as_array().context("pixels")? {
            let (x, y) = (
                sample["x"].as_f64().unwrap() as f32,
                sample["y"].as_f64().unwrap() as f32,
            );
            let expected: [u8; 3] = serde_json::from_value(sample["rgb"].clone())?;
            let actual = image.get_pixel((x * scale) as u32, (y * scale) as u32).0;
            // Fills match to the unit; the 16 px ellipsis glyph's center is a raster away.
            let tolerance = if (x, y) == (310., 192.) { 24 } else { 1 };
            ensure!(
                (0..3).all(|c| actual[c].abs_diff(expected[c]) <= tolerance),
                "{}: pixel ({x}, {y}) {actual:?} != {expected:?}",
                entry["note"]
            );
        }
        Ok(())
    };
    // At rest: no fill, no action button.
    fx.hover((700., 600.))?;
    fx.advance(Duration::from_millis(300))?;
    check(&mut fx, &dom[0])?;
    // AM-015: the hovered row gets the list fill and its action button.
    let at = |step: &Value| {
        (
            step["at"]["x"].as_f64().unwrap() as f32,
            step["at"]["y"].as_f64().unwrap() as f32,
        )
    };
    let steps = recording["steps"].as_array().context("steps")?;
    fx.hover(at(&steps[0]))?;
    fx.advance(Duration::from_millis(steps[0]["waitMs"].as_u64().unwrap()))?;
    check(&mut fx, &dom[1])?;
    // AM-014: the click selects the task and opens its thread.
    fx.click(at(&steps[1]))?;
    fx.advance(Duration::from_millis(steps[1]["waitMs"].as_u64().unwrap()))?;
    ensure!(
        fx.route() == Route::Automation("s037-weekly-status".into()),
        "route {:?}",
        fx.route()
    );
    ensure!(fx.shell(|s| s.can_go_back), "history entry");
    ensure!(fx.shell(|s| s.selected_rail) == Some("scheduled"));
    fx.hover(at(&steps[2]))?;
    fx.advance(Duration::from_millis(steps[2]["waitMs"].as_u64().unwrap()))?;
    check(&mut fx, &dom[2])?;
    // The click sends nothing to the app-server (the recording's requests are page-mount
    // reads); the port's sidebar and thread are local state too.
    println!("s037: row hover fill and actions, click selects the task and opens its thread");
    Ok(())
}

struct ObservedLocal {
    local: app::host::LocalHost,
    requests: parking_lot::Mutex<Vec<Request>>,
}

impl Host for ObservedLocal {
    fn fetch(&self, request: &Request) -> Result<Value> {
        self.requests.lock().push(request.clone());
        self.local.fetch(request)
    }
}

impl ObservedLocal {
    fn updates(&self) -> Vec<Value> {
        self.requests
            .lock()
            .iter()
            .filter_map(|request| {
                let Request::Fetch { url, body, .. } = request else {
                    return None;
                };
                (url == "vscode://codex/automation-update")
                    .then(|| serde_json::from_str(body).unwrap())
            })
            .collect()
    }
}

struct ReadOnlyDirectory(std::path::PathBuf);

impl Drop for ReadOnlyDirectory {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755)).ok();
    }
}

fn native_key(fx: &mut common::Fx, key: &str) -> Result<()> {
    let keystroke = gpui::Keystroke::parse(key)?;
    fx.input(gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    }))?;
    fx.input(gpui::PlatformInput::KeyUp(gpui::KeyUpEvent { keystroke }))
}

/// The stored definition without the two keys a status update may change.
fn definition_without_status(home: &std::path::Path, id: &str) -> Result<toml::Table> {
    let path = home.join("automations").join(id).join("automation.toml");
    let mut raw: toml::Table = toml::from_str(&std::fs::read_to_string(path)?)?;
    raw.remove("status");
    raw.remove("updated_at");
    Ok(raw)
}

/// The listed item without the fields a status update may change.
fn item_without_status(list: &Value, id: &str) -> Value {
    let mut item = list["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == id))
        .cloned()
        .unwrap_or_default();
    for key in ["status", "updatedAt", "nextRunAt", "nextRunNominalAt"] {
        item.as_object_mut().map(|item| item.remove(key));
    }
    item
}

fn listed(host: &dyn Host) -> Result<Value> {
    app::host::json_body(&host.fetch(&Request::post("list-automations", None))?)
}

fn listed_status(list: &Value, id: &str) -> Value {
    list["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == id))
        .map_or(Value::Null, |item| item["status"].clone())
}

fn root_status(fx: &common::Fx, id: &str) -> Value {
    fx.read(|root, _| match root.automations() {
        Some(Ok(list)) => listed_status(list, id),
        _ => Value::Null,
    })
}

fn toast_count(fx: &common::Fx, title: &str) -> usize {
    fx.read(|root, cx| {
        root.toasts(cx)
            .iter()
            .filter(|t| t.as_ref() == title)
            .count()
    })
}

#[test]
fn task_status_controls_persist_through_the_real_local_host() -> Result<()> {
    use anyhow::ensure;
    use common::{Fx, RAIL_SCHEDULED};
    use parking_lot::Mutex;
    use std::{fs, os::unix::fs::PermissionsExt as _, sync::Arc};
    let scratch = std::path::Path::new("/tmp/cgx-mac/task-management");
    fs::create_dir_all(scratch)?;
    let output = std::env::var_os("CGX_TASK_MANAGEMENT_PROOF").map(std::path::PathBuf::from);
    if let Some(output) = &output {
        ensure!(
            output.starts_with(scratch),
            "proof output escapes the task-owned scratch"
        );
        fs::create_dir_all(output)?;
    }
    let shot = |fx: &mut Fx, name: &str| -> Result<()> {
        if let Some(output) = &output {
            fx.shot()?.save(output.join(format!("{name}.png")))?;
        }
        Ok(())
    };
    let home = tempfile::Builder::new()
        .prefix("status-controls-")
        .tempdir_in(scratch)?;
    let (cron, heartbeat) = ("status-cron", "status-heartbeat");
    let workspace = scratch.join("workspace");
    fs::create_dir_all(home.path().join("sqlite"))?;
    let db = rusqlite::Connection::open(home.path().join("sqlite/codex.db"))?;
    db.execute_batch(
        "CREATE TABLE automations (id TEXT PRIMARY KEY, name TEXT, prompt TEXT, status TEXT,
         next_run_at INTEGER, next_run_nominal_at INTEGER, last_run_at INTEGER, cwds TEXT,
         target_type TEXT, project_id TEXT, rrule TEXT, model TEXT, reasoning_effort TEXT,
         created_at INTEGER, updated_at INTEGER, kind TEXT, target_thread_id TEXT,
         execution_environment TEXT, local_environment_config_path TEXT, plugin_template_id TEXT,
         notification_policy TEXT, auto_archive INTEGER, account_id TEXT, user_id TEXT,
         installation_id TEXT);",
    )?;
    let local = app::host::LocalHost::new(home.path());
    // A cached run from yesterday: resume must recalculate, not reuse it.
    let cached = chrono::Utc::now().timestamp_millis() - 86_400_000;
    for (id, kind, status, updated) in [
        (cron, "cron", "ACTIVE", 20),
        (heartbeat, "heartbeat", "PAUSED", 10),
    ] {
        local.write_automation(id, &format!(
            "version = 1\nid = \"{id}\"\nkind = \"{kind}\"\nname = \"{id}\"\nprompt = \"Keep this prompt\"\nstatus = \"{status}\"\nrrule = \"FREQ=HOURLY;INTERVAL=24;BYMINUTE=0\"\ncwds = [\"{}\"]\ntarget_thread_id = \"fixture-thread\"\nnotification_policy = \"failed_runs_only\"\ncreated_at = 1\nupdated_at = {updated}\n",
            workspace.display(),
        ))?;
        db.execute(
            "INSERT INTO automations (id, status, next_run_at, next_run_nominal_at, last_run_at, rrule)
             VALUES (?1, ?2, ?3, ?3, 5, 'FREQ=HOURLY;INTERVAL=24;BYMINUTE=0')",
            (id, status, cached),
        )?;
    }
    let host = Arc::new(ObservedLocal {
        local,
        requests: Mutex::new(Vec::new()),
    });
    let before = listed(host.as_ref())?;
    let cron_definition = definition_without_status(home.path(), cron)?;
    let heartbeat_toml = fs::read(
        home.path()
            .join("automations/status-heartbeat/automation.toml"),
    )?;
    /// `(status, next_run_at, next_run_nominal_at, last_run_at)`.
    type Row = (String, Option<i64>, Option<i64>, Option<i64>);
    let db_row = |id: &str| -> Result<Row> {
        Ok(db.query_row(
            "SELECT status, next_run_at, next_run_nominal_at, last_run_at FROM automations WHERE id = ?",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?)
    };

    let mut fx = Fx::new()?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.click(RAIL_SCHEDULED)?;
    fx.wait("local task definitions", |fx| {
        fx.read(|root, _| root.automations().is_some())
    })?;
    shot(&mut fx, "1-before")?;

    // AM-022: pointer opens the row menu; Escape sends nothing; the keyboard reopens it and pauses.
    // Home selects the first item, which is the status item in the original order (`statusFirst`).
    fx.click((310., 192.))?;
    shot(&mut fx, "2-menu")?;
    native_key(&mut fx, "escape")?;
    ensure!(host.updates().is_empty(), "Escape updated an automation");
    native_key(&mut fx, "enter")?;
    native_key(&mut fx, "home")?;
    native_key(&mut fx, "enter")?;
    fx.wait("pause success", |fx| {
        toast_count(fx, "Successfully paused automation") == 1
    })?;
    fx.wait("refetched pause", |fx| root_status(fx, cron) == "PAUSED")?;
    ensure!(
        fx.route() == app::Route::Scheduled,
        "the status menu opened a task thread"
    );
    let updates = host.updates();
    ensure!(
        updates.len() == 1,
        "one menu action sent {} writes",
        updates.len()
    );
    ensure!(
        updates[0]
            == json!({
                "id": cron, "kind": "cron", "autoArchive": false, "name": cron,
                "prompt": "Keep this prompt", "status": "PAUSED", "projectId": workspace.display().to_string(),
                "executionEnvironment": "worktree", "localEnvironmentConfigPath": null, "model": null,
                "reasoningEffort": null, "notificationPolicy": "failed_runs_only",
                "rrule": "FREQ=HOURLY;INTERVAL=24;BYMINUTE=0",
            }),
        "pause request differs from the original body: {}",
        updates[0]
    );
    ensure!(
        db_row(cron)? == ("PAUSED".into(), None, None, Some(5)),
        "database pause"
    );
    ensure!(
        definition_without_status(home.path(), cron)? == cron_definition,
        "pause changed the definition"
    );
    shot(&mut fx, "3-paused")?;
    drop(fx);

    // A fresh host and app start read the persisted pause.
    let fresh = Arc::new(app::host::LocalHost::new(home.path()));
    let restarted = listed(fresh.as_ref())?;
    ensure!(listed_status(&restarted, cron) == "PAUSED");
    ensure!(restarted["items"][0]["nextRunAt"].is_null());
    ensure!(item_without_status(&restarted, cron) == item_without_status(&before, cron));
    let host = Arc::new(ObservedLocal {
        local: app::host::LocalHost::new(home.path()),
        requests: Mutex::new(host.requests.lock().clone()),
    });
    let mut fx = Fx::new()?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.click(RAIL_SCHEDULED)?;
    fx.wait("restarted list", |fx| root_status(fx, cron) == "PAUSED")?;
    shot(&mut fx, "4-restarted")?;

    // AM-023 and AM-028: the inline row control resumes and recalculates the next run.
    let start = chrono::Utc::now().timestamp_millis();
    fx.click((282., 192.))?;
    fx.wait("resume success", |fx| {
        toast_count(fx, "Successfully resumed automation") == 1
    })?;
    fx.wait("refetched resume", |fx| root_status(fx, cron) == "ACTIVE")?;
    let updates = host.updates();
    ensure!(updates.len() == 2 && updates[1]["status"] == "ACTIVE" && updates[1]["id"] == cron);
    let (status, next, nominal, last) = db_row(cron)?;
    ensure!(status == "ACTIVE" && last == Some(5), "database resume");
    ensure!(
        next.is_some_and(|next| next > start) && nominal.is_some_and(|n| n > start),
        "resume kept a stale next run"
    );
    let resumed = listed(fresh.as_ref())?;
    ensure!(item_without_status(&resumed, cron) == item_without_status(&before, cron));
    ensure!(
        definition_without_status(home.path(), cron)? == cron_definition,
        "resume changed the definition"
    );
    shot(&mut fx, "5-resumed")?;

    // An actual local write failure keeps the stored state and shows the host error.
    let directory = home.path().join("automations").join(heartbeat);
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555))?;
    let restore = ReadOnlyDirectory(directory);
    fx.click((282., 241.))?;
    fx.wait("update error", |fx| {
        toast_count(fx, "Could not update scheduled task") == 1
    })?;
    drop(restore);
    let updates = host.updates();
    ensure!(
        updates.len() == 3 && updates[2]["id"] == heartbeat && updates[2]["status"] == "ACTIVE"
    );
    ensure!(updates[2]["targetThreadId"] == "fixture-thread");
    ensure!(updates[2]["model"].is_null() && updates[2]["reasoningEffort"].is_null());
    ensure!(
        toast_count(&fx, "Successfully resumed automation") == 1,
        "failure reported success"
    );
    fx.wait("refetched failure", |fx| {
        root_status(fx, heartbeat) == "PAUSED"
    })?;
    ensure!(
        db_row(heartbeat)? == ("PAUSED".into(), Some(cached), Some(cached), Some(5)),
        "failed write changed the database"
    );
    ensure!(
        fs::read(
            home.path()
                .join("automations/status-heartbeat/automation.toml")
        )? == heartbeat_toml
    );
    ensure!(
        listed(fresh.as_ref())?["items"][1] == before["items"][1],
        "failed write changed the list"
    );
    shot(&mut fx, "6-error")?;
    if let Some(output) = &output {
        fs::write(
            output.join("status-update-requests.json"),
            serde_json::to_vec_pretty(&updates)?,
        )?;
    }
    println!("s037: pause, restart, resume and a real write failure through the local host");
    Ok(())
}
