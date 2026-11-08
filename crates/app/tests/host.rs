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
fn global_state_reads_preserve_the_profile() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let path = home.path().join(".codex-global-state.json");
    let request = Request::post("get-global-state", Some(json!({"key": "project-order"})));
    assert_eq!(json_body(&host.fetch(&request)?)?, json!({"value": null}));
    assert!(!path.exists());

    let fixture: Value =
        serde_json::from_str(include_str!("../../../refs/interactions/f1d_projects.json"))?;
    let state = &fixture["project_snapshot"]["state"];
    let contents = serde_json::to_vec(state)?;
    fs::write(&path, &contents)?;
    assert_eq!(
        json_body(&host.fetch(&request)?)?,
        json!({"value": state["project-order"]})
    );
    let missing = Request::post("get-global-state", Some(json!({"key": "missing"})));
    assert_eq!(json_body(&host.fetch(&missing)?)?, json!({"value": null}));
    assert_eq!(fs::read(path)?, contents);
    Ok(())
}

#[test]
fn global_state_rejects_invalid_requests_and_profile_data() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    for params in [
        json!({}),
        json!({"key": 7}),
        json!({"key": "project-order", "hostId": "remote"}),
    ] {
        assert!(
            host.fetch(&Request::post("get-global-state", Some(params)))
                .is_err()
        );
    }
    let path = home.path().join(".codex-global-state.json");
    let request = Request::post("get-global-state", Some(json!({"key": "project-order"})));
    for contents in ["{", "[]", "null"] {
        fs::write(&path, contents)?;
        assert!(host.fetch(&request).is_err());
        assert_eq!(fs::read_to_string(&path)?, contents);
    }
    Ok(())
}

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

