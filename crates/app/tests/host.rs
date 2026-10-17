use anyhow::Result;
use app::host::{CloudHost, Credentials, Host, LocalHost, ReplayHost, Request, json_body};
use parking_lot::Mutex;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::fs;
use std::io::Write as _;
use std::sync::Arc;

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

/// One HTTP/1.1 request the mock backend received.
struct Received {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl Received {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A loopback backend answering `answers` in order; never leaves the machine.
fn mock_backend(answers: Vec<(u16, &'static str, String)>) -> (String, Arc<Mutex<Vec<Received>>>) {
    use std::io::{BufRead as _, BufReader, Read as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/backend-api", listener.local_addr().unwrap());
    let received = Arc::new(Mutex::new(Vec::new()));
    let log = received.clone();
    std::thread::spawn(move || {
        for (status, extra_headers, body) in answers {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap().to_owned();
            let path = parts.next().unwrap().to_owned();
            let mut headers = Vec::new();
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                let Some((k, v)) = line.trim_end().split_once(':') else {
                    break;
                };
                headers.push((k.to_owned(), v.trim().to_owned()));
            }
            let length: usize = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .map_or(0, |(_, v)| v.parse().unwrap());
            let mut request_body = vec![0; length];
            reader.read_exact(&mut request_body).unwrap();
            log.lock().push(Received {
                method,
                path,
                headers,
            });
            let mut stream = reader.into_inner();
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    (base, received)
}

/// A ChatGPT-style token whose claims name `account` and `user`; the signature is fake.
fn token(account: &str, user: &str, suffix: &str) -> String {
    use base64::Engine as _;
    let claims = json!({"https://api.openai.com/auth": {
        "chatgpt_account_id": account, "chatgpt_user_id": user
    }});
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
    format!("eyJhbGciOiJSUzI1NiJ9.{payload}.signature-{suffix}")
}

fn recorded_http(url_fragment: &str) -> Value {
    let recording: Value = serde_json::from_str(RECORDING).unwrap();
    recording["exchanges"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| {
            e["request"]["transport"] == "http"
                && e["request"]["request"]["url"]
                    .as_str()
                    .is_some_and(|u| u.contains(url_fragment))
        })
        .unwrap()["request"]["request"]
        .clone()
}

#[test]
fn cloud_requests_replay_the_recorded_endpoints_with_the_original_auth_headers() -> Result<()> {
    let replay = ReplayHost::from_recording(RECORDING)?;
    let endpoints = [
        "/wham/usage",
        "/beacons/home?",
        "/automations?filter=scheduled",
        "/suggested_automations",
    ];
    let requests: Vec<Value> = endpoints.iter().map(|e| recorded_http(e)).collect();
    let answers = requests
        .iter()
        .map(|request| {
            let body = replay.fetch(&Request::Http(request.clone())).unwrap();
            let body = body["value"]["response"]["__response"]["body"]
                .as_str()
                .unwrap()
                .to_owned();
            (200, "", body)
        })
        .collect();
    let (base, received) = mock_backend(answers);
    let account = "00000000-0000-4000-8000-000000000000";
    let token = token(account, "user-000000000000000000000000", "live");
    let source_token = token.clone();
    let cloud = CloudHost::new(
        LocalHost::new(tempfile::tempdir()?.path()),
        Box::new(move |_| {
            Ok(Some(Credentials {
                token: source_token.clone(),
                backend_origin: None,
                routing_override: Some("us".into()),
            }))
        }),
        base,
    );
    for request in &requests {
        let request = Request::Http(request.clone());
        assert_eq!(
            json_body(&cloud.fetch(&request)?)?,
            json_body(&replay.fetch(&request)?)?
        );
    }
    let received = received.lock();
    assert_eq!(received.len(), endpoints.len());
    for (received, endpoint) in received.iter().zip(endpoints) {
        assert_eq!(received.method, "GET");
        assert!(
            received
                .path
                .starts_with(&format!("/backend-api{endpoint}"))
        );
        assert_eq!(
            received.header("Authorization"),
            Some(format!("Bearer {token}").as_str())
        );
        assert_eq!(received.header("ChatGPT-Account-Id"), Some(account));
        assert_eq!(
            received.header("X-OpenAI-Account-Routing-Override"),
            Some("us")
        );
        assert_eq!(received.header("OAI-Language"), Some("en-US"));
        assert!(
            received
                .headers
                .iter()
                .all(|(k, _)| !k.to_ascii_lowercase().starts_with("x-openai-attach-"))
        );
    }
    assert_eq!(received[0].header("originator"), Some("Codex Desktop"));
    assert_eq!(received[0].header("OAI-App-Brand"), Some("chatgpt"));
    assert_eq!(
        received[1].header("User-Agent"),
        Some("Codex Desktop/26.1007.30912 (Mac OS; arm64)")
    );
    assert_eq!(
        received[3].header("X-OpenAI-Task-Suggestions-Surface"),
        Some("scheduled_page")
    );
    Ok(())
}

const CONFLICT: &str = r#"{"detail":"The Codex session changed. Reopen Code Review"}"#;

#[test]
fn cloud_errors_retry_once_after_401_and_never_expose_the_token() -> Result<()> {
    let (base, received) = mock_backend(vec![
        (401, "", r#"{"detail":"expired"}"#.to_owned()),
        (
            200,
            "X-OAI-IS-Update: ois1.a.b.c\r\n",
            r#"{"items":[],"cursor":null}"#.to_owned(),
        ),
        (409, "", CONFLICT.to_owned()),
        (409, "", CONFLICT.to_owned()),
        (200, "", "{}".to_owned()),
        (200, "", r#"{"conduit_token":"c"}"#.to_owned()),
    ]);
    let stale = token("acct", "user", "stale");
    let fresh = token("acct", "user", "fresh");
    let refreshes = Arc::new(Mutex::new(Vec::new()));
    let (log, s, f) = (refreshes.clone(), stale.clone(), fresh.clone());
    let cloud = CloudHost::new(
        LocalHost::new(tempfile::tempdir()?.path()),
        Box::new(move |refresh| {
            log.lock().push(refresh);
            Ok(Some(Credentials {
                token: if refresh { f.clone() } else { s.clone() },
                backend_origin: None,
                routing_override: None,
            }))
        }),
        base.clone(),
    );
    let automations = Request::Http(recorded_http("/automations?filter=scheduled"));
    let body = json_body(&cloud.fetch(&automations)?)?;
    assert_eq!(body, json!({"items": [], "cursor": null}));
    assert_eq!(*refreshes.lock(), vec![false, true]);
    {
        let received = received.lock();
        assert_eq!(
            received[0].header("Authorization"),
            Some(format!("Bearer {stale}").as_str())
        );
        assert_eq!(
            received[1].header("Authorization"),
            Some(format!("Bearer {fresh}").as_str())
        );
        assert_eq!(received[0].header("X-OAI-IS"), None);
    }
    // A 409 with `returnErrorResponse` is delivered; without it, it becomes the error object.
    let mut conflict = recorded_http("/wham/github/connections");
    let error = json_body(&cloud.fetch(&Request::Http(conflict.clone()))?).unwrap_err();
    assert!(error.to_string().contains("409"), "{error}");
    conflict["returnErrorResponse"] = json!(false);
    let response = cloud.fetch(&Request::Http(conflict))?;
    assert_eq!(response["value"]["responseType"], "error");
    assert_eq!(response["value"]["responseStatus"], 409);
    assert_eq!(response["value"]["error"], CONFLICT);
    // The accepted integrity update rides along on the next integrity-state request.
    cloud.fetch(&automations)?;
    assert_eq!(
        received.lock().last().unwrap().header("X-OAI-IS"),
        Some("ois1.a.b.c")
    );
    // The suggestion card's guided conversation start: the webview client's POST with a JSON
    // body and the identity it expects, like `/f/conversation/prepare` in the original.
    let prepare = json!({
        "url": "/f/conversation/prepare", "method": "POST",
        "body": r#"{"client_prepare_source":"suggested_automation"}"#,
        "headers": {"x-openai-web-frontend": "codex_webview", "OAI-Language": "en-US",
            "X-OpenAI-Attach-Auth": "1", "X-OpenAI-Attach-Desktop-Surface": "1",
            "X-OpenAI-Attach-Integrity-State": "1", "ChatGPT-Account-ID": "acct"},
        "expectedIdentity": {"accountId": "acct", "userId": "user"},
        "returnErrorResponse": true
    });
    assert_eq!(
        json_body(&cloud.fetch(&Request::Http(prepare))?)?,
        json!({"conduit_token": "c"})
    );
    {
        let received = received.lock();
        let prepare = received.last().unwrap();
        assert_eq!(prepare.method, "POST");
        assert_eq!(prepare.path, "/backend-api/f/conversation/prepare");
        assert_eq!(prepare.header("Content-Type"), Some("application/json"));
        assert_eq!(prepare.header("Content-Length"), Some("48"));
        assert_eq!(prepare.header("originator"), Some("Codex Desktop"));
        assert_eq!(prepare.header("X-OAI-IS"), Some("ois1.a.b.c"));
        assert_eq!(
            prepare.header("Authorization"),
            Some(format!("Bearer {fresh}").as_str())
        );
    }
    // Identity guards and refusals answer before any request leaves.
    let count = received.lock().len();
    let mut moved = recorded_http("/wham/usage");
    moved["expectedIdentity"] = json!({"accountId": "other", "userId": "user"});
    let response = cloud.fetch(&Request::Http(moved))?;
    assert_eq!(response["value"]["status"], 499);
    assert_eq!(
        response["value"]["error"],
        "Authenticated principal changed"
    );
    let elsewhere = json!({"url": "https://example.com/x", "method": "GET",
        "headers": {"X-OpenAI-Attach-Auth": "1"}});
    let response = cloud.fetch(&Request::Http(elsewhere))?;
    assert_eq!(response["value"]["status"], 400);
    assert_eq!(received.lock().len(), count);
    // Nothing observable carries the token: refusals, transport failures, credential
    // failures. `CloudHost` and `Credentials` have no `Debug`, so logging cannot print it.
    let mut observed = vec![response.to_string()];
    let closed = CloudHost::new(
        LocalHost::new(tempfile::tempdir()?.path()),
        Box::new(move |_| {
            Ok(Some(Credentials {
                token: fresh.clone(),
                backend_origin: None,
                routing_override: None,
            }))
        }),
        "http://127.0.0.1:1/backend-api",
    );
    let response = closed.fetch(&Request::Http(recorded_http("/wham/usage")))?;
    assert_eq!(response["value"]["responseType"], "error");
    assert_eq!(response["value"]["status"], 500);
    observed.push(response.to_string());
    let failing = CloudHost::new(
        LocalHost::new(tempfile::tempdir()?.path()),
        Box::new(|_| anyhow::bail!("app-server said: {}", token("a", "b", "leak"))),
        base,
    );
    let response = failing.fetch(&Request::Http(recorded_http("/wham/usage")))?;
    assert_eq!(response["value"]["status"], 432);
    assert_eq!(
        response["value"]["error"],
        "Failed to retrieve authentication token"
    );
    observed.push(response.to_string());
    for text in observed {
        assert!(!text.contains("signature-"), "token leaked: {text}");
        assert!(
            !text.contains("eyJhbGciOiJSUzI1NiJ9"),
            "token leaked: {text}"
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

#[cfg(target_os = "macos")]
#[test]
fn scheduled_navigation_loads_the_original_host_response() -> Result<()> {
    let recording: Value = serde_json::from_str(RECORDING)?;
    let host = Arc::new(ReplayHost::from_recording(RECORDING)?);
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

#[cfg(target_os = "macos")]
#[test]
fn scheduled_suggestions_follow_late_reads_and_host_replacement() -> Result<()> {
    let mut fx = common::Fx::new()?;
    let replay = Arc::new(ReplayHost::from_recording(RECORDING)?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(replay.clone(), cx));
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(ui::OpenScheduled), cx);
    })?;
    let fallback = fx.cx.capture_screenshot(fx.window)?;
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(ui::GoBack), cx);
    })?;
    fx.cx.run_until_parked();
    assert_eq!(fx.route(), app::Route::Home);
    fx.action(Box::new(ui::OpenScheduled))?;
    fx.parity("automations")?;

    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(replay, cx));
    let home = tempfile::tempdir()?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(Arc::new(LocalHost::new(home.path())), cx)
    });
    fx.cx.run_until_parked();
    let replaced = fx.shot()?;
    assert_eq!(
        common::changed(&fallback, &replaced, common::MAIN_PANEL),
        0,
        "the old host's suggestions must not reach the replacement page"
    );
    assert!(fx.read(|root, cx| root.toasts(cx).is_empty()));
    Ok(())
}

#[cfg(target_os = "macos")]
fn recorded_suggestions() -> Result<Value> {
    json_body(&ReplayHost::from_recording(RECORDING)?.fetch(&app::host::suggested_automations())?)
}

#[cfg(target_os = "macos")]
#[test]
fn late_suggestions_keep_starting_on_the_clicked_task() -> Result<()> {
    let mut fx = common::Fx::new()?;
    let host = Arc::new(ReplayHost::from_recording(RECORDING)?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(ui::OpenScheduled), cx);
    })?;
    let page = fx.read(|root, _| root.suggestions().unwrap().clone());
    page.update(&mut fx.cx, |page, cx| page.set_pending(Some(0), cx));
    let server = serde_json::from_value(recorded_suggestions()?)?;
    page.update(&mut fx.cx, |page, cx| page.set_suggestions(server, cx));
    let actual = fx.cx.capture_screenshot(fx.window)?;
    // The recorded response puts the clicked Weekly meal plan at index three.
    page.update(&mut fx.cx, |page, cx| page.set_pending(Some(3), cx));
    let expected = fx.cx.capture_screenshot(fx.window)?;
    assert_eq!(
        common::changed(&actual, &expected, common::MAIN_PANEL),
        0,
        "Starting must stay on Weekly meal plan, not AP Lang practice"
    );
    page.update(&mut fx.cx, |page, cx| page.set_pending(None, cx));
    fx.cx.run_until_parked();
    fx.parity("automations")?;
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn unbundled_suggestion_icon_shows_its_emoji() -> Result<()> {
    let mut fx = common::Fx::new()?;
    let host = Arc::new(ReplayHost::from_recording(RECORDING)?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    fx.action(Box::new(ui::OpenScheduled))?;
    let page = fx.read(|root, _| root.suggestions().unwrap().clone());
    let mut server = recorded_suggestions()?;
    // Change only the view input; the host still replays the genuine response.
    server[0]["icon_url"] = "https://example.invalid/not-bundled.svg".into();
    let suggestions = serde_json::from_value(server.clone())?;
    page.update(&mut fx.cx, |page, cx| page.set_suggestions(suggestions, cx));
    let unbundled = fx.shot()?;
    server[0]["icon_url"] = Value::Null;
    let suggestions = serde_json::from_value(server)?;
    page.update(&mut fx.cx, |page, cx| page.set_suggestions(suggestions, cx));
    let emoji = fx.shot()?;
    assert_eq!(
        common::changed(&unbundled, &emoji, common::MAIN_PANEL),
        0,
        "an unbundled icon must show the same emoji as a missing icon URL"
    );
    Ok(())
}
