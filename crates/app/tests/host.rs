use anyhow::Result;
use app::host::{Host, LocalHost, ReplayHost, Request, json_body};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::fs;

const RECORDING: &str = include_str!("../../../refs/interactions/host.json");
const ORACLE: &str = include_str!("../../../tools/route1/oracle.json");

#[test]
fn all_original_host_and_http_responses_replay_exactly() -> Result<()> {
    let recorded = ReplayHost::from_recording(RECORDING)?;
    let oracle = ReplayHost::from_oracle(ORACLE)?;
    let recording: Value = serde_json::from_str(RECORDING)?;
    let source: serde_json::Map<String, Value> = serde_json::from_str(ORACLE)?;
    assert_eq!(
        recording["exchanges"].as_array().unwrap().len(),
        source
            .keys()
            .filter(|key| key.starts_with("fetch ") || key.starts_with("svc httpFetch.fetch "))
            .count()
    );
    for exchange in recording["exchanges"].as_array().unwrap() {
        let request: Request = serde_json::from_value(exchange["request"].clone())?;
        assert_eq!(recorded.fetch(&request)?, exchange["response"]);
        assert_eq!(oracle.fetch(&request)?, exchange["response"]);
    }
    assert_eq!(
        json_body(&recorded.fetch(&Request::post("list-automations", None))?)?,
        json!({"items": []})
    );
    assert!(
        recorded
            .fetch(&Request::post(
                "list-automations",
                Some(json!({"unexpected": true}))
            ))
            .is_err()
    );
    assert!(
        recorded
            .fetch(&Request::post("not-recorded", None))
            .is_err()
    );
    let mut http = recording["exchanges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["request"]["transport"] == "http")
        .unwrap()["request"]["request"]
        .clone();
    http["headers"]["ChatGPT-Account-ID"] = json!("another-account");
    assert!(recorded.fetch(&Request::Http(http)).is_err());
    Ok(())
}

#[test]
fn wire_errors_and_binary_responses_are_not_empty_json_successes() -> Result<()> {
    let recording: Value = serde_json::from_str(RECORDING)?;
    let responses = recording["exchanges"].as_array().unwrap();
    let error = responses
        .iter()
        .find(|e| e["response"]["value"]["responseType"] == "error")
        .unwrap();
    assert!(
        json_body(&error["response"])
            .unwrap_err()
            .to_string()
            .contains("Authenticated principal changed")
    );
    let binary = responses
        .iter()
        .find(|e| {
            e["response"]
                .pointer("/value/response/__response/base64")
                .is_some()
        })
        .unwrap();
    assert!(json_body(&binary["response"]).is_err());
    Ok(())
}

fn definition(id: &str, updated: i64) -> String {
    format!(
        r#"version = 1
id = "{id}"
kind = "cron"
name = "Local task"
prompt = "Check the project"
status = "PAUSED"
rrule = "FREQ=DAILY;BYHOUR=9"
cwds = ["/tmp/project"]
created_at = 10
updated_at = {updated}
"#
    )
}

#[test]
fn definitions_round_trip_and_reject_escape_without_damaging_existing_data() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    assert_eq!(
        json_body(&host.fetch(&Request::post("list-automations", None))?)?,
        json!({"items": []})
    );
    host.write_automation("first", &definition("first", 20))?;
    host.write_automation("second", &definition("second", 30))?;
    let body = json_body(&host.fetch(&Request::post("list-automations", None))?)?;
    assert_eq!(body["items"][0]["id"], "second");
    assert_eq!(body["items"][1]["id"], "first");
    assert_eq!(body["items"][0]["executionEnvironment"], "worktree");
    assert_eq!(body["items"][0]["nextRunAt"], Value::Null);
    assert_eq!(body["items"][0]["cwds"], json!(["/tmp/project"]));
    let path = home.path().join("automations/first/automation.toml");
    let before = fs::read(&path)?;
    assert!(
        host.write_automation("first", &definition("mismatch", 40))
            .is_err()
    );
    assert!(
        host.write_automation("../outside", &definition("../outside", 40))
            .is_err()
    );
    assert!(host.write_automation("first", "invalid TOML").is_err());
    assert_eq!(fs::read(path)?, before);
    host.write_automation("first", &definition("first", 40))?;
    assert_eq!(
        json_body(&host.fetch(&Request::post("list-automations", None))?)?["items"][0]["id"],
        "first"
    );
    assert!(!home.path().join("sqlite/codex.db").exists());
    assert!(
        host.fetch(&Request::Http(json!({"method":"GET","url":"/wham/usage"})))
            .unwrap_err()
            .to_string()
            .contains("unsupported")
    );
    Ok(())
}

fn database(home: &std::path::Path) -> Result<Connection> {
    fs::create_dir_all(home.join("sqlite"))?;
    let db = Connection::open(home.join("sqlite/codex.db"))?;
    db.execute_batch("CREATE TABLE automations (
        id TEXT PRIMARY KEY, name TEXT, prompt TEXT, next_run_at INTEGER, next_run_nominal_at INTEGER,
        last_run_at INTEGER, rrule TEXT, status TEXT, account_id TEXT, user_id TEXT, installation_id TEXT);
        CREATE TABLE automation_runs (
        thread_id TEXT PRIMARY KEY, automation_id TEXT, status TEXT, read_at INTEGER,
        thread_title TEXT, source_cwd TEXT, inbox_title TEXT, inbox_summary TEXT,
        archived_assistant_message TEXT, archived_user_message TEXT, archived_reason TEXT,
        created_at INTEGER, updated_at INTEGER);
        INSERT INTO automations VALUES ('local', 'Local task', 'Fallback prompt', 100, 100, 50, 'FREQ=DAILY;BYHOUR=9', 'ACTIVE', NULL, NULL, NULL);
        INSERT INTO automations VALUES ('private', 'Other account', 'Secret', 100, 100, 50, '', 'ACTIVE', 'account', 'user', 'installation');
        INSERT INTO automation_runs (thread_id, automation_id, status, created_at, inbox_summary) VALUES
        ('running', 'local', 'IN_PROGRESS', 1, ''),
        ('review', 'local', 'PENDING_REVIEW', 2, 'Review this'),
        ('accepted', 'local', 'ACCEPTED', 3, NULL),
        ('archived', 'local', 'ARCHIVED', 4, NULL),
        ('private-run', 'private', 'PENDING_REVIEW', 5, 'Secret');")?;
    Ok(db)
}