#[test]
fn ssh_discovery_refreshes_the_selected_home_without_writes() -> Result<()> {
    let home = tempfile::tempdir()?;
    let storage = tempfile::tempdir()?;
    let ssh = home.path().join(".ssh");
    fs::create_dir(&ssh)?;
    let config = ssh.join("config");
    fs::write(
        &config,
        "Host capture-fixture\n  HostName 127.0.0.1\n  Port 1\n",
    )?;
    let host = LocalHost::new(storage.path()).with_ssh_home(home.path());
    let request = Request::post("discover-remote-ssh-connections", None);
    assert_eq!(
        json_body(&host.fetch(&request)?)?,
        json!({"sshAliases": ["capture-fixture"]})
    );

    let refreshed = b"Host updated-fixture\n  HostName 127.0.0.1\n  Port 1\n";
    fs::write(&config, refreshed)?;
    assert_eq!(
        json_body(&host.fetch(&request)?)?,
        json!({"sshAliases": ["updated-fixture"]})
    );
    assert_eq!(fs::read(&config)?, refreshed);
    assert!(!storage.path().join(".codex-global-state.json").exists());

    fs::write(&config, [0xff])?;
    assert!(host.fetch(&request).is_err());
    fs::remove_file(&config)?;
    assert_eq!(
        json_body(&host.fetch(&request)?)?,
        json!({"sshAliases": []})
    );
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
fn run_bulk_actions_preserve_other_tasks_running_archived_and_owned_rows() -> Result<()> {
    let home = tempfile::tempdir()?;
    let db = database(home.path())?;
    db.execute(
        "INSERT INTO automations (id, name) VALUES ('other', 'Other task')",
        [],
    )?;
    db.execute("INSERT INTO automation_runs (thread_id, automation_id, status, created_at) VALUES ('other-run', 'other', 'PENDING_REVIEW', 6)", [])?;
    let host = LocalHost::new(home.path());
    let scoped = json_body(&host.fetch(&Request::post(
        "inbox-items",
        Some(json!({"automationId":"local"})),
    ))?)?;
    assert_eq!(scoped["items"].as_array().unwrap().len(), 4);
    let result = json_body(&host.fetch(&Request::post(
        "inbox-automation-runs-mark-all-read",
        Some(json!({"automationId":"local"})),
    ))?)?;
    assert_eq!(result["changed"], 2);
    for id in ["running", "archived", "private-run", "other-run"] {
        assert_eq!(
            db.query_row(
                "SELECT read_at FROM automation_runs WHERE thread_id = ?",
                [id],
                |row| row.get::<_, Option<i64>>(0)
            )?,
            None,
            "{id}"
        );
    }
    for (automation, thread) in [
        ("local", "running"),
        ("local", "archived"),
        ("local", "other-run"),
        ("private", "private-run"),
    ] {
        assert!(
            host.fetch(&Request::post(
                "automation-run-archive",
                Some(json!({"automationId":automation,"threadId":thread}))
            ))
            .is_err(),
            "{thread}"
        );
    }
    host.fetch(&Request::post(
        "automation-run-archive",
        Some(json!({"automationId":"local","threadId":"review"})),
    ))?;
    assert_eq!(
        db.query_row(
            "SELECT status FROM automation_runs WHERE thread_id='review'",
            [],
            |row| row.get::<_, String>(0)
        )?,
        "ARCHIVED"
    );
    let scoped = json_body(&host.fetch(&Request::post(
        "inbox-items",
        Some(json!({"automationId":"private"})),
    ))?)?;
    assert!(scoped["items"].as_array().unwrap().is_empty());
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
    body: Vec<u8>,
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
                body: request_body,
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

#[test]
fn binary_upload_keeps_non_utf8_bytes_and_content_type() -> Result<()> {
    use base64::Engine as _;
    let (base, received) = mock_backend(vec![(200, "", "{}".into())]);
    let home = tempfile::tempdir()?;
    let cloud = CloudHost::new(LocalHost::new(home.path()), Box::new(|_| Ok(None)), &base);
    let bytes = [0, 255, 13, 10, 128];
    let response = cloud.fetch(&Request::Http(json!({
        "url":format!("{}/upload", base.trim_end_matches("/backend-api")), "method":"PUT",
        "headers":{"Content-Type":"application/octet-stream"},
        "bodyBase64":base64::engine::general_purpose::STANDARD.encode(bytes),
        "returnErrorResponse":true
    })))?;
    assert_eq!(json_body(&response)?, json!({}));
    let requests = received.lock();
    assert_eq!(requests[0].body, bytes);
    assert_eq!(
        requests[0].header("content-type"),
        Some("application/octet-stream")
    );
    Ok(())
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
        "/wham/remote/control/mfa_requirement",
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

#[path = "../src/host/automation_schedule.rs"]
mod automation_schedule;

fn update_database(home: &std::path::Path) -> Result<Connection> {
    let db = database(home)?;
    db.execute_batch(
        "ALTER TABLE automations ADD COLUMN cwds TEXT;
         ALTER TABLE automations ADD COLUMN target_type TEXT;
         ALTER TABLE automations ADD COLUMN project_id TEXT;
         ALTER TABLE automations ADD COLUMN model TEXT;
         ALTER TABLE automations ADD COLUMN reasoning_effort TEXT;
         ALTER TABLE automations ADD COLUMN created_at INTEGER;
         ALTER TABLE automations ADD COLUMN updated_at INTEGER;
         ALTER TABLE automations ADD COLUMN kind TEXT;
         ALTER TABLE automations ADD COLUMN target_thread_id TEXT;
         ALTER TABLE automations ADD COLUMN execution_environment TEXT;
         ALTER TABLE automations ADD COLUMN local_environment_config_path TEXT;
         ALTER TABLE automations ADD COLUMN plugin_template_id TEXT;
         ALTER TABLE automations ADD COLUMN notification_policy TEXT;
         ALTER TABLE automations ADD COLUMN auto_archive INTEGER;
         ALTER TABLE automations ADD COLUMN legacy_automation_id TEXT;",
    )?;
    Ok(db)
}

fn update_status(host: &LocalHost, status: &str) -> Result<Value> {
    let list = json_body(&host.fetch(&Request::post("list-automations", None))?)?;
    let item = &list["items"][0];
    let mut body = json!({"id": "task", "status": status});
    for key in [
        "kind",
        "name",
        "prompt",
        "model",
        "reasoningEffort",
        "notificationPolicy",
        "rrule",
    ] {
        body[key] = item[key].clone();
    }
    if item["kind"] == "heartbeat" {
        body["targetThreadId"] = item["targetThreadId"].clone();
    } else {
        for key in [
            "autoArchive",
            "executionEnvironment",
            "localEnvironmentConfigPath",
        ] {
            body[key] = item[key].clone();
        }
        body["projectId"] = item["target"]["projectId"].clone();
    }
    json_body(&host.fetch(&Request::post("automation-update", Some(body)))?)
}

fn unrelated_database_fields(
    db: &Connection,
) -> Result<std::collections::BTreeMap<String, rusqlite::types::Value>> {
    Ok(
        db.query_row("SELECT * FROM automations WHERE id='task'", [], |row| {
            row.as_ref()
                .column_names()
                .into_iter()
                .enumerate()
                .filter(|(_, name)| {
                    !["status", "next_run_at", "next_run_nominal_at", "updated_at"].contains(name)
                })
                .map(|(index, name)| Ok((name.to_owned(), row.get(index)?)))
                .collect()
        })?,
    )
}

#[test]
fn automation_status_persists_without_changing_definition_or_identity() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let definition = definition("task", 20)
        .replace("FREQ=DAILY;BYHOUR=9", "FREQ=MINUTELY;INTERVAL=30")
        + "custom_field = \"retained\"\n";
    host.write_automation("task", &definition)?;
    let db = update_database(home.path())?;
    fs::write(
        home.path().join("automations/.run-jitter-salt"),
        "fixed-salt\n",
    )?;
    let before = chrono::Utc::now().timestamp_millis();
    let resumed = update_status(&host, "ACTIVE")?["item"].clone();
    let after = chrono::Utc::now().timestamp_millis();
    let next = resumed["nextRunAt"].as_i64().unwrap();
    assert!(next > before && next <= after + 30 * 60_000);
    assert_eq!(resumed["nextRunNominalAt"], next);
    db.execute("UPDATE automations SET legacy_automation_id='retained-id', last_run_at=123 WHERE id='task'", [])?;
    let unrelated = unrelated_database_fields(&db)?;
    let fresh = LocalHost::new(home.path());
    let readback = json_body(&fresh.fetch(&Request::post("list-automations", None))?)?;
    assert_eq!(readback["items"][0]["nextRunAt"], next);
    assert_eq!(readback["items"][0]["status"], "ACTIVE");
    let paused = update_status(&fresh, "PAUSED")?["item"].clone();
    assert_eq!(paused["nextRunAt"], Value::Null);
    assert_eq!(paused["nextRunNominalAt"], Value::Null);
    assert_eq!(paused["lastRunAt"], 123);
    let row: (String, Option<i64>, Option<i64>, String) = db.query_row(
        "SELECT status, next_run_at, next_run_nominal_at, legacy_automation_id FROM automations WHERE id='task'",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(row, ("PAUSED".into(), None, None, "retained-id".into()));
    assert_eq!(unrelated_database_fields(&db)?, unrelated);
    let mut expected: toml::Table = toml::from_str(&definition)?;
    let actual: toml::Table = toml::from_str(&fs::read_to_string(
        home.path().join("automations/task/automation.toml"),
    )?)?;
    expected.insert("updated_at".into(), actual["updated_at"].clone());
    assert_eq!(actual, expected);
    let readback =
        json_body(&LocalHost::new(home.path()).fetch(&Request::post("list-automations", None))?)?;
    assert_eq!(readback["items"][0], paused);
    assert_eq!(
        fs::read_to_string(home.path().join("automations/.run-jitter-salt"))?,
        "fixed-salt\n"
    );
    update_status(&LocalHost::new(home.path()), "ACTIVE")?;
    assert_eq!(unrelated_database_fields(&db)?, unrelated);
    Ok(())
}

#[test]
fn automation_update_restores_definition_when_database_commit_fails() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    host.write_automation("task", &definition("task", 20))?;
    let db = update_database(home.path())?;
    update_status(&host, "ACTIVE")?;
    let path = home.path().join("automations/task/automation.toml");
    let original = fs::read(&path)?;
    let before = json_body(&host.fetch(&Request::post("list-automations", None))?)?;
    db.execute_batch("BEGIN; SELECT * FROM automations;")?;
    let error = update_status(&host, "PAUSED").unwrap_err();
    assert!(
        format!("{error:#}").contains("database is locked"),
        "{error:#}"
    );
    db.execute_batch("ROLLBACK")?;
    assert_eq!(fs::read(&path)?, original);
    assert_eq!(
        json_body(&LocalHost::new(home.path()).fetch(&Request::post("list-automations", None))?)?,
        before
    );
    Ok(())
}

#[test]
fn automation_update_rejects_invalid_schedule_and_unowned_field_changes() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let definition = definition("task", 20).replace("FREQ=DAILY;BYHOUR=9", "FREQ=DAILY;COUNT=1");
    host.write_automation("task", &definition)?;
    let db = update_database(home.path())?;
    db.execute("INSERT INTO automations(id,status,rrule,next_run_at) VALUES ('task','ACTIVE','FREQ=DAILY;COUNT=1',9999999999999)", [])?;
    assert!(automation_schedule::valid(
        "FREQ=DAILY;COUNT=1",
        chrono::Utc::now().timestamp_millis()
    ));
    assert!(!automation_schedule::valid(
        "FREQ=WEEKLY;BYDAY=XX",
        chrono::Utc::now().timestamp_millis()
    ));
    assert_eq!(
        update_status(&host, "ACTIVE").unwrap_err().to_string(),
        automation_schedule::NO_FUTURE_RUN
    );
    let changed = host.fetch(&Request::post(
        "automation-update",
        Some(json!({
            "id": "task", "status": "PAUSED", "createdAt": 999
        })),
    ));
    assert!(changed.is_err());
    assert_eq!(
        fs::read_to_string(home.path().join("automations/task/automation.toml"))?,
        definition
    );
    assert_eq!(
        db.query_row(
            "SELECT next_run_at FROM automations WHERE id='task'",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        9999999999999
    );
    Ok(())
}

#[test]
fn automation_execution_settings_survive_reload_and_kind_changes() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    host.write_automation(
        "task",
        &(definition("task", 20) + "custom_field = \"retained\"\n"),
    )?;
    let db = update_database(home.path())?;
    let project = home.path().join("project");
    fs::create_dir(&project)?;
    let updated = json_body(&host.fetch(&Request::post(
        "automation-update",
        Some(json!({
            "id": "task", "status": "PAUSED", "kind": "cron",
            "model": "gpt-6-astra", "reasoningEffort": "high",
            "notificationPolicy": "failed_runs_only", "autoArchive": true,
            "executionEnvironment": "local", "projectId": "project-one", "cwds": [project],
        })),
    ))?)?;
    let item = &updated["item"];
    assert_eq!(item["model"], "gpt-6-astra");
    assert_eq!(item["reasoningEffort"], "high");
    assert_eq!(
        item["target"],
        json!({"type":"project","projectId":"project-one"})
    );
    assert_eq!(item["autoArchive"], true);
    assert_eq!(item["notificationPolicy"], "failed_runs_only");
    assert_eq!(
        db.query_row("SELECT model,reasoning_effort,project_id,execution_environment,auto_archive FROM automations WHERE id='task'", [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, bool>(4)?)))?,
        ("gpt-6-astra".into(), "high".into(), "project-one".into(), "local".into(), true)
    );
    let heartbeat = json_body(&host.fetch(&Request::post(
        "automation-update",
        Some(json!({
            "id":"task", "status":"PAUSED", "kind":"heartbeat", "targetThreadId":"existing-thread"
        })),
    ))?)?;
    assert_eq!(heartbeat["item"]["kind"], "heartbeat");
    assert_eq!(heartbeat["item"]["targetThreadId"], "existing-thread");
    assert!(heartbeat["item"]["model"].is_null());
    assert_eq!(
        db.query_row(
            "SELECT cwds,model,target_type,target_thread_id FROM automations WHERE id='task'",
            [],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?
            ))
        )?,
        ("[]".into(), None, None, "existing-thread".into())
    );
    let path = home.path().join("automations/task/automation.toml");
    let previous = fs::read(&path)?;
    assert!(
        host.fetch(&Request::post(
            "automation-update",
            Some(json!({"id":"task", "status":"PAUSED", "kind":"cron"})),
        ))
        .is_err()
    );
    assert_eq!(fs::read(&path)?, previous);
    host.fetch(&Request::post(
        "automation-update",
        Some(json!({
            "id":"task", "status":"PAUSED", "kind":"cron", "cwds":[project],
            "projectId":null, "executionEnvironment":"worktree", "autoArchive":false
        })),
    ))?;
    let reloaded =
        json_body(&LocalHost::new(home.path()).fetch(&Request::post("list-automations", None))?)?;
    assert_eq!(reloaded["items"][0]["kind"], "cron");
    assert_eq!(
        reloaded["items"][0]["target"],
        json!({"type":"projectless"})
    );
    assert!(reloaded["items"][0]["targetThreadId"].is_null());
    let raw: toml::Table = toml::from_str(&fs::read_to_string(
        home.path().join("automations/task/automation.toml"),
    )?)?;
    assert_eq!(raw["custom_field"].as_str(), Some("retained"));
    assert_eq!(raw["created_at"].as_integer(), Some(10));
    Ok(())
}

#[test]
fn invalid_execution_settings_leave_the_definition_unchanged() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    host.write_automation("task", &definition("task", 20))?;
    let path = home.path().join("automations/task/automation.toml");
    let original = fs::read(&path)?;
    for (field, value) in [
        ("reasoningEffort", json!("invalid")),
        ("executionEnvironment", json!("cloud")),
        ("notificationPolicy", json!("unknown")),
        ("autoArchive", json!("yes")),
        ("cwds", json!(["relative"])),
        ("kind", json!("unknown")),
        ("name", json!("x".repeat(251))),
    ] {
        let mut params = json!({"id":"task", "status":"PAUSED"});
        params[field] = value;
        assert!(
            host.fetch(&Request::post("automation-update", Some(params)))
                .is_err(),
            "{field}"
        );
        assert_eq!(fs::read(&path)?, original, "{field}");
    }
    Ok(())
}

#[test]
fn automation_recurrence_matches_original_timezone_and_jitter_boundaries() {
    let zone = rrule::Tz::America__New_York;
    for (rule, now, actual, nominal) in [
        (
            "FREQ=DAILY;BYHOUR=2;BYMINUTE=30",
            "2026-03-08T06:00:00Z",
            1772955202000,
            1772955000000,
        ),
        (
            "FREQ=DAILY;BYHOUR=2;BYMINUTE=30",
            "2026-03-08T08:00:00Z",
            1773041340000,
            1773041400000,
        ),
        (
            "FREQ=DAILY;BYHOUR=1;BYMINUTE=30",
            "2026-11-01T05:45:00Z",
            1793600832000,
            1793601000000,
        ),
        (
            "FREQ=HOURLY;INTERVAL=3;BYHOUR=8,14;BYMINUTE=15",
            "2026-10-18T10:35:00Z",
            1792325490000,
            1792325700000,
        ),
        (
            "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1;BYHOUR=9;BYMINUTE=0",
            "2026-10-18T08:00:00Z",
            1793365200000,
            1793365200000,
        ),
        (
            "DTSTART:20261017T090000Z\nRRULE:FREQ=DAILY;COUNT=3",
            "2026-10-18T10:00:00Z",
            1792400396000,
            1792400400000,
        ),
        (
            "DTSTART;TZID=America/New_York:20260307T023000\nRRULE:FREQ=DAILY;COUNT=3",
            "2026-03-08T06:00:00Z",
            1773023538000,
            1773023400000,
        ),
        (
            "DTSTART;TZID=America/New_York:20261031T013000\nRRULE:FREQ=DAILY;COUNT=3",
            "2026-11-01T05:45:00Z",
            1793583033000,
            1793583000000,
        ),
        (
            "DTSTART:20261017T090000Z\nRRULE:FREQ=DAILY;COUNT=5\nEXDATE:20261019T090000Z\nRDATE:20261018T120000Z",
            "2026-10-18T10:00:00Z",
            1792324800000,
            1792324800000,
        ),
        // Original second 60 moves into the next minute, also across midnight, BYSETPOS and a DST gap.
        (
            "FREQ=DAILY;BYHOUR=9;BYMINUTE=0;BYSECOND=60",
            "2026-10-18T08:00:00Z",
            1792328399000,
            1792328460000,
        ),
        (
            "FREQ=DAILY;BYHOUR=23;BYMINUTE=59;BYSECOND=60",
            "2026-10-18T08:00:00Z",
            1792382366000,
            1792382400000,
        ),
        (
            "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1;BYHOUR=9;BYMINUTE=0;BYSECOND=0,60",
            "2026-10-18T08:00:00Z",
            1793365260000,
            1793365260000,
        ),
        (
            "FREQ=DAILY;BYHOUR=1;BYMINUTE=59;BYSECOND=60",
            "2026-03-08T06:00:00Z",
            1772952961000,
            1772953200000,
        ),
        // Original safe-integer INTERVAL and COUNT beyond u16/u32, up to year 9999.
        (
            "FREQ=HOURLY;INTERVAL=65536",
            "2026-10-18T08:00:00Z",
            2028240035000,
            2028240000000,
        ),
        (
            "FREQ=SECONDLY;INTERVAL=4294967296",
            "2026-10-18T08:00:00Z",
            6087277696000,
            6087277696000,
        ),
        (
            "FREQ=MINUTELY;INTERVAL=4000000000",
            "2026-10-18T08:00:00Z",
            241792310201000,
            241792310400000,
        ),
        (
            "FREQ=YEARLY;INTERVAL=7973",
            "2026-10-18T08:00:00Z",
            253395849600000,
            253395849600000,
        ),
        (
            "DTSTART:20261017T090000Z\nRRULE:FREQ=DAILY;COUNT=4294967297",
            "2026-10-18T08:00:00Z",
            1792314174000,
            1792314000000,
        ),
    ] {
        let now = chrono::DateTime::parse_from_rfc3339(now)
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            automation_schedule::next_run(rule, "cron", "task", now, zone, "fixed-salt"),
            Some((actual, nominal)),
            "{rule}"
        );
    }
    let now = 1792312632345;
    assert_eq!(
        automation_schedule::next_run(
            "FREQ=MINUTELY;INTERVAL=30",
            "heartbeat",
            "task",
            now,
            zone,
            "fixed-salt"
        ),
        Some((now + 1_800_000, now + 1_800_000))
    );
    for rule in [
        "FREQ=DAILY;COUNT=1",
        "FREQ=MONTHLY;BYMONTH=2;BYMONTHDAY=31",
        "FREQ=MINUTELY;INTERVAL=60;BYMINUTE=5",
        "FREQ=HOURLY;INTERVAL=0",
        // After year 9999, beyond the safe-integer limit, an unreachable second, and qP's hourly step limit.
        "FREQ=YEARLY;INTERVAL=7974",
        "FREQ=SECONDLY;INTERVAL=9007199254740992",
        "FREQ=DAILY;COUNT=9007199254740992;BYHOUR=9;BYMINUTE=0",
        "FREQ=SECONDLY;BYSECOND=60",
        "FREQ=HOURLY;INTERVAL=2501999792984",
    ] {
        assert_eq!(
            automation_schedule::next_run(rule, "cron", "task", now, zone, "fixed-salt"),
            None,
            "{rule}"
        );
    }
}