#[test]
fn inbox_preserves_order_filtering_counts_and_account_isolation() -> Result<()> {
    let home = tempfile::tempdir()?;
    let db = database(home.path())?;
    let host = LocalHost::new(home.path());
    let request = Request::post("inbox-items", Some(json!({"limit": 2})));
    let body = json_body(&host.fetch(&request)?)?;
    assert_eq!(body["items"][0]["id"], "running");
    assert_eq!(body["items"][0]["description"], "Fallback prompt");
    assert_eq!(body["items"][1]["id"], "review");
    assert_eq!(body["unreadRunCounts"]["total"], 2);
    assert_eq!(body["unreadRunCounts"]["automationIds"], json!(["local"]));
    let filtered = json_body(&host.fetch(&Request::post(
        "inbox-items",
        Some(json!({"threadId": "accepted"})),
    ))?)?;
    assert_eq!(filtered["items"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["items"][0]["id"], "accepted");
    assert_eq!(filtered["unreadRunCounts"]["total"], 2);
    assert!(host.mark_inbox_read("review", Some(1234))?);
    assert_eq!(
        json_body(&host.fetch(&request)?)?["unreadRunCounts"]["total"],
        1
    );
    assert!(host.mark_inbox_read("review", None)?);
    assert_eq!(
        json_body(&host.fetch(&request)?)?["unreadRunCounts"]["total"],
        2
    );
    assert!(!host.mark_inbox_read("private-run", Some(1234))?);
    assert_eq!(
        db.query_row(
            "SELECT read_at FROM automation_runs WHERE thread_id='private-run'",
            [],
            |r| r.get::<_, Option<i64>>(0)
        )?,
        None
    );
    assert!(
        host.fetch(&Request::post("inbox-items", Some(json!({"limit": -1}))))
            .is_err()
    );
    Ok(())
}

#[test]
fn local_definitions_use_matching_schedule_metadata_not_another_accounts_cache() -> Result<()> {
    let home = tempfile::tempdir()?;
    let _db = database(home.path())?;
    let host = LocalHost::new(home.path());
    host.write_automation(
        "local",
        &definition("local", 20).replace("PAUSED", "ACTIVE"),
    )?;
    fs::create_dir_all(home.path().join("automations/private"))?;
    fs::write(
        home.path().join("automations/private/automation.toml"),
        definition("private", 30),
    )?;
    assert!(
        host.write_automation("private", &definition("private", 40))
            .is_err()
    );
    let body = json_body(&host.fetch(&Request::post("list-automations", None))?)?;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["id"], "local");
    assert_eq!(body["items"][0]["nextRunAt"], 100);
    assert_eq!(body["items"][0]["lastRunAt"], 50);
    host.write_automation("local", &definition("local", 40))?;
    assert_eq!(
        json_body(&host.fetch(&Request::post("list-automations", None))?)?["items"][0]["nextRunAt"],
        Value::Null
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

#[cfg(target_os = "macos")]
#[test]
fn scheduled_navigation_loads_the_original_host_response() -> Result<()> {
    use parking_lot::Mutex;
    use std::sync::Arc;
    struct Captured {
        replay: ReplayHost,
        requests: Mutex<Vec<Request>>,
    }
    impl Host for Captured {
        fn fetch(&self, request: &Request) -> Result<Value> {
            self.requests.lock().push(request.clone());
            self.replay.fetch(request)
        }
    }
    let recording: Value = serde_json::from_str(RECORDING)?;
    let host = Arc::new(Captured {
        replay: ReplayHost::from_recording(RECORDING)?,
        requests: Mutex::new(Vec::new()),
    });
    let mut fx = common::Fx::new()?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    let bbox = &recording["steps"][0]["target"]["bbox"];
    let x = bbox["x"].as_f64().unwrap() + bbox["w"].as_f64().unwrap() / 2.;
    let y = bbox["y"].as_f64().unwrap() + bbox["h"].as_f64().unwrap() / 2.;
    fx.click((x as f32, y as f32))?;
    fx.wait("host automation list", |fx| {
        fx.read(|root, _| root.automations().is_some())
    })?;
    assert_eq!(fx.route(), app::Route::Scheduled);
    assert_eq!(
        fx.read(|root, _| root.automations().unwrap().clone()),
        Ok(json!({"items": []}))
    );
    assert_eq!(
        *host.requests.lock(),
        vec![Request::post("list-automations", None)]
    );
    assert!(fx.read(|root, cx| root.toasts(cx).is_empty()));
    fx.shot()?
        .save(std::env::temp_dir().join("cgx-f4-host-port.png"))?;
    let home = tempfile::tempdir()?;
    let local = LocalHost::new(home.path());
    local.write_automation("project-check", &definition("project-check", 30))?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(Arc::new(local), cx));
    fx.wait("local automation list", |fx| {
        fx.read(|root, _| root.automations().is_some())
    })?;
    assert_eq!(
        fx.read(|root, _| root.automations().unwrap().as_ref().unwrap()["items"][0]["id"].clone()),
        "project-check"
    );
    Ok(())
}