#[test]
fn automation_resume_reuses_persisted_jitter_after_a_fresh_host() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let definition = definition("task", 20).replace(
        "FREQ=DAILY;BYHOUR=9",
        "DTSTART:20400101T090000Z\\nRRULE:FREQ=DAILY;COUNT=2",
    );
    host.write_automation("task", &definition)?;
    let _db = update_database(home.path())?;
    let first = update_status(&host, "ACTIVE")?;
    let salt = fs::read_to_string(home.path().join("automations/.run-jitter-salt"))?;
    update_status(&host, "PAUSED")?;
    let second = update_status(&LocalHost::new(home.path()), "ACTIVE")?;
    assert_eq!(first["item"]["nextRunAt"], second["item"]["nextRunAt"]);
    assert_eq!(
        first["item"]["nextRunNominalAt"],
        second["item"]["nextRunNominalAt"]
    );
    assert_eq!(
        fs::read_to_string(home.path().join("automations/.run-jitter-salt"))?,
        salt
    );
    Ok(())
}

#[test]
fn automation_heartbeat_status_retains_its_thread_and_original_fields() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let definition = definition("task", 20)
        .replace(
            "kind = \"cron\"",
            "kind = \"heartbeat\"\ntarget_thread_id = \"retained-thread\"",
        )
        .replace("FREQ=DAILY;BYHOUR=9", "FREQ=MINUTELY;INTERVAL=30");
    host.write_automation("task", &definition)?;
    let db = update_database(home.path())?;
    let active = update_status(&host, "ACTIVE")?["item"].clone();
    assert_eq!(
        active["nextRunNominalAt"].as_i64().unwrap(),
        active["updatedAt"].as_i64().unwrap() + 1_800_000
    );
    assert_eq!(active["targetThreadId"], "retained-thread");
    assert_eq!(active["nextRunAt"], active["nextRunNominalAt"]);
    let unrelated = unrelated_database_fields(&db)?;
    let paused = update_status(&LocalHost::new(home.path()), "PAUSED")?["item"].clone();
    assert_eq!(paused["targetThreadId"], "retained-thread");
    assert_eq!(paused["nextRunAt"], Value::Null);
    assert_eq!(unrelated_database_fields(&db)?, unrelated);
    let mut expected: toml::Table = toml::from_str(&definition)?;
    let actual: toml::Table = toml::from_str(&fs::read_to_string(
        home.path().join("automations/task/automation.toml"),
    )?)?;
    expected.insert("updated_at".into(), actual["updated_at"].clone());
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn automation_update_rejects_missing_invalid_and_account_owned_targets() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let request = |id: &str| {
        Request::post(
            "automation-update",
            Some(json!({"id": id, "status": "ACTIVE"})),
        )
    };
    assert_eq!(
        host.fetch(&request("missing")).unwrap_err().to_string(),
        "Automation does not exist in the app and could not be updated. It may have been deleted manually by the user."
    );
    assert_eq!(
        host.fetch(&request("../escape")).unwrap_err().to_string(),
        "invalid automation id"
    );
    assert!(!home.path().join("automations").exists());
    let definition = definition("task", 20);
    host.write_automation("task", &definition)?;
    let db = update_database(home.path())?;
    db.execute("INSERT INTO automations(id,account_id,status,rrule,next_run_at) VALUES ('task','other-account','ACTIVE','original-rule',123)", [])?;
    let before = unrelated_database_fields(&db)?;
    assert!(host.fetch(&request("task")).is_err());
    assert_eq!(unrelated_database_fields(&db)?, before);
    assert_eq!(
        db.query_row(
            "SELECT next_run_at FROM automations WHERE id='task'",
            [],
            |row| row.get::<_, i64>(0)
        )?,
        123
    );
    assert_eq!(
        fs::read_to_string(home.path().join("automations/task/automation.toml"))?,
        definition
    );
    assert!(!home.path().join("automations/.run-jitter-salt").exists());
    Ok(())
}

#[test]
fn automation_resume_schedules_when_the_jitter_salt_cannot_be_saved() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    host.write_automation("task", &definition("task", 20))?;
    let _db = update_database(home.path())?;
    let salt = home.path().join("automations/.run-jitter-salt");
    fs::create_dir(&salt)?;
    let resumed = update_status(&host, "ACTIVE")?["item"].clone();
    assert_eq!(resumed["status"], "ACTIVE");
    assert!(resumed["nextRunAt"].as_i64().unwrap() >= resumed["updatedAt"].as_i64().unwrap());
    assert!(salt.is_dir());
    Ok(())
}

/// Mobile `Get started` reads the MFA requirement, then the MFA status, through the host. A
/// host without the authenticated transport leaves the initial step with its error; the retry
/// shows the loading step, then the recorded `required` requirement and `mfa_enabled_v2: true`
/// status open the allow-host step, not the MFA step.
#[cfg(target_os = "macos")]
#[test]
fn mobile_get_started_follows_the_recorded_mfa_status() -> Result<()> {
    use ui::screens::mobile::{GetStarted, SetupStep};
    let mut fx = common::Fx::new()?;
    let home = tempfile::tempdir()?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(Arc::new(LocalHost::new(home.path())), cx)
    });
    fx.action(Box::new(ui::OpenMobileSetup))?;
    let page = fx.read(|root, _| root.mobile_page().unwrap().clone());
    let step = |fx: &common::Fx| fx.read(|_, cx| page.read(cx).step());
    assert_eq!(step(&fx), SetupStep::Initial);

    fx.click(fx.at("mobile", "Get started")?)?;
    assert_eq!(step(&fx), SetupStep::Error);

    let replay = Arc::new(ReplayHost::from_recording(include_str!(
        "../../../refs/interactions/mobile-mfa.json"
    ))?);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(replay, cx));
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(GetStarted), cx);
    })?;
    assert_eq!(step(&fx), SetupStep::Pending);
    fx.cx.run_until_parked();
    assert_eq!(step(&fx), SetupStep::AllowHost);
    Ok(())
}

#[test]
fn edited_task_keeps_its_identity_and_unrelated_fields_across_all_recurrences() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let created = json_body(&host.fetch(&Request::post(
        "automation-create",
        Some(json!({
            "name": "Original", "prompt": "Original prompt",
            "rrule": "FREQ=MINUTELY;INTERVAL=30", "cwds": [home.path()]
        })),
    ))?)?["item"]
        .clone();
    let id = created["id"].as_str().unwrap();
    let path = home
        .path()
        .join("automations")
        .join(id)
        .join("automation.toml");
    let mut raw: toml::Table = toml::from_str(&fs::read_to_string(&path)?)?;
    raw.insert("custom_field".into(), "retained".into());
    raw.insert("model".into(), "stored-model".into());
    host.write_automation(id, &toml::to_string(&raw)?)?;
    let db = update_database(home.path())?;
    for (index, rule) in [
        "FREQ=MINUTELY;INTERVAL=30",
        "FREQ=HOURLY;INTERVAL=2;BYMINUTE=0",
        "FREQ=DAILY;BYHOUR=18;BYMINUTE=35",
        "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR;BYHOUR=9;BYMINUTE=15",
        "FREQ=WEEKLY;INTERVAL=2;BYDAY=TU,FR;BYHOUR=14;BYMINUTE=20",
        "FREQ=MONTHLY;BYMONTHDAY=1,15;BYHOUR=22;BYMINUTE=10",
        "FREQ=YEARLY;INTERVAL=2;BYMONTH=10;BYMONTHDAY=20;BYHOUR=9;BYMINUTE=0",
    ]
    .into_iter()
    .enumerate()
    {
        let name = format!("Edited {index}");
        let prompt = format!("Prompt {index}\nRetain the second line.");
        let saved = json_body(&host.fetch(&Request::post(
            "automation-update",
            Some(json!({
                "id": id, "status": "ACTIVE", "name": name, "prompt": prompt, "rrule": rule
            })),
        ))?)?["item"]
            .clone();
        assert_eq!(saved["id"], id);
        assert_eq!(saved["createdAt"], created["createdAt"]);
        assert_eq!(saved["name"], name);
        assert_eq!(saved["prompt"], prompt);
        assert_eq!(saved["rrule"], rule);
        assert_eq!(saved["model"], "stored-model");
        let raw: toml::Table = toml::from_str(&fs::read_to_string(&path)?)?;
        assert_eq!(raw["custom_field"].as_str(), Some("retained"));
        let (stored_name, stored_prompt, stored_rule, next): (String, String, String, i64) = db
            .query_row(
                "SELECT name,prompt,rrule,next_run_nominal_at FROM automations WHERE id=?",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        assert_eq!(
            (stored_name, stored_prompt, stored_rule),
            (name, prompt, rule.into())
        );
        assert!(next > saved["updatedAt"].as_i64().unwrap());
    }
    let before = fs::read(&path)?;
    for patch in [
        json!({"name": " "}),
        json!({"prompt": "\n"}),
        json!({"rrule": "FREQ=WEEKLY;BYDAY=XX"}),
    ] {
        let mut body = json!({"id": id, "status": "PAUSED"});
        body.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(
            host.fetch(&Request::post("automation-update", Some(body)))
                .is_err()
        );
        assert_eq!(fs::read(&path)?, before);
    }
    let loaded =
        json_body(&LocalHost::new(home.path()).fetch(&Request::post("list-automations", None))?)?;
    assert_eq!(
        loaded["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["id"] == id)
            .unwrap()["name"],
        "Edited 6"
    );
    Ok(())
}

#[test]
fn fresh_profile_task_edits_do_not_require_or_create_the_original_database() -> Result<()> {
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let created = json_body(&host.fetch(&Request::post("automation-create", Some(json!({
        "name": "Original", "prompt": "Original prompt", "rrule": "FREQ=DAILY;BYHOUR=9", "cwds": [home.path()]
    }))))?)?["item"].clone();
    host.fetch(&Request::post(
        "automation-update",
        Some(json!({
            "id": created["id"], "status": "ACTIVE", "name": "Renamed",
            "prompt": "A new prompt", "rrule": "FREQ=MONTHLY;BYMONTHDAY=20;BYHOUR=18;BYMINUTE=30"
        })),
    ))?;
    let reloaded =
        json_body(&LocalHost::new(home.path()).fetch(&Request::post("list-automations", None))?)?;
    assert_eq!(reloaded["items"][0]["name"], "Renamed");
    assert_eq!(reloaded["items"][0]["prompt"], "A new prompt");
    assert_eq!(
        reloaded["items"][0]["rrule"],
        "FREQ=MONTHLY;BYMONTHDAY=20;BYHOUR=18;BYMINUTE=30"
    );
    assert!(!home.path().join("sqlite/codex.db").exists());
    Ok(())
}

fn cloud_automation(id: &str) -> Value {
    json!({
        "id": id, "title": "Cloud task", "prompt": "Cloud prompt",
        "schedule": "BEGIN:VEVENT\nDTSTART;TZID=Europe/London:20261029T090000\nRRULE:FREQ=DAILY;BYHOUR=9\nEND:VEVENT",
        "default_timezone": "Europe/London", "is_enabled": true,
        "email_enabled": false, "notifications_enabled": true,
        "model": "gpt-5", "reasoning_effort": "high", "emoji": "clock",
        "executor": "chatgpt", "timing_mode": "flexible_schedule",
        "created_at": "2026-10-28T12:00:00Z",
        "next_run_times": ["2026-10-29T09:00:00Z"]
    })
}

#[test]
fn cloud_automation_edits_and_deletion_leave_the_same_local_id_untouched() -> Result<()> {
    let home = tempfile::tempdir()?;
    let local = LocalHost::new(home.path());
    local.write_automation("same-id", &definition("same-id", 20))?;
    let path = home.path().join("automations/same-id/automation.toml");
    let before = fs::read(&path)?;
    let original = cloud_automation("same-id");
    let mut completed = cloud_automation("finished");
    completed["is_enabled"] = json!(false);
    completed["next_run_times"] = json!([]);
    let mut edited = original.clone();
    edited["title"] = json!("Renamed");
    edited["prompt"] = json!("Edited prompt");
    let mut paused = edited.clone();
    paused["is_enabled"] = json!(false);
    let (base, received) = mock_backend(vec![
        (
            200,
            "",
            json!({"items":[{"automation":original}], "cursor":"next+page"}).to_string(),
        ),
        (
            200,
            "",
            json!({"items":[{"automation":completed}], "cursor":null}).to_string(),
        ),
        (200, "", json!({"automation":original}).to_string()),
        (200, "", edited.to_string()),
        (200, "", paused.to_string()),
        (200, "", "{}".into()),
        (200, "", "{}".into()),
    ]);
    let access = token("cloud-account", "cloud-user", "live");
    let cloud = CloudHost::new(
        local,
        Box::new(move |_| {
            Ok(Some(Credentials {
                token: access.clone(),
                backend_origin: None,
                routing_override: None,
            }))
        }),
        base,
    );
    let list = json_body(&cloud.fetch(&Request::post("cloud-list-automations", Some(json!({}))))?)?;
    assert_eq!(list["items"][0]["status"], "ACTIVE");
    assert_eq!(list["items"][1]["status"], "COMPLETED");
    assert_eq!(list["items"][0]["rrule"], "FREQ=DAILY;BYHOUR=9");
    assert_eq!(list["items"][0]["model"], "gpt-5");
    let mut item = json_body(&cloud.fetch(&Request::post(
        "cloud-read-automation",
        Some(json!({"id":"same-id"})),
    ))?)?["item"]
        .clone();
    assert_eq!(item["nextRunAt"], 1793264400000_i64);
    item["name"] = json!(" Renamed ");
    item["prompt"] = json!(" Edited prompt ");
    let saved =
        json_body(&cloud.fetch(&Request::post("cloud-automation-update", Some(item)))?)?["item"]
            .clone();
    assert_eq!(saved["name"], "Renamed");
    assert_eq!(saved["prompt"], "Edited prompt");
    let mut status = saved.clone();
    status["status"] = json!("PAUSED");
    let paused = json_body(
        &cloud.fetch(&Request::post("cloud-automation-set-status", Some(status)))?,
    )?["item"]
        .clone();
    assert_eq!(paused["status"], "PAUSED");
    let run = json_body(&cloud.fetch(&Request::post(
        "cloud-automation-run-now",
        Some(paused.clone()),
    ))?)?;
    assert_eq!(run["success"], true);
    let deleted = json_body(&cloud.fetch(&Request::post(
        "cloud-automation-delete",
        Some(paused.clone()),
    ))?)?;
    assert_eq!(deleted["status"], "deleted");
    let error = cloud
        .fetch(&Request::post("automation-delete", Some(paused)))
        .unwrap_err();
    assert!(error.to_string().contains("unavailable through LocalHost"));
    assert_eq!(fs::read(path)?, before);
    let local = json_body(&cloud.fetch(&Request::post("list-automations", None))?)?;
    assert_eq!(local["items"][0]["id"], "same-id");
    let requests = received.lock();
    assert_eq!(
        requests[1].path,
        "/backend-api/automations?limit=100&cursor=next%2Bpage"
    );
    let update: Value = serde_json::from_slice(&requests[3].body)?;
    assert_eq!(update["jawbone_id"], "same-id");
    assert_eq!(update["schedule"], original["schedule"]);
    assert_eq!(update["default_timezone"], "Europe/London");
    assert_eq!(update["notifications_enabled"], true);
    assert_eq!(update["model"], "gpt-5");
    assert_eq!(update["reasoning_effort"], "high");
    assert_eq!(update["title"], "Renamed");
    assert_eq!(update["prompt"], "Edited prompt");
    assert!(update.get("cwds").is_none());
    assert_eq!(requests[4].path, "/backend-api/automations/set_status");
    assert_eq!(requests[5].path, "/backend-api/automation/same-id/run");
    assert_eq!(requests[6].path, "/backend-api/automations/remove");
    Ok(())
}

#[test]
fn cloud_automation_mutation_rejects_an_account_change_and_quotes_the_id() -> Result<()> {
    let original = cloud_automation("one/two?x");
    let (base, received) =
        mock_backend(vec![(200, "", json!({"automation":original}).to_string())]);
    let access = Arc::new(Mutex::new(token("first-account", "first-user", "live")));
    let source = access.clone();
    let home = tempfile::tempdir()?;
    let cloud = CloudHost::new(
        LocalHost::new(home.path()),
        Box::new(move |_| {
            Ok(Some(Credentials {
                token: source.lock().clone(),
                backend_origin: None,
                routing_override: None,
            }))
        }),
        base,
    );
    let item = json_body(&cloud.fetch(&Request::post(
        "cloud-read-automation",
        Some(json!({"id":"one/two?x"})),
    ))?)?["item"]
        .clone();
    assert_eq!(
        received.lock()[0].path,
        "/backend-api/automation/one%2Ftwo%3Fx"
    );
    *access.lock() = token("second-account", "second-user", "live");
    cloud.invalidate_identity();
    let error = cloud
        .fetch(&Request::post("cloud-automation-delete", Some(item)))
        .unwrap_err();
    assert_eq!(error.to_string(), "Automation account changed");
    assert!(!home.path().join("automations").exists());
    Ok(())
}
