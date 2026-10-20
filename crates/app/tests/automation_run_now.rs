//! `automation-run-now` over a real stdio JSON-RPC peer: a shell script that logs each request
//! line it receives and answers it, so the test sees exactly what the host sent the app-server.
use anyhow::{Result, ensure};
use app::host::{Host, LocalHost, Request, json_body};
use bridge::Bridge;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, sync::Arc};

const SERVER: &str = r#"
[ -n "$CONFIG" ] || CONFIG='{}'
[ -n "$MODELS" ] || MODELS='[]'
[ -n "$REQUIREMENTS" ] || REQUIREMENTS='null'
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$LOG"
  case "$line" in
    *'"turn/start"'*) [ -z "$STATE_ON_TURN" ] || cp "$STATE" "$STATE_ON_TURN" ;;
  esac
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"model/list"'*) printf '{"id":%s,"result":{"data":%s}}\n' "$id" "$MODELS" ;;
    *'"config/read"'*) printf '{"id":%s,"result":{"config":%s}}\n' "$id" "$CONFIG" ;;
    *'"configRequirements/read"'*) [ -z "$FAIL_REQUIREMENTS" ] || { printf '{"id":%s,"error":{"code":-32000,"message":"requirements unavailable"}}\n' "$id"; continue; }; printf '{"id":%s,"result":{"requirements":%s}}\n' "$id" "$REQUIREMENTS" ;;
    *'"thread/read"'*)
      if [ -n "$FAIL_READ" ]; then printf '{"id":%s,"error":{"code":-32000,"message":"missing fixture thread"}}\n' "$id"; continue; fi
      if [ -n "$ACTIVE_RACE" ]; then printf '{"method":"item/started","params":{"threadId":"heartbeat-thread","turnId":"racing-turn"}}\n'; fi
      printf '{"id":%s,"result":{"thread":%s}}\n' "$id" "${THREAD:-null}" ;;
    *'"thread/resume"'*)
      if [ -n "$FAIL_RESUME" ]; then printf '{"id":%s,"error":{"code":-32000,"message":"resume refused"}}\n' "$id"; continue; fi
      printf '{"id":%s,"result":{"thread":%s}}\n' "$id" "${THREAD:-null}" ;;
    *'"thread/start"'*)
      [ -z "$FAIL_THREAD" ] || { printf '{"id":%s,"error":{"code":-32000,"message":"no thread"}}\n' "$id"; continue; }
      [ -z "$STATE_AFTER_THREAD" ] || printf '%s' "$STATE_AFTER_THREAD" > "$STATE"
      [ -z "$BLOCK_STATE" ] || { mv "$STATE" "$STATE.saved"; mkdir "$STATE"; }
      printf '{"id":%s,"result":{"thread":{"id":"%s"}}}\n' "$id" "${THREAD_ID:-thread-$id}" ;;
    *'"turn/start"'*REFUSE*) printf '{"id":%s,"error":{"code":-32000,"message":"model unavailable"}}\n' "$id" ;;
    *'"turn/start"'*automation_heartbeat_run_now*)
      [ -z "$SLOW" ] || sleep 2
      printf '{"method":"turn/started","params":{"threadId":"heartbeat-thread","turn":{"id":"heartbeat-turn","status":"inProgress","items":[]}}}\n'
      printf '{"id":%s,"result":{"turn":{"id":"heartbeat-turn"}}}\n' "$id" ;;
    *'"turn/start"'*)
      [ -z "$SLOW" ] || sleep 2
      if [ -n "$HOLD_TURN" ]; then
        thread=$(printf '%s' "$line" | sed -n 's/.*"threadId":"\([^"]*\)".*/\1/p')
        printf '{"method":"turn/completed","params":{"threadId":"%s","turn":{"id":"turn-%s","status":"completed"}}}\n' "$thread" "$id"
        while [ ! -e "$TURN_RELEASE" ]; do [ -d "${TURN_RELEASE%/*}" ] || exit; sleep 0.01; done
      fi
      printf '{"id":%s,"result":{"turn":{"id":"turn-%s"}}}\n' "$id" "$id" ;;
    *'"thread/archive"'*)
      if [ -n "$HOLD_ARCHIVE" ]; then
        thread=$(printf '%s' "$line" | sed -n 's/.*"threadId":"\([^"]*\)".*/\1/p')
        printf '{"method":"thread/archived","params":{"threadId":"%s"}}\n' "$thread"
        while [ ! -e "$ARCHIVE_RELEASE" ]; do [ -d "${ARCHIVE_RELEASE%/*}" ] || exit; sleep 0.01; done
      fi
      [ -z "$ARCHIVE_FAIL" ] || { printf '{"id":%s,"error":{"code":-32000,"message":"archive refused"}}\n' "$id"; continue; }
      printf '{"id":%s,"result":{}}\n' "$id" ;;
    *'"thread/unarchive"'*)
      [ ! -e "$RESTORE_FAIL_FILE" ] || { printf '{"id":%s,"error":{"code":-32000,"message":"restore refused"}}\n' "$id"; continue; }
      printf '{"id":%s,"result":{}}\n' "$id" ;;
    *) printf '{"id":%s,"result":{}}\n' "$id" ;;
  esac
done
"#;

fn host(home: &Path, log: &Path, slow: bool) -> Result<LocalHost> {
    host_with(home, log, slow, &[])
}

/// `env` sets the stub's `CONFIG`, `MODELS` and `REQUIREMENTS` answers (JSON).
fn host_with(home: &Path, log: &Path, slow: bool, env: &[(&str, &str)]) -> Result<LocalHost> {
    let mut server = Command::new("sh");
    server.envs(env.iter().copied());
    server
        .arg("-c")
        .arg(SERVER)
        .env("LOG", log)
        .env("STATE", home.join(".codex-global-state.json"))
        .env("SLOW", if slow { "1" } else { "" });
    let (bridge, _events) = Bridge::spawn(server)?;
    Ok(LocalHost::new(home)
        .with_host_settings_path(home.join("host-settings.json"))
        .with_bridge(Arc::new(bridge)))
}

fn create(host: &LocalHost, home: &Path, prompt: &str) -> Result<String> {
    let params = json!({"name": "Review", "prompt": prompt, "rrule": "FREQ=DAILY;BYHOUR=9;BYMINUTE=0",
        "cwds": [home]});
    let created = json_body(&host.fetch(&Request::post("create-automation", Some(params)))?)?;
    Ok(created["id"].as_str().unwrap_or_default().to_owned())
}

fn run(host: &LocalHost, id: &str) -> Result<Value> {
    json_body(&host.fetch(&Request::post(
        "automation-run-now",
        Some(json!({"id": id, "collaborationMode": null, "permissions": null})),
    ))?)
}

fn all_requests(log: &Path) -> Result<Vec<Value>> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

/// The thread and turn requests of a run, without the config and model reads before them.
fn requests(log: &Path) -> Result<Vec<Value>> {
    let mut sent = all_requests(log)?;
    sent.retain(|r| {
        matches!(
            r["method"].as_str(),
            Some("thread/start" | "thread/name/set" | "turn/start")
        )
    });
    Ok(sent)
}

fn completion(log: &Path, thread: &str, status: &str) -> Result<Value> {
    let request = all_requests(log)?
        .into_iter()
        .find(|request| {
            request["method"] == "turn/start" && request["params"]["threadId"] == thread
        })
        .ok_or_else(|| anyhow::anyhow!("missing initial turn"))?;
    Ok(json!({"id": format!("turn-{}", request["id"]), "status": status}))
}

#[test]
fn run_now_starts_a_titled_thread_and_turn_with_the_task_prompt() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let log = dir.path().join("requests.jsonl");
    let host = host(dir.path(), &log, false)?;
    let id = create(&host, dir.path(), "Review local changes")?;
    ensure!(run(&host, &id)? == json!({"success": true}));

    let sent = requests(&log)?;
    let methods: Vec<_> = sent.iter().map(|r| r["method"].as_str().unwrap()).collect();
    ensure!(methods == ["thread/start", "thread/name/set", "turn/start"]);
    ensure!(sent[0]["params"]["cwd"] == json!(dir.path()));
    let thread = sent[2]["params"]["threadId"].as_str().unwrap_or_default();
    ensure!(sent[1]["params"] == json!({"threadId": thread, "name": "Review"}));
    let turn = &sent[2]["params"];
    ensure!(turn["threadId"] == thread);
    ensure!(turn["effort"] == "medium");
    let text = turn["input"][0]["text"].as_str().unwrap_or_default();
    ensure!(text.starts_with(&format!("Automation: Review\n\nAutomation ID: {id}\n")));
    ensure!(text.ends_with("Last run: never\n\nReview local changes"));
    Ok(())
}

#[test]
fn run_now_reports_the_app_server_refusal_and_starts_no_second_run() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let log = dir.path().join("requests.jsonl");
    let host = host(dir.path(), &log, false)?;
    let id = create(&host, dir.path(), "REFUSE this")?;
    let error = run(&host, &id).unwrap_err().to_string();
    ensure!(error.contains("model unavailable"), "{error}");
    ensure!(requests(&log)?.len() == 3);
    Ok(())
}

#[test]
fn run_now_refuses_tasks_it_cannot_run_before_touching_the_app_server() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let log = dir.path().join("requests.jsonl");
    let host = host(dir.path(), &log, false)?;
    ensure!(
        run(&host, "missing")
            .unwrap_err()
            .to_string()
            .contains("Automation not found")
    );
    let event = "## When this happens\n> a PR opens\n\n## Perform action\n> review it";
    let id = create(&host, dir.path(), event)?;
    ensure!(
        run(&host, &id)
            .unwrap_err()
            .to_string()
            .contains("triggered by events")
    );
    let heartbeat = "version = 1\nid = \"hb\"\nname = \"H\"\nprompt = \"p\"\nstatus = \"ACTIVE\"\nkind = \"heartbeat\"\ntarget_thread_id = \"t\"\ncreated_at = 1\nupdated_at = 1\n";
    host.write_automation("hb", heartbeat)?;
    ensure!(
        run(&host, "hb")
            .unwrap_err()
            .to_string()
            .contains("Heartbeat")
    );
    ensure!(requests(&log)?.is_empty());
    Ok(())
}

const AUTOMATIONS: &str = "CREATE TABLE automations (id TEXT PRIMARY KEY, name TEXT, prompt TEXT,
    status TEXT, next_run_at INTEGER, next_run_nominal_at INTEGER, last_run_at INTEGER, cwds TEXT,
    target_type TEXT, project_id TEXT, rrule TEXT, model TEXT, reasoning_effort TEXT,
    created_at INTEGER, updated_at INTEGER, kind TEXT, target_thread_id TEXT,
    execution_environment TEXT, local_environment_config_path TEXT, plugin_template_id TEXT,
    notification_policy TEXT, auto_archive INTEGER, account_id TEXT, user_id TEXT,
    installation_id TEXT)";

const RUNS: &str =
    "CREATE TABLE automation_runs (thread_id TEXT PRIMARY KEY, automation_id TEXT NOT NULL,
    status TEXT NOT NULL, read_at INTEGER, thread_title TEXT, source_cwd TEXT, inbox_title TEXT,
    inbox_summary TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    archived_user_message TEXT, archived_assistant_message TEXT, archived_reason TEXT)";

/// `(thread_id, status, read_at, archived_reason, source_cwd)`.
type Run = (String, String, Option<i64>, Option<String>, Option<String>);

fn runs(home: &Path) -> Result<Vec<Run>> {
    let db = rusqlite::Connection::open(home.join("sqlite/codex.db"))?;
    let mut query = db.prepare(
        "SELECT thread_id, status, read_at, archived_reason, source_cwd FROM automation_runs ORDER BY thread_id",
    )?;
    let rows = query.query_map([], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

#[test]
fn run_now_uses_the_saved_config_and_the_listed_model() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let log = dir.path().join("requests.jsonl");
    let models = r#"[{"model":"gpt-6.1-sol","isDefault":true,"defaultReasoningEffort":"low","supportedReasoningEfforts":[{"reasoningEffort":"low"}]}]"#;
    let config = r#"{"sandbox_mode":"read-only","approval_policy":"untrusted"}"#;
    let requirements =
        r#"{"allowedSandboxModes":["read-only"],"allowedApprovalPolicies":["untrusted"]}"#;
    let host = host_with(
        dir.path(),
        &log,
        false,
        &[
            ("MODELS", models),
            ("CONFIG", config),
            ("REQUIREMENTS", requirements),
        ],
    )?;
    let id = create(&host, dir.path(), "Review local changes")?;
    ensure!(run(&host, &id)? == json!({"success": true}));
    let sent = all_requests(&log)?;
    let methods: Vec<_> = sent.iter().map(|r| r["method"].as_str().unwrap()).collect();
    ensure!(
        methods
            == [
                "model/list",
                "configRequirements/read",
                "config/read",
                "thread/start",
                "thread/name/set",
                "turn/start"
            ],
        "{methods:?}"
    );
    ensure!(sent[2]["params"] == json!({"includeLayers": false, "cwd": dir.path()}));
    let (start, turn) = (&sent[3]["params"], &sent[5]["params"]);
    // The task names no model, and the original's own default is listed: it wins, with its effort.
    ensure!(
        start["model"] == "gpt-6.1-sol"
            && turn["model"] == "gpt-6.1-sol"
            && turn["effort"] == "low"
    );
    ensure!(start["sandbox"] == "read-only" && start["approvalPolicy"] == "untrusted");
    ensure!(turn["sandboxPolicy"]["type"] == "readOnly" && turn["approvalPolicy"] == "untrusted");
    Ok(())
}

#[test]
fn run_now_opens_moves_finishes_and_archives_the_run_row() -> Result<()> {
    let dir = tempfile::tempdir()?;
    fs::create_dir_all(dir.path().join("sqlite"))?;
    let db = rusqlite::Connection::open(dir.path().join("sqlite/codex.db"))?;
    db.execute(RUNS, [])?;
    db.execute(AUTOMATIONS, [])?;
    drop(db);
    let log = dir.path().join("requests.jsonl");
    let host = host(dir.path(), &log, false)?;
    let id = create(&host, dir.path(), "Review local changes")?;
    ensure!(run(&host, &id)? == json!({"success": true}));
    let thread = requests(&log)?[2]["params"]["threadId"]
        .as_str()
        .unwrap()
        .to_owned();
    let thread = thread.as_str();
    let rows = runs(dir.path())?;
    ensure!(
        rows.len() == 1 && rows[0].0 == thread,
        "the pending row took the thread id: {rows:?}"
    );
    ensure!(rows[0].1 == "IN_PROGRESS" && rows[0].4.as_deref() == dir.path().to_str());
    let completed = completion(&log, thread, "completed")?;
    ensure!(!host.finish_run("unknown-thread", &completed)?);
    ensure!(host.finish_run(thread, &completed)?);
    ensure!(runs(dir.path())?[0].1 == "PENDING_REVIEW" && runs(dir.path())?[0].2.is_none());
    ensure!(
        !host.finish_run(thread, &completed)?,
        "a finished run does not finish twice"
    );

    // A refused turn archives the run it opened.
    let refused = create(&host, dir.path(), "REFUSE this")?;
    ensure!(run(&host, &refused).is_err());
    let archived = runs(dir.path())?;
    ensure!(archived.len() == 2);
    ensure!(
        archived
            .iter()
            .any(|r| r.1 == "ARCHIVED" && r.3.as_deref() == Some("auto"))
    );
    ensure!(archived.iter().all(|r| !r.0.starts_with("pending:")));
    Ok(())
}

#[test]
fn a_muted_task_reads_its_completed_run_and_keeps_a_failed_one_unread() -> Result<()> {
    let dir = tempfile::tempdir()?;
    fs::create_dir_all(dir.path().join("sqlite"))?;
    let db = rusqlite::Connection::open(dir.path().join("sqlite/codex.db"))?;
    db.execute(RUNS, [])?;
    db.execute(AUTOMATIONS, [])?;
    drop(db);
    let log = dir.path().join("requests.jsonl");
    let host = host(dir.path(), &log, false)?;
    let toml = format!(
        "version = 1\nid = \"muted\"\nname = \"M\"\nprompt = \"p\"\nstatus = \"ACTIVE\"\ncwds = [{:?}]\ncreated_at = 1\nupdated_at = 1\nexecution_environment = \"local\"\nnotification_policy = \"failed_runs_only\"\n",
        dir.path()
    );
    host.write_automation("muted", &toml)?;
    run(&host, "muted")?;
    run(&host, "muted")?;
    let threads: Vec<String> = runs(dir.path())?.into_iter().map(|r| r.0).collect();
    ensure!(threads.len() == 2);
    ensure!(host.finish_run(&threads[0], &completion(&log, &threads[0], "completed")?)?);
    ensure!(host.finish_run(&threads[1], &completion(&log, &threads[1], "failed")?)?);
    let rows = runs(dir.path())?;
    ensure!(
        rows[0].1 == "PENDING_REVIEW" && rows[0].2.is_some(),
        "completed and muted: read"
    );
    ensure!(
        rows[1].1 == "PENDING_REVIEW" && rows[1].2.is_none(),
        "failed: unread"
    );
    Ok(())
}

#[test]
fn run_now_resolves_a_project_target_to_its_first_root() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let log = dir.path().join("requests.jsonl");
    let root = dir.path().join("project");
    fs::create_dir(&root)?;
    fs::write(
        dir.path().join(".codex-global-state.json"),
        json!({"local-projects": {"p": {"id": "p1", "name": "P", "rootPaths": [root]}}})
            .to_string(),
    )?;
    let host = host(dir.path(), &log, false)?;
    let task = |id: &str, target: &str| {
        format!(
            "version = 1\nid = \"{id}\"\nname = \"N\"\nprompt = \"p\"\nstatus = \"ACTIVE\"\ncwds = []\ncreated_at = 1\nupdated_at = 1\nexecution_environment = \"local\"\n{target}"
        )
    };
    host.write_automation(
        "known",
        &task(
            "known",
            "[target]\ntype = \"project\"\nproject_id = \"p1\"\n",
        ),
    )?;
    host.write_automation(
        "lost",
        &task(
            "lost",
            "[target]\ntype = \"project\"\nproject_id = \"gone\"\n",
        ),
    )?;
    ensure!(run(&host, "lost").is_err());
    ensure!(requests(&log)?.is_empty());
    ensure!(run(&host, "known")? == json!({"success": true}));
    ensure!(requests(&log)?[0]["params"]["cwd"] == json!(root));
    Ok(())
}

fn project_task(
    host: &LocalHost,
    id: &str,
    project: &str,
    environment: &str,
    prompt: &str,
) -> Result<()> {
    host.write_automation(
        id,
        &format!(
            "version = 1\nid = \"{id}\"\nname = \"Review\"\nprompt = \"{prompt}\"\nstatus = \"ACTIVE\"\ncwds = []\ncreated_at = 1\nupdated_at = 1\nexecution_environment = \"{environment}\"\n[target]\ntype = \"project\"\nproject_id = \"{project}\"\n"
        ),
    )
}

#[test]
fn project_runs_save_assignments_before_the_turn_and_keep_other_memberships() -> Result<()> {
    let outer = tempfile::tempdir()?;
    for environment in ["local", "worktree"] {
        let dir = outer.path().join(environment);
        fs::create_dir(&dir)?;
        let home = dir.join("home");
        fs::create_dir(&home)?;
        let repo = repository(&dir)?;
        let project = format!("project-{environment}");
        let state = json!({
            "local-projects": {"p": {"id": project, "rootPaths": [repo, "/unused/root"]}},
            "project-order": ["p"],
            "thread-project-assignments": {
                "owned-thread": {"projectKind": "remote", "projectId": "old", "hostId": "remote"},
                "other": {"projectKind": "local", "projectId": "untouched", "projectOrigin": "chatgpt"}
            },
            "projectless-thread-ids": ["owned-thread", "other"],
            "thread-workspace-root-hints": {"other": "/workspace"},
            "thread-projectless-output-directories": {"other": "/output"},
            "electron-persisted-atom-state": {"keep": [1, 2]},
            "unknown": {"keep": true}
        });
        let path = home.join(".codex-global-state.json");
        fs::write(&path, state.to_string())?;
        let observed = dir.join("state-on-turn.json");
        let host = host_with(
            &home,
            &dir.join("requests.jsonl"),
            false,
            &[
                ("THREAD_ID", "owned-thread"),
                ("STATE_ON_TURN", observed.to_str().unwrap()),
            ],
        )?;
        project_task(&host, "assigned", &project, environment, "Review")?;
        ensure!(run(&host, "assigned")? == json!({"success": true}));
        let mut expected = state;
        expected["thread-project-assignments"]["owned-thread"] =
            json!({"projectKind": "local", "projectId": project});
        let saved: Value = serde_json::from_slice(&fs::read(&path)?)?;
        ensure!(saved == expected, "{saved}");
        ensure!(serde_json::from_slice::<Value>(&fs::read(observed)?)? == expected);
        ensure!(!home.join("sqlite/codex.db").exists());
    }
    for environment in ["local", "worktree"] {
        let path = outer
            .path()
            .join(environment)
            .join("home/.codex-global-state.json");
        let state: Value = serde_json::from_slice(&fs::read(path)?)?;
        ensure!(
            state["thread-project-assignments"]["owned-thread"]["projectId"]
                == format!("project-{environment}")
        );
    }
    Ok(())
}

#[test]
fn assignment_failures_stop_before_the_turn_and_preserve_the_owned_worktree() -> Result<()> {
    for (fault, expected) in [
        ("thread", "no thread"),
        ("state", "thread-project-assignments"),
        ("write", "global state"),
        ("turn", "model unavailable"),
    ] {
        let dir = tempfile::tempdir()?;
        let home = dir.path().join("home");
        fs::create_dir_all(home.join("sqlite"))?;
        let db = rusqlite::Connection::open(home.join("sqlite/codex.db"))?;
        db.execute(RUNS, [])?;
        db.execute(AUTOMATIONS, [])?;
        let repo = repository(dir.path())?;
        let state = json!({"local-projects": {"p": {"id": "p", "rootPaths": [repo]}}});
        let path = home.join(".codex-global-state.json");
        fs::write(&path, state.to_string())?;
        let log = dir.path().join("requests.jsonl");
        let env = match fault {
            "thread" => vec![("FAIL_THREAD", "1")],
            "state" => vec![(
                "STATE_AFTER_THREAD",
                r#"{"thread-project-assignments":[],"keep":true}"#,
            )],
            "write" => vec![("BLOCK_STATE", "1")],
            _ => vec![],
        };
        let host = host_with(&home, &log, false, &env)?;
        project_task(&host, "assigned", "p", "worktree", "REFUSE this")?;
        let error = run(&host, "assigned").unwrap_err().to_string();
        ensure!(error.contains(expected), "{fault}: {error}");
        let sent = requests(&log)?;
        ensure!(sent.len() == if fault == "turn" { 3 } else { 1 });
        let rows = runs(&home)?;
        ensure!(rows.len() == 1 && rows[0].1 == "ARCHIVED" && rows[0].3.as_deref() == Some("auto"));
        if fault == "turn" {
            let thread = sent[0]["id"].as_u64().unwrap();
            let saved: Value = serde_json::from_slice(&fs::read(&path)?)?;
            ensure!(
                saved["thread-project-assignments"][format!("thread-{thread}")]
                    == json!({"projectKind": "local", "projectId": "p"})
            );
            ensure!(!rows[0].0.starts_with("pending:"));
        } else {
            ensure!(rows[0].0.starts_with("pending:"));
            if fault == "write" {
                ensure!(path.is_dir());
                ensure!(
                    fs::read_to_string(home.join(".codex-global-state.json.saved"))?
                        == state.to_string()
                );
            } else if fault == "state" {
                ensure!(fs::read_to_string(&path)? == env[0].1);
            } else {
                ensure!(fs::read_to_string(&path)? == state.to_string());
            }
        }
        let trees = git(&repo, &["worktree", "list", "--porcelain"])?;
        ensure!(trees.matches("worktree ").count() == if fault == "thread" { 1 } else { 2 });
        if fault != "thread" {
            let cwd = Path::new(sent[0]["params"]["cwd"].as_str().unwrap());
            let owner = git(cwd, &["rev-parse", "--git-path", "codex-thread.json"])?;
            let owner: Value = serde_json::from_slice(&fs::read(cwd.join(owner))?)?;
            ensure!(owner["ownerThreadId"] == format!("thread-{}", sent[0]["id"]));
        }
    }
    Ok(())
}

#[test]
fn a_legacy_folder_run_does_not_assign_its_thread_to_a_matching_project() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join(".codex-global-state.json");
    let state = json!({
        "local-projects": {"p": {"id": "p", "rootPaths": [dir.path()]}},
        "thread-project-assignments": {"other": {"projectKind": "remote", "projectId": "r"}}
    })
    .to_string();
    fs::write(&path, &state)?;
    let host = host(dir.path(), &dir.path().join("requests.jsonl"), false)?;
    let id = create(&host, dir.path(), "Review")?;
    ensure!(run(&host, &id)? == json!({"success": true}));
    ensure!(fs::read_to_string(path)? == state);
    Ok(())
}

#[test]
fn adding_projects_during_a_run_keeps_every_global_state_write() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    fs::create_dir(&home)?;
    let repo = repository(dir.path())?;
    fs::write(
        home.join(".codex-global-state.json"),
        json!({"local-projects": {"p": {"id": "p", "rootPaths": [repo]}}}).to_string(),
    )?;
    let host = host(&home, &dir.path().join("requests.jsonl"), false)?;
    project_task(&host, "assigned", "p", "local", "Review")?;
    let folders: Vec<_> = (0..8)
        .map(|i| dir.path().join(format!("folder-{i}")))
        .collect();
    for folder in &folders {
        fs::create_dir(folder)?;
    }
    let barrier = std::sync::Barrier::new(folders.len() + 1);
    std::thread::scope(|scope| -> Result<()> {
        let writers: Vec<_> = folders
            .iter()
            .map(|folder| {
                let (barrier, home) = (&barrier, &home);
                scope.spawn(move || {
                    barrier.wait();
                    LocalHost::new(home).add_project(folder)
                })
            })
            .collect();
        barrier.wait();
        run(&host, "assigned")?;
        for writer in writers {
            writer.join().unwrap()?;
        }
        Ok(())
    })?;
    let state: Value = serde_json::from_slice(&fs::read(home.join(".codex-global-state.json"))?)?;
    let assigned = state["thread-project-assignments"].as_object().unwrap();
    ensure!(
        assigned
            .values()
            .any(|a| a == &json!({"projectKind": "local", "projectId": "p"}))
    );
    for folder in &folders {
        ensure!(
            state["local-projects"]
                .as_object()
                .unwrap()
                .values()
                .any(|p| p["rootPaths"][0] == json!(folder))
        );
    }
    Ok(())
}

#[test]
fn run_now_without_an_app_server_fails_instead_of_pretending() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let host = LocalHost::new(dir.path());
    let id = create(&host, dir.path(), "Review local changes")?;
    ensure!(
        run(&host, &id)
            .unwrap_err()
            .to_string()
            .contains("not connected")
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[path = "common/mod.rs"]
mod common;

/// The actual window: the row menu opens, `Run now` shows `Starting run…` while the app-server
/// holds the turn, the stub receives the thread and turn, and the event task's item is dead.
#[cfg(target_os = "macos")]
#[test]
fn row_menu_run_now_starts_the_task_through_the_app_server() -> Result<()> {
    use common::{Fx, RAIL_SCHEDULED};
    let dir = tempfile::tempdir()?;
    let log = dir.path().join("requests.jsonl");
    let host = host(dir.path(), &log, true)?;
    let task = |id: &str, prompt: &str, updated: u32| {
        format!(
            "version = 1\nid = \"{id}\"\nname = \"{id}\"\nprompt = {prompt:?}\nstatus = \"ACTIVE\"\ncwds = [{:?}]\ncreated_at = 1\nupdated_at = {updated}\nexecution_environment = \"local\"\n",
            dir.path()
        )
    };
    host.write_automation("plain", &task("plain", "Review local changes", 2))?;
    let event = "## When this happens\n> a PR opens\n\n## Perform action\n> review it";
    host.write_automation("event", &task("event", event, 1))?;
    let mut fx = Fx::new()?;
    fx.cx.update(|cx| {
        fx.root
            .update(cx, |root, cx| root.set_host(Arc::new(host), cx))
    });
    fx.click(RAIL_SCHEDULED)?;
    fx.wait("two tasks", |fx| {
        fx.read(|r, _| {
            r.automations().is_some_and(|v| {
                v.as_ref()
                    .is_ok_and(|v| v["items"].as_array().is_some_and(|v| v.len() == 2))
            })
        })
    })?;
    let shots = std::env::var("CGX_RUN_NOW_SHOTS").ok();
    let shot = |fx: &mut Fx, name: &str| -> Result<()> {
        if let Some(out) = &shots {
            fs::create_dir_all(out)?;
            fx.shot()?.save(format!("{out}/{name}.png"))?;
        }
        Ok(())
    };
    let open_menu =
        |fx: &Fx| fx.read(|r, cx| Some(r.scheduled_sidebar()?.read(cx).open_menu()?.to_owned()));
    fx.hover((150., 190.))?;
    shot(&mut fx, "1-row-hover")?;
    fx.click((302., 190.))?;
    ensure!(open_menu(&fx).as_deref() == Some("plain"));
    shot(&mut fx, "2-menu-open")?;
    // The click without running the app to rest: the host call is queued, not yet sent, so the
    // pending state is real. `Fx::click` would run the queued call to its end first.
    fx.hover((250., 228.))?;
    let at = gpui::point(gpui::px(250.), gpui::px(228.));
    fx.cx.update_window(fx.window, |_, window, cx| {
        for event in [
            gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                button: gpui::MouseButton::Left,
                position: at,
                modifiers: gpui::Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            }),
            gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                button: gpui::MouseButton::Left,
                position: at,
                modifiers: gpui::Modifiers::default(),
                click_count: 1,
            }),
        ] {
            window.dispatch_event(event, cx);
        }
    })?;
    ensure!(open_menu(&fx).is_none());
    ensure!(fx.read(|r, cx| r.toasts(cx)) == ["Starting run…"]);
    let starting = |fx: &Fx| {
        fx.read(|r, cx| {
            r.scheduled_sidebar()
                .is_some_and(|s| s.read(cx).is_starting("plain"))
        })
    };
    ensure!(starting(&fx), "the row is Starting run…");
    ensure!(
        requests(&log).unwrap_or_default().is_empty(),
        "nothing sent yet"
    );
    if let Some(out) = &shots {
        fs::create_dir_all(out)?;
        let pending = fx.cx.capture_screenshot(fx.window)?;
        pending.save(format!("{out}/3-starting-run.png"))?;
    }
    // Release the queued call: the app-server receives the thread and turn, holds the turn for
    // two seconds, then answers.
    fx.wait("the run's answer", |fx| !starting(fx))?;
    let sent = requests(&log)?;
    ensure!(sent.len() == 3 && sent[2]["method"] == "turn/start");
    ensure!(sent[0]["params"]["cwd"] == json!(dir.path()));
    ensure!(
        sent[2]["params"]["input"][0]["text"]
            .as_str()
            .is_some_and(|t| t.ends_with("Review local changes"))
    );
    ensure!(
        fx.read(|r, cx| r.toasts(cx)) == ["Starting run…"],
        "a started run shows no error"
    );
    shot(&mut fx, "4-run-started")?;

    fx.hover((150., 239.))?;
    fx.click((302., 239.))?;
    ensure!(open_menu(&fx).as_deref() == Some("event"));
    fx.hover((250., 277.))?;
    fx.advance(std::time::Duration::from_secs(1))?;
    shot(&mut fx, "5-event-task-disabled")?;
    fx.click((250., 277.))?;
    ensure!(requests(&log)?.len() == 3, "an event task starts nothing");
    Ok(())
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()?;
    ensure!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8(out.stdout)?.trim().to_owned())
}

fn repository(root: &Path) -> Result<std::path::PathBuf> {
    let repo = root.join("repo");
    fs::create_dir_all(&repo)?;
    git(&repo, &["init", "-q"])?;
    fs::write(repo.join("a.txt"), "a")?;
    git(&repo, &["add", "."])?;
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "init",
        ],
    )?;
    Ok(repo)
}

/// A task of the original's default environment: no `execution_environment`, so a worktree.
fn worktree_task(host: &LocalHost, id: &str, cwd: &Path) -> Result<()> {
    let toml = format!(
        "version = 1\nid = \"{id}\"\nname = \"W\"\nprompt = \"p\"\nstatus = \"ACTIVE\"\ncwds = [{cwd:?}]\ncreated_at = 1\nupdated_at = 1\n"
    );
    host.write_automation(id, &toml)
}

#[test]
fn a_default_task_runs_in_a_new_worktree_that_its_thread_owns() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let repo = repository(dir.path())?;
    let log = dir.path().join("requests.jsonl");
    let host = host(&home, &log, false)?;
    worktree_task(&host, "wt", &repo)?;
    ensure!(run(&host, "wt")? == json!({"success": true}));
    let sent = requests(&log)?;
    let (start, turn) = (&sent[0]["params"], &sent[2]["params"]);
    let cwd = std::path::PathBuf::from(start["cwd"].as_str().unwrap_or_default());
    ensure!(
        cwd.starts_with(home.join("worktrees")) && cwd.ends_with("repo"),
        "{cwd:?}"
    );
    ensure!(turn["cwd"] == start["cwd"] && cwd.join("a.txt").is_file());
    // The thread owns the worktree, and the turn may write the worktree and the source repository.
    let owner = git(&cwd, &["rev-parse", "--git-path", "codex-thread.json"])?;
    let owner: Value = serde_json::from_str(&fs::read_to_string(cwd.join(owner))?)?;
    ensure!(owner["ownerThreadId"] == turn["threadId"]);
    let roots = turn["sandboxPolicy"]["writableRoots"].to_string();
    ensure!(roots.contains(&*cwd.to_string_lossy()) && roots.contains(&*repo.to_string_lossy()));
    Ok(())
}

#[test]
fn a_default_task_outside_git_runs_in_its_folder() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let log = dir.path().join("requests.jsonl");
    let host = host(&dir.path().join("home"), &log, false)?;
    let plain = dir.path().join("plain");
    fs::create_dir(&plain)?;
    worktree_task(&host, "wt", &plain)?;
    ensure!(run(&host, "wt")? == json!({"success": true}));
    ensure!(requests(&log)?[0]["params"]["cwd"] == json!(plain));
    ensure!(!dir.path().join("home/worktrees").exists());
    Ok(())
}

#[test]
fn a_thread_that_does_not_start_leaves_no_worktree_behind() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let repo = repository(dir.path())?;
    let log = dir.path().join("requests.jsonl");
    let host = host_with(&home, &log, false, &[("FAIL_THREAD", "1")])?;
    worktree_task(&host, "wt", &repo)?;
    let error = run(&host, "wt").unwrap_err().to_string();
    ensure!(error.contains("no thread"), "{error}");
    ensure!(
        git(&repo, &["worktree", "list", "--porcelain"])?
            .matches("worktree ")
            .count()
            == 1
    );
    ensure!(
        requests(&log)?.len() == 1,
        "only the refused thread/start was sent"
    );
    Ok(())
}

#[test]
fn the_default_path_has_config_permissions_run_rows_and_keeps_its_worktree() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    fs::create_dir_all(home.join("sqlite"))?;
    let db = rusqlite::Connection::open(home.join("sqlite/codex.db"))?;
    db.execute(RUNS, [])?;
    db.execute(AUTOMATIONS, [])?;
    drop(db);
    let repo = repository(dir.path())?;
    let log = dir.path().join("requests.jsonl");
    let config = r#"{"sandbox_mode":"workspace-write"}"#;
    let host = host_with(&home, &log, false, &[("CONFIG", config)])?;
    worktree_task(&host, "wt", &repo)?;
    ensure!(run(&host, "wt")? == json!({"success": true}));
    let sent = requests(&log)?;
    let turn = &sent[2]["params"];
    let (cwd, thread) = (
        turn["cwd"].as_str().unwrap_or_default(),
        turn["threadId"].as_str().unwrap_or_default(),
    );
    ensure!(sent[0]["params"]["sandbox"] == "workspace-write" && turn["approvalPolicy"] == "never");
    ensure!(
        turn["sandboxPolicy"]["writableRoots"][0] == json!(repo),
        "the source repository is writable"
    );
    let rows = runs(&home)?;
    ensure!(rows.len() == 1 && rows[0].0 == thread && rows[0].1 == "IN_PROGRESS");
    ensure!(
        rows[0].4.as_deref() == repo.to_str(),
        "the row names the source folder, not the worktree"
    );
    ensure!(host.finish_run(thread, &completion(&log, thread, "completed")?)?);
    ensure!(runs(&home)?[0].1 == "PENDING_REVIEW");
    ensure!(
        Path::new(cwd).join("a.txt").is_file(),
        "a finished run keeps its worktree"
    );

    // A turn the server refuses may already have produced work: the worktree stays, owned.
    let refused = repository(&dir.path().join("second"))?;
    fs::write(refused.join("REFUSE"), "")?;
    worktree_task(&host, "refused", &refused)?;
    host.write_automation(
        "refused",
        &fs::read_to_string(home.join("automations/refused/automation.toml"))?
            .replace("\"p\"", "\"REFUSE this\""),
    )?;
    ensure!(run(&host, "refused").is_err());
    ensure!(
        git(&refused, &["worktree", "list", "--porcelain"])?
            .matches("worktree ")
            .count()
            == 2
    );
    ensure!(
        runs(&home)?
            .iter()
            .any(|r| r.1 == "ARCHIVED" && r.3.as_deref() == Some("auto"))
    );
    Ok(())
}

fn schedule(home: &Path, id: &str) -> Result<(Option<i64>, Option<i64>, Option<i64>)> {
    let db = rusqlite::Connection::open(home.join("sqlite/codex.db"))?;
    Ok(db.query_row(
        "SELECT next_run_at, next_run_nominal_at, last_run_at FROM automations WHERE id = ?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?)
}

#[test]
fn a_run_now_records_the_last_run_and_keeps_or_recalculates_the_next_one() -> Result<()> {
    let dir = tempfile::tempdir()?;
    fs::create_dir_all(dir.path().join("sqlite"))?;
    let db = rusqlite::Connection::open(dir.path().join("sqlite/codex.db"))?;
    db.execute(RUNS, [])?;
    db.execute(AUTOMATIONS, [])?;
    let log = dir.path().join("requests.jsonl");
    let host = host(dir.path(), &log, false)?;
    let rule = "FREQ=DAILY;BYHOUR=9;BYMINUTE=0";
    let (ahead, now) = (
        chrono::Utc::now().timestamp_millis() + 3_600_000,
        chrono::Utc::now().timestamp_millis(),
    );
    let stored = |id: &str, next: i64| {
        db.execute(
            "INSERT INTO automations (id, next_run_at, next_run_nominal_at, rrule, status) VALUES (?, ?, ?, ?, 'ACTIVE')",
            rusqlite::params![id, next, next, rule],
        )
    };
    // A next run that is still ahead stays; the run becomes the last run.
    let keep = create(&host, dir.path(), "keep")?;
    stored(&keep, ahead)?;
    ensure!(run(&host, &keep)? == json!({"success": true}));
    let (next, nominal, last) = schedule(dir.path(), &keep)?;
    ensure!(next == Some(ahead) && nominal == Some(ahead));
    ensure!(last.is_some_and(|last| last >= now));
    // A next run that is already past is recalculated to a future time.
    let past = create(&host, dir.path(), "past")?;
    stored(&past, 1_000)?;
    ensure!(run(&host, &past)? == json!({"success": true}));
    let (next, nominal, last) = schedule(dir.path(), &past)?;
    ensure!(next.is_some_and(|next| next > now) && nominal.is_some_and(|n| n > now - 1));
    ensure!(last.is_some_and(|last| last >= now));
    // A task with no database row gets one, so its last run is not lost.
    let fresh = create(&host, dir.path(), "fresh")?;
    ensure!(run(&host, &fresh)? == json!({"success": true}));
    let (next, _, last) = schedule(dir.path(), &fresh)?;
    ensure!(next.is_some_and(|next| next > now) && last.is_some());
    Ok(())
}

fn projectless_task(host: &LocalHost, home: &Path, root: &Path, prompt: &str) -> Result<()> {
    fs::create_dir_all(home.join("sqlite"))?;
    let db = rusqlite::Connection::open(home.join("sqlite/codex.db"))?;
    db.execute(RUNS, [])?;
    db.execute(AUTOMATIONS, [])?;
    fs::write(
        home.join("host-settings.json"),
        json!({"projectlessWorkspaceRoot": root}).to_string(),
    )?;
    host.write_automation("free", &format!(
        "version = 1\nid = \"free\"\nname = \"Free\"\nprompt = {prompt:?}\nstatus = \"ACTIVE\"\ncwds = []\ncreated_at = 1\nupdated_at = 1\n[target]\ntype = \"projectless\"\n"
    ))
}

#[test]
fn projectless_runs_own_distinct_directories_and_persist_thread_metadata() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let root = dir.path().join("workspace");
    let log = dir.path().join("requests.jsonl");
    let config =
        r#"{"sandbox_mode":"workspace-write","sandbox_workspace_write":{"network_access":true}}"#;
    let host = host_with(&home, &log, false, &[("CONFIG", config)])?;
    projectless_task(
        &host,
        &home,
        &root,
        "İnfo REVIEW local changes with six words",
    )?;
    let initial = json!({"local-projects":{"keep":{"id":"keep"}}, "projectless-thread-ids":["older"],
        "thread-workspace-root-hints":{"older":"/old"}, "thread-projectless-output-directories":{"older":"/old/outputs"},
        "thread-project-assignments":{"older":{"projectKind":"local","projectId":"keep"}}});
    fs::write(home.join(".codex-global-state.json"), initial.to_string())?;
    run(&host, "free")?;
    run(&host, "free")?;
    let sent = requests(&log)?;
    let first = Path::new(sent[0]["params"]["cwd"].as_str().unwrap());
    let second = Path::new(sent[3]["params"]["cwd"].as_str().unwrap());
    ensure!(first.file_name().unwrap() == "i-nfo-review-local-changes-with");
    ensure!(second.file_name().unwrap() == "i-nfo-review-local-changes-with-2");
    ensure!(
        first.parent() == second.parent()
            && first.parent().unwrap().parent() == Some(root.as_path())
    );
    let state: Value = serde_json::from_slice(&fs::read(home.join(".codex-global-state.json"))?)?;
    for run in sent.as_chunks::<3>().0 {
        let cwd = Path::new(run[0]["params"]["cwd"].as_str().unwrap());
        let thread = run[2]["params"]["threadId"].as_str().unwrap();
        ensure!(cwd.join("outputs").is_dir() && cwd.join("work").is_dir());
        ensure!(
            state["projectless-thread-ids"]
                .as_array()
                .unwrap()
                .contains(&json!(thread))
        );
        ensure!(state["thread-workspace-root-hints"][thread] == json!(root));
        ensure!(
            state["thread-projectless-output-directories"][thread] == json!(cwd.join("outputs"))
        );
        let policy = &run[2]["params"]["sandboxPolicy"];
        ensure!(policy["networkAccess"] == true);
        ensure!(
            policy["writableRoots"]
                .as_array()
                .unwrap()
                .contains(&json!(root))
        );
        ensure!(
            run[0]["params"]["approvalPolicy"] == "never"
                && run[2]["params"]["approvalsReviewer"] == "user"
        );
    }
    ensure!(state["local-projects"] == initial["local-projects"]);
    ensure!(state["thread-project-assignments"] == initial["thread-project-assignments"]);
    ensure!(state["thread-workspace-root-hints"]["older"] == "/old");
    ensure!(runs(&home)?.iter().all(|row| row.4.is_none()));
    ensure!(!home.join("worktrees").exists());
    Ok(())
}

fn heartbeat_definition(host: &LocalHost, prompt: &str) -> Result<()> {
    host.write_automation("heartbeat", &format!(
        "version = 1\nid = \"heartbeat\"\nname = \"Monitor\"\nprompt = {prompt:?}\nstatus = \"ACTIVE\"\nkind = \"heartbeat\"\ntarget_thread_id = \"heartbeat-thread\"\nrrule = \"FREQ=MINUTELY;INTERVAL=30\"\ncreated_at = 1\nupdated_at = 1\n"
    ))
}

fn heartbeat_params() -> Value {
    json!({"id": "heartbeat", "collaborationMode": {"mode": "plan", "settings": {
        "model": "gpt-6.1-sol", "reasoning_effort": "low", "developer_instructions": "Keep the user's instructions",
    }}, "permissions": {
        "approvalPolicy": "on-request", "approvalsReviewer": "user",
        "sandboxPolicy": {"type": "readOnly", "networkAccess": false},
    }})
}

fn heartbeat_run(host: &LocalHost, params: Value) -> Result<Value> {
    json_body(&host.fetch(&Request::post("automation-run-now", Some(params)))?)
}

#[test]
fn heartbeat_reuses_its_thread_with_renderer_mode_permissions_and_interval_state() -> Result<()> {
    let dir = tempfile::tempdir()?;
    fs::create_dir_all(dir.path().join("sqlite"))?;
    let db = rusqlite::Connection::open(dir.path().join("sqlite/codex.db"))?;
    db.execute_batch(AUTOMATIONS)?;
    db.execute_batch(RUNS)?;
    let thread = json!({"id": "heartbeat-thread", "cwd": dir.path(), "path": "/fixture/session.jsonl", "status": {"type": "idle"}});
    let log = dir.path().join("requests.jsonl");
    let host = host_with(dir.path(), &log, false, &[("THREAD", &thread.to_string())])?;
    heartbeat_definition(&host, "Check the fixture job")?;
    let before = chrono::Utc::now().timestamp_millis();
    ensure!(heartbeat_run(&host, heartbeat_params())? == json!({"success": true}));
    let sent = all_requests(&log)?;
    ensure!(
        sent.iter()
            .map(|r| r["method"].as_str().unwrap())
            .collect::<Vec<_>>()
            == ["thread/read", "model/list", "thread/resume", "turn/start"]
    );
    ensure!(
        sent[2]["params"]
            == json!({"threadId": "heartbeat-thread", "cwd": dir.path(), "path": "/fixture/session.jsonl"})
    );
    let turn = &sent[3]["params"];
    ensure!(
        turn["threadId"] == "heartbeat-thread"
            && turn["model"].is_null()
            && turn["effort"].is_null()
    );
    ensure!(turn["collaborationMode"]["mode"] == "plan");
    ensure!(turn["collaborationMode"]["settings"]["model"] == "gpt-6.1-sol");
    ensure!(turn["collaborationMode"]["settings"]["reasoning_effort"] == "low");
    let instructions = turn["collaborationMode"]["settings"]["developer_instructions"]
        .as_str()
        .unwrap();
    ensure!(instructions.starts_with("Keep the user's instructions\n\n## Heartbeats"));
    ensure!(instructions.contains("DONT_NOTIFY") && instructions.contains("automation_update"));
    ensure!(turn["approvalPolicy"] == "on-request" && turn["sandboxPolicy"]["type"] == "readOnly");
    let text = turn["input"][0]["text"].as_str().unwrap();
    ensure!(text.starts_with(
        "<heartbeat>\n  <automation_id>heartbeat</automation_id>\n  <current_time_iso>"
    ));
    ensure!(
        text.ends_with("<instructions>\nCheck the fixture job\n  </instructions>\n</heartbeat>\n")
    );
    let (last, next, nominal): (i64, i64, i64) = db.query_row(
        "SELECT last_run_at, next_run_at, next_run_nominal_at FROM automations WHERE id='heartbeat'",
        [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    )?;
    ensure!(last >= before && nominal == last + 30 * 60_000 && next == nominal);
    ensure!(
        runs(dir.path())?.is_empty(),
        "a heartbeat must not open a cron run row"
    );
    Ok(())
}

#[test]
fn projectless_refuses_relative_and_symlink_roots_before_starting_a_thread() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let log = dir.path().join("requests.jsonl");
    let host = host(&home, &log, false)?;
    projectless_task(&host, &home, Path::new("relative"), "p")?;
    ensure!(
        run(&host, "free")
            .unwrap_err()
            .to_string()
            .contains("absolute")
    );
    let real = dir.path().join("real");
    let link = dir.path().join("link");
    fs::create_dir(&real)?;
    std::os::unix::fs::symlink(&real, &link)?;
    fs::write(
        home.join("host-settings.json"),
        json!({"projectlessWorkspaceRoot": link}).to_string(),
    )?;
    ensure!(
        run(&host, "free")
            .unwrap_err()
            .to_string()
            .contains("real directory")
    );
    fs::write(
        home.join("host-settings.json"),
        json!({"projectlessWorkspaceRoot": real}).to_string(),
    )?;
    let date = real.join(chrono::Local::now().format("%Y-%m-%d").to_string());
    let outside = dir.path().join("outside");
    fs::create_dir(&outside)?;
    std::os::unix::fs::symlink(&outside, &date)?;
    ensure!(
        run(&host, "free")
            .unwrap_err()
            .to_string()
            .contains("real directory")
    );
    ensure!(fs::read_dir(&outside)?.next().is_none());
    ensure!(requests(&log)?.is_empty() && runs(&home)?.is_empty());
    Ok(())
}

#[test]
fn projectless_start_and_turn_failures_keep_owned_files_and_truthful_run_state() -> Result<()> {
    for fail_thread in [true, false] {
        let dir = tempfile::tempdir()?;
        let home = dir.path().join("home");
        let root = dir.path().join("workspace");
        let log = dir.path().join("requests.jsonl");
        let host = host_with(
            &home,
            &log,
            false,
            &[("FAIL_THREAD", if fail_thread { "1" } else { "" })],
        )?;
        projectless_task(&host, &home, &root, "REFUSE")?;
        let error = run(&host, "free").unwrap_err().to_string();
        ensure!(error.contains(if fail_thread {
            "no thread"
        } else {
            "model unavailable"
        }));
        let sent = requests(&log)?;
        let cwd = Path::new(sent[0]["params"]["cwd"].as_str().unwrap());
        ensure!(cwd.join("outputs").is_dir() && cwd.join("work").is_dir());
        let rows = runs(&home)?;
        ensure!(rows[0].1 == "ARCHIVED" && rows[0].3.as_deref() == Some("auto"));
        ensure!(rows[0].0.starts_with("pending:") == fail_thread);
        ensure!(home.join(".codex-global-state.json").exists() != fail_thread);
    }
    Ok(())
}

#[test]
fn heartbeat_refuses_missing_busy_loading_and_failed_threads_without_a_replacement() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let idle = json!({"id": "heartbeat-thread", "cwd": dir.path(), "status": {"type": "idle"}});
    let busy = json!({"id": "heartbeat-thread", "cwd": dir.path(), "status": {"type": "active", "activeFlags": ["waitingOnApproval"]}});
    let cases = [
        (
            "missing",
            json!(null),
            "Heartbeat thread not found.",
            Vec::new(),
        ),
        ("busy", busy, "busy right now", Vec::new()),
        ("loading", idle.clone(), "mode is still loading", Vec::new()),
        (
            "resume",
            idle.clone(),
            "resume refused",
            vec![("FAIL_RESUME", "1")],
        ),
        (
            "requirements",
            idle.clone(),
            "requirements unavailable",
            vec![("FAIL_REQUIREMENTS", "1")],
        ),
        ("turn", idle, "model unavailable", Vec::new()),
    ];
    for (case, thread, error, mut env) in cases {
        let home = dir.path().join(case);
        fs::create_dir_all(&home)?;
        let log = home.join("requests.jsonl");
        let thread = thread.to_string();
        env.push(("THREAD", &thread));
        let host = host_with(&home, &log, false, &env)?;
        heartbeat_definition(&host, if case == "turn" { "REFUSE" } else { "Check" })?;
        let mut params = heartbeat_params();
        if case == "loading" {
            params["collaborationMode"] = Value::Null;
        }
        if case == "requirements" {
            params["permissions"] = Value::Null;
        }
        let actual = heartbeat_run(&host, params).unwrap_err().to_string();
        ensure!(actual.contains(error), "{case}: {actual}");
        let sent = all_requests(&log)?;
        ensure!(!sent.iter().any(|r| matches!(
            r["method"].as_str(),
            Some("thread/start" | "thread/name/set")
        )));
        if matches!(case, "missing" | "busy" | "loading") {
            ensure!(
                !sent
                    .iter()
                    .any(|r| r["method"] == "thread/resume" || r["method"] == "turn/start")
            );
        } else if case == "requirements" {
            ensure!(!sent.iter().any(|r| r["method"] == "turn/start"));
        }
    }
    Ok(())
}

#[test]
fn projectless_metadata_write_failure_starts_no_turn_and_preserves_state() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let log = dir.path().join("requests.jsonl");
    let host = host(&home, &log, false)?;
    projectless_task(&host, &home, &dir.path().join("workspace"), "p")?;
    fs::write(home.join(".codex-global-state.json"), b"{invalid")?;
    ensure!(run(&host, "free").is_err());
    let sent = requests(&log)?;
    ensure!(sent.iter().all(|request| request["method"] != "turn/start"));
    ensure!(fs::read(home.join(".codex-global-state.json"))? == b"{invalid");
    ensure!(
        Path::new(sent[0]["params"]["cwd"].as_str().unwrap())
            .join("outputs")
            .is_dir()
    );
    ensure!(runs(&home)?[0].1 == "ARCHIVED");
    Ok(())
}

struct ArchiveRun {
    _directory: tempfile::TempDir,
    home: std::path::PathBuf,
    log: std::path::PathBuf,
    cwd: std::path::PathBuf,
    host: Arc<LocalHost>,
    bridge: Arc<Bridge>,
    events: async_channel::Receiver<bridge::Event>,
    thread: String,
    turn: Value,
}

impl ArchiveRun {
    fn start(auto_archive: bool, muted: bool, env: &[(&str, &str)]) -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let home = directory.path().join("home");
        fs::create_dir_all(home.join("sqlite"))?;
        let db = rusqlite::Connection::open(home.join("sqlite/codex.db"))?;
        db.execute_batch(&format!("{AUTOMATIONS};{RUNS};"))?;
        let repo = repository(directory.path())?;
        let log = directory.path().join("requests.jsonl");
        let mut server = Command::new("sh");
        server
            .arg("-c")
            .arg(SERVER)
            .env("LOG", &log)
            .envs(env.iter().copied());
        let (bridge, events) = Bridge::spawn(server)?;
        let bridge = Arc::new(bridge);
        let host = Arc::new(LocalHost::new(&home).with_bridge(bridge.clone()));
        let policy = if muted {
            "notification_policy = \"failed_runs_only\"\n"
        } else {
            ""
        };
        host.write_automation("archive", &format!(
            "version = 1\nid = \"archive\"\nname = \"Review\"\nprompt = \"Review changes\"\nstatus = \"ACTIVE\"\ncwds = [{repo:?}]\ncreated_at = 1\nupdated_at = 1\nauto_archive = {auto_archive}\n{policy}"
        ))?;
        run(&host, "archive")?;
        let sent = requests(&log)?;
        let thread = sent[2]["params"]["threadId"].as_str().unwrap().to_owned();
        let cwd = sent[2]["params"]["cwd"].as_str().unwrap().into();
        let turn = completion(&log, &thread, "completed")?;
        Ok(Self {
            _directory: directory,
            home,
            log,
            cwd,
            host,
            bridge,
            events,
            thread,
            turn,
        })
    }

    fn db(&self) -> Result<rusqlite::Connection> {
        Ok(rusqlite::Connection::open(
            self.home.join("sqlite/codex.db"),
        )?)
    }

    fn row(&self) -> Result<Run> {
        Ok(runs(&self.home)?.remove(0))
    }

    fn finish(&self) -> Result<bool> {
        self.host.finish_automation_run(&self.thread, &self.turn)
    }

    fn archive_requests(&self) -> Result<Vec<String>> {
        Ok(all_requests(&self.log)?
            .into_iter()
            .filter_map(|request| {
                let method = request["method"].as_str()?;
                matches!(method, "thread/archive" | "thread/unarchive").then(|| method.to_owned())
            })
            .collect())
    }
}

#[test]
fn autoarchive_uses_the_initial_policy_and_restores_before_user_work() -> Result<()> {
    for muted in [false, true] {
        let run = ArchiveRun::start(true, muted, &[])?;
        fs::write(run.cwd.join("work.txt"), "automation work")?;
        let read_at = muted.then_some(17);
        run.db()?
            .execute("UPDATE automation_runs SET read_at = ?", [read_at])?;
        let definition = run.home.join("automations/archive/automation.toml");
        let mut edited = fs::read_to_string(&definition)?
            .replace("auto_archive = true", "auto_archive = false")
            .replace("notification_policy = \"failed_runs_only\"\n", "");
        if !muted {
            edited.push_str("notification_policy = \"failed_runs_only\"\n");
        }
        fs::write(&definition, edited)?;
        ensure!(run.finish()?);
        let row = run.row()?;
        ensure!(row.1 == "ARCHIVED" && row.2 == read_at && row.3.as_deref() == Some("auto"));
        ensure!(
            !run.finish()?,
            "a repeated completion must not archive twice"
        );
        pollster::block_on(run.bridge.request::<_, Value>(
            "turn/start",
            json!({
                "threadId": run.thread, "input": [{"type": "text", "text": "Continue"}]
            }),
        ))?;
        let row = run.row()?;
        ensure!(row.1 == "ACCEPTED" && row.2.is_some() && row.3.is_none());
        if muted {
            ensure!(row.2 == Some(17));
        }
        ensure!(run.archive_requests()? == ["thread/archive", "thread/unarchive"]);
        ensure!(fs::read_to_string(run.cwd.join("work.txt"))? == "automation work");
        ensure!(run.cwd.join("a.txt").is_file(), "the worktree stays");
    }
    Ok(())
}

#[test]
fn autoarchive_keeps_failed_interrupted_disabled_and_heartbeat_runs_for_review() -> Result<()> {
    for (auto_archive, status, heartbeat) in [
        (true, "failed", false),
        (true, "interrupted", false),
        (false, "completed", false),
        (true, "completed", true),
    ] {
        let mut run = ArchiveRun::start(auto_archive, true, &[])?;
        run.db()?
            .execute("UPDATE automation_runs SET read_at = 17", [])?;
        run.turn["status"] = json!(status);
        if heartbeat {
            run.host.write_automation("heartbeat", &format!(
                "version = 1\nid = \"heartbeat\"\nname = \"Heartbeat\"\nprompt = \"Check\"\nstatus = \"PAUSED\"\nkind = \"heartbeat\"\ntarget_thread_id = {:?}\ncreated_at = 1\nupdated_at = 1\n",
                run.thread
            ))?;
        }
        ensure!(run.finish()?);
        let row = run.row()?;
        ensure!(row.1 == "PENDING_REVIEW" && row.3.is_none(), "{row:?}");
        ensure!(
            row.2
                == if status == "completed" {
                    Some(17)
                } else {
                    None
                }
        );
        ensure!(run.archive_requests()?.is_empty());
        ensure!(run.cwd.join("a.txt").is_file());
    }
    Ok(())
}

#[test]
fn autoarchive_cancels_for_every_user_request_and_rejects_other_turn_completions() -> Result<()> {
    for method in [
        "turn/start",
        "turn/steer",
        "turn/addUserMessage",
        "thread/queue/add",
        "thread/queue/start",
    ] {
        let run = ArchiveRun::start(true, false, &[])?;
        pollster::block_on(
            run.bridge
                .request::<_, Value>(method, json!({"threadId": run.thread})),
        )?;
        ensure!(!run.host.finish_run(
            &run.thread,
            &json!({"id": "another-turn", "status": "completed"})
        )?);
        ensure!(run.row()?.1 == "IN_PROGRESS");
        ensure!(run.finish()?);
        ensure!(run.row()?.1 == "PENDING_REVIEW");
        ensure!(run.archive_requests()?.is_empty(), "{method}");
    }
    Ok(())
}

#[test]
fn autoarchive_refusal_keeps_the_review_row_and_notification_policy() -> Result<()> {
    let run = ArchiveRun::start(true, true, &[("ARCHIVE_FAIL", "1")])?;
    ensure!(run.finish()?);
    let row = run.row()?;
    ensure!(row.1 == "PENDING_REVIEW" && row.2.is_some() && row.3.is_none());
    ensure!(run.archive_requests()? == ["thread/archive"]);
    ensure!(run.cwd.join("a.txt").is_file());
    Ok(())
}

#[test]
fn autoarchive_retries_a_failed_completion_write_and_undoes_a_failed_archive_write() -> Result<()> {
    let run = ArchiveRun::start(true, true, &[])?;
    let db = run.db()?;
    db.execute_batch(
        "CREATE TRIGGER fail_finish BEFORE UPDATE ON automation_runs
        WHEN NEW.status = 'PENDING_REVIEW' BEGIN SELECT RAISE(ABORT, 'finish failed'); END;",
    )?;
    ensure!(
        run.finish()
            .unwrap_err()
            .to_string()
            .contains("finish failed")
    );
    ensure!(run.row()?.1 == "IN_PROGRESS" && run.archive_requests()?.is_empty());
    db.execute_batch(
        "DROP TRIGGER fail_finish;
        CREATE TRIGGER fail_archive BEFORE UPDATE ON automation_runs
        WHEN NEW.status = 'ARCHIVED' BEGIN SELECT RAISE(ABORT, 'archive write failed'); END;",
    )?;
    ensure!(run.finish()?);
    let row = run.row()?;
    ensure!(row.1 == "PENDING_REVIEW" && row.2.is_some() && row.3.is_none());
    ensure!(run.archive_requests()? == ["thread/archive", "thread/unarchive"]);
    ensure!(run.cwd.join("a.txt").is_file());
    Ok(())
}

#[test]
fn autoarchive_restores_an_inflight_archive_before_sending_the_user_request() -> Result<()> {
    let sync = tempfile::tempdir()?;
    let release = sync.path().join("release");
    let run = ArchiveRun::start(
        true,
        false,
        &[
            ("HOLD_ARCHIVE", "1"),
            ("ARCHIVE_RELEASE", release.to_str().unwrap()),
        ],
    )?;
    let host = run.host.clone();
    let (thread, turn) = (run.thread.clone(), run.turn.clone());
    let finishing = std::thread::spawn(move || host.finish_run(&thread, &turn));
    let bridge::Event::Notification { method, params } = pollster::block_on(run.events.recv())?
    else {
        anyhow::bail!("expected archive notification");
    };
    ensure!(method == "thread/archived" && params["threadId"] == run.thread);
    let mut user = std::pin::pin!(
        run.bridge
            .request::<_, Value>("thread/queue/add", json!({"threadId": run.thread}))
    );
    ensure!(
        user.as_mut()
            .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
            .is_pending()
    );
    fs::write(&release, "")?;
    ensure!(finishing.join().unwrap()?);
    pollster::block_on(user)?;
    ensure!(run.row()?.1 == "ACCEPTED");
    ensure!(run.archive_requests()? == ["thread/archive", "thread/unarchive"]);
    let methods: Vec<_> = all_requests(&run.log)?
        .into_iter()
        .filter_map(|request| request["method"].as_str().map(str::to_owned))
        .collect();
    ensure!(methods.ends_with(&[
        "thread/archive".into(),
        "thread/unarchive".into(),
        "thread/queue/add".into()
    ]));
    ensure!(run.cwd.join("a.txt").is_file());
    Ok(())
}

#[test]
fn autoarchive_restore_failure_blocks_user_work_until_restore_succeeds() -> Result<()> {
    let sync = tempfile::tempdir()?;
    let fail = sync.path().join("fail");
    fs::write(&fail, "")?;
    let run = ArchiveRun::start(
        true,
        false,
        &[("RESTORE_FAIL_FILE", fail.to_str().unwrap())],
    )?;
    ensure!(run.finish()?);
    let user = || {
        run.bridge
            .request::<_, Value>("thread/queue/add", json!({"threadId": run.thread}))
    };
    ensure!(
        pollster::block_on(user())
            .unwrap_err()
            .to_string()
            .contains("restore refused")
    );
    ensure!(run.row()?.1 == "ARCHIVED");
    ensure!(
        !all_requests(&run.log)?
            .iter()
            .any(|request| request["method"] == "thread/queue/add")
    );
    fs::remove_file(&fail)?;
    pollster::block_on(user())?;
    ensure!(run.row()?.1 == "ACCEPTED");
    ensure!(run.cwd.join("a.txt").is_file());
    Ok(())
}

#[test]
fn autoarchive_waits_for_the_start_reply_when_completion_arrives_first() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let home = directory.path().join("home");
    fs::create_dir_all(home.join("sqlite"))?;
    let db = rusqlite::Connection::open(home.join("sqlite/codex.db"))?;
    db.execute_batch(&format!("{AUTOMATIONS};{RUNS};"))?;
    let release = directory.path().join("release");
    let log = directory.path().join("requests.jsonl");
    let mut server = Command::new("sh");
    server
        .arg("-c")
        .arg(SERVER)
        .env("LOG", &log)
        .env("HOLD_TURN", "1")
        .env("TURN_RELEASE", &release);
    let (bridge, events) = Bridge::spawn(server)?;
    let bridge = Arc::new(bridge);
    let host = Arc::new(LocalHost::new(&home).with_bridge(bridge.clone()));
    let id = create(&host, &home, "Review")?;
    let definition = home.join("automations").join(&id).join("automation.toml");
    fs::write(
        &definition,
        format!("{}auto_archive = true\n", fs::read_to_string(&definition)?),
    )?;
    let starter = host.clone();
    let starting = std::thread::spawn(move || run(&starter, &id));
    let bridge::Event::Notification { method, params } = pollster::block_on(events.recv())? else {
        anyhow::bail!("expected completion");
    };
    ensure!(method == "turn/completed");
    let thread = params["threadId"].as_str().unwrap();
    let tracked = bridge.automation_turn(thread).unwrap();
    let mut ready = std::pin::pin!(tracked.matches_turn(params["turn"]["id"].as_str().unwrap()));
    ensure!(
        ready
            .as_mut()
            .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
            .is_pending()
    );
    let finisher = host.clone();
    let completion = params.clone();
    let finishing = std::thread::spawn(move || {
        finisher.finish_automation_run(
            completion["threadId"].as_str().unwrap(),
            &completion["turn"],
        )
    });
    fs::write(&release, "")?;
    ensure!(starting.join().unwrap()? == json!({"success": true}));
    ensure!(finishing.join().unwrap()?);
    ensure!(runs(&home)?[0].1 == "ARCHIVED");
    Ok(())
}

#[test]
fn concurrent_hosts_preserve_all_project_updates() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let roots: Vec<_> = (0..12)
        .map(|id| dir.path().join(format!("project-{id}")))
        .collect();
    for root in &roots {
        fs::create_dir(root)?;
    }
    let expected = roots
        .iter()
        .map(fs::canonicalize)
        .collect::<std::io::Result<std::collections::BTreeSet<_>>>()?;
    let barrier = std::sync::Barrier::new(roots.len());
    std::thread::scope(|scope| -> Result<()> {
        let (home, barrier) = (&home, &barrier);
        let writers: Vec<_> = roots
            .iter()
            .map(|root| {
                scope.spawn(move || {
                    barrier.wait();
                    LocalHost::new(home).add_project(root)
                })
            })
            .collect();
        for writer in writers {
            writer.join().expect("project writer panicked")?;
        }
        Ok(())
    })?;
    let state: Value = serde_json::from_slice(&fs::read(home.join(".codex-global-state.json"))?)?;
    let saved: std::collections::BTreeSet<_> = state["local-projects"]
        .as_object()
        .unwrap()
        .values()
        .map(|project| std::path::PathBuf::from(project["rootPaths"][0].as_str().unwrap()))
        .collect();
    ensure!(saved == expected, "concurrent writes lost a project");
    Ok(())
}

#[test]
fn autoarchive_does_not_finish_an_account_owned_run() -> Result<()> {
    let run = ArchiveRun::start(true, false, &[])?;
    run.db()?
        .execute("UPDATE automations SET account_id = 'another-account'", [])?;
    ensure!(!run.finish()?);
    ensure!(run.row()?.1 == "IN_PROGRESS");
    ensure!(run.archive_requests()?.is_empty());
    Ok(())
}

#[test]
fn projectless_requirements_failure_creates_no_workspace_or_run() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let home = dir.path().join("home");
    let root = dir.path().join("workspace");
    let log = dir.path().join("requests.jsonl");
    let host = host_with(&home, &log, false, &[("FAIL_REQUIREMENTS", "1")])?;
    projectless_task(&host, &home, &root, "p")?;
    ensure!(run(&host, "free").is_err());
    ensure!(!root.exists() && !home.join(".codex-global-state.json").exists());
    ensure!(runs(&home)?.is_empty() && requests(&log)?.is_empty());
    Ok(())
}

#[test]
fn autoarchive_finishes_a_run_that_already_has_an_inbox_result() -> Result<()> {
    let run = ArchiveRun::start(true, true, &[])?;
    run.db()?.execute(
        "UPDATE automation_runs SET status = 'PENDING_REVIEW', inbox_title = 'Result'",
        [],
    )?;
    ensure!(run.finish()?);
    let row = run.row()?;
    ensure!(row.1 == "ARCHIVED" && row.2.is_some());
    ensure!(!run.finish()?);
    Ok(())
}

#[test]
fn autoarchive_retries_the_restore_write_without_repeating_the_server_unarchive() -> Result<()> {
    let run = ArchiveRun::start(true, false, &[])?;
    ensure!(run.finish()?);
    let db = run.db()?;
    db.execute_batch(
        "CREATE TRIGGER fail_restore BEFORE UPDATE ON automation_runs
        WHEN NEW.status = 'ACCEPTED' BEGIN SELECT RAISE(ABORT, 'restore write failed'); END;",
    )?;
    let user = || {
        run.bridge
            .request::<_, Value>("thread/queue/add", json!({"threadId": run.thread}))
    };
    ensure!(
        pollster::block_on(user())
            .unwrap_err()
            .to_string()
            .contains("restore write failed")
    );
    ensure!(run.row()?.1 == "ARCHIVED");
    ensure!(run.archive_requests()? == ["thread/archive", "thread/unarchive"]);
    db.execute_batch("DROP TRIGGER fail_restore;")?;
    pollster::block_on(user())?;
    ensure!(run.row()?.1 == "ACCEPTED");
    ensure!(run.archive_requests()? == ["thread/archive", "thread/unarchive"]);
    ensure!(run.cwd.join("a.txt").is_file());
    Ok(())
}

#[test]
fn unrecorded_completion_releases_only_its_matching_tracker_without_archiving() -> Result<()> {
    for database in [false, true] {
        let directory = tempfile::tempdir()?;
        let home = directory.path().join("home");
        if database {
            fs::create_dir_all(home.join("sqlite"))?;
            rusqlite::Connection::open(home.join("sqlite/codex.db"))?
                .execute_batch(&format!("{AUTOMATIONS};{RUNS};"))?;
        }
        let log = directory.path().join("requests.jsonl");
        let mut server = Command::new("sh");
        server.arg("-c").arg(SERVER).env("LOG", &log);
        let (bridge, _events) = Bridge::spawn(server)?;
        let bridge = Arc::new(bridge);
        let host = LocalHost::new(&home).with_bridge(bridge.clone());
        let id = create(&host, &home, "Review")?;
        run(&host, &id)?;
        let sent = requests(&log)?;
        let thread = sent[2]["params"]["threadId"].as_str().unwrap();
        if database {
            rusqlite::Connection::open(home.join("sqlite/codex.db"))?
                .execute("DELETE FROM automation_runs", [])?;
        }
        ensure!(!host.finish_run(
            thread,
            &json!({"id": "another-turn", "status": "completed"})
        )?);
        ensure!(bridge.automation_turn(thread).is_some());
        ensure!(!host.finish_run(thread, &completion(&log, thread, "completed")?)?);
        ensure!(bridge.automation_turn(thread).is_none());
        ensure!(
            !all_requests(&log)?
                .iter()
                .any(|request| request["method"] == "thread/archive")
        );
    }
    Ok(())
}

#[test]
fn projectless_autoarchive_keeps_its_workspace_and_thread_metadata() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let home = directory.path().join("home");
    let root = directory.path().join("workspace");
    let log = directory.path().join("requests.jsonl");
    let host = host(&home, &log, false)?;
    projectless_task(&host, &home, &root, "Keep projectless outputs")?;
    let definition = home.join("automations/free/automation.toml");
    fs::write(
        &definition,
        fs::read_to_string(&definition)?.replace("\n[target]", "\nauto_archive = true\n[target]"),
    )?;
    ensure!(run(&host, "free")? == json!({"success": true}));
    let sent = requests(&log)?;
    let thread = sent[2]["params"]["threadId"].as_str().unwrap();
    let cwd = Path::new(sent[0]["params"]["cwd"].as_str().unwrap());
    let output = cwd.join("outputs/result.txt");
    fs::write(&output, "retained output")?;
    let metadata = fs::read(home.join(".codex-global-state.json"))?;
    ensure!(host.finish_automation_run(thread, &completion(&log, thread, "completed")?)?);
    let row = runs(&home)?.remove(0);
    ensure!(row.1 == "ARCHIVED" && row.4.is_none());
    ensure!(fs::read(home.join(".codex-global-state.json"))? == metadata);
    ensure!(fs::read_to_string(output)? == "retained output" && cwd.join("work").is_dir());
    ensure!(
        all_requests(&log)?
            .iter()
            .filter(|request| request["method"] == "thread/archive")
            .count()
            == 1
    );
    Ok(())
}

#[test]
fn heartbeat_checks_active_history_and_preserves_saved_thread_approval_policy() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let thread = json!({"id": "heartbeat-thread", "cwd": dir.path(), "status": {"type": "active", "activeFlags": []},
        "turns": [{"id": "previous", "status": "completed"}]}).to_string();
    let log = dir.path().join("requests.jsonl");
    let config = r#"{"sandbox_mode":"read-only","approval_policy":"untrusted"}"#;
    let host = host_with(
        dir.path(),
        &log,
        false,
        &[("THREAD", &thread), ("CONFIG", config)],
    )?;
    heartbeat_definition(&host, "Check")?;
    let mut params = heartbeat_params();
    params["permissions"] = Value::Null;
    heartbeat_run(&host, params)?;
    let sent = all_requests(&log)?;
    let turn = &sent.last().unwrap()["params"];
    ensure!(turn["approvalPolicy"] == "untrusted" && turn["sandboxPolicy"]["type"] == "readOnly");
    let reads: Vec<_> = sent
        .iter()
        .filter(|r| r["method"] == "thread/read")
        .collect();
    ensure!(
        reads
            .iter()
            .map(|r| r["params"]["includeTurns"].as_bool().unwrap())
            .collect::<Vec<_>>()
            == [false, true, false]
    );
    let racing_home = dir.path().join("race");
    fs::create_dir(&racing_home)?;
    let racing_log = racing_home.join("requests.jsonl");
    let racing = host_with(
        &racing_home,
        &racing_log,
        false,
        &[("THREAD", &thread), ("ACTIVE_RACE", "1")],
    )?;
    heartbeat_definition(&racing, "Check")?;
    ensure!(
        heartbeat_run(&racing, heartbeat_params())
            .unwrap_err()
            .to_string()
            .contains("busy right now")
    );
    ensure!(
        !all_requests(&racing_log)?
            .iter()
            .any(|r| r["method"] == "turn/start")
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn proxy() {
    common::proxy_child();
}

#[cfg(target_os = "macos")]
#[test]
fn heartbeat_row_run_navigates_only_on_success_at_the_original_location() -> Result<()> {
    use app::{GoToRecentChat, Route};
    use common::{Fx, RAIL_SCHEDULED, exchange, recorded_thread, script};
    use ui::screens::{automations::RunAutomationNow, settings::AppServer};
    use ui::{GoBack, GoForward, NewChat};
    for outcome in ["success", "failure", "navigated-away"] {
        let home = tempfile::tempdir()?;
        let project = home.path().join("project");
        fs::create_dir(&project)?;
        let mut thread = recorded_thread(common::GREETING)?;
        thread["id"] = json!("heartbeat-thread");
        thread["cwd"] = json!(project);
        thread["model"] = json!("gpt-6.1-sol");
        thread["reasoningEffort"] = json!("low");
        thread["status"] = json!({"type": "idle"});
        let mut lines = script(None, |line| {
            if line["msg"]["result"]["data"].as_array().is_some_and(|d| {
                d.iter().any(|m| {
                    m.get("model").is_some() && m.get("supportedReasoningEfforts").is_some()
                })
            }) {
                line["msg"]["result"]["data"] = json!([{
                    "id": "gpt-6.1-sol", "model": "gpt-6.1-sol", "displayName": "GPT-6.1 Sol", "description": "",
                    "hidden": false, "defaultReasoningEffort": "low", "supportedReasoningEfforts": [{"reasoningEffort": "low"}, {"reasoningEffort": "high"}], "isDefault": true
                }]);
            }
            if let Some(data) = line["msg"]["result"]["data"].as_array_mut() {
                for entry in data {
                    if entry["id"] == common::GREETING {
                        entry["id"] = json!("heartbeat-thread");
                    }
                }
            }
        })?;
        let mut add = |method: &str, params: Value, result: Value| {
            let mut pair = exchange(method, result);
            pair[0]["msg"]["params"] = params;
            lines.extend(pair);
        };
        add(
            "thread/read",
            json!({"threadId": "heartbeat-thread", "includeTurns": true}),
            json!({"thread": thread}),
        );
        for _ in 0..2 {
            add(
                "thread/read",
                json!({"threadId": "heartbeat-thread", "includeTurns": false}),
                json!({"thread": thread}),
            );
        }
        add(
            "config/read",
            json!({"cwd": project, "includeLayers": false}),
            json!({"config": {"sandbox_mode": "read-only", "approval_policy": "untrusted", "model": "gpt-6.1-sol", "model_reasoning_effort": "high"}}),
        );
        add(
            "configRequirements/read",
            json!({}),
            json!({"requirements": null}),
        );
        add(
            "model/list",
            json!({"includeHidden": true, "cursor": null, "limit": 100}),
            json!({"data": []}),
        );
        add(
            "thread/resume",
            json!({"threadId": "heartbeat-thread", "cwd": project, "path": null}),
            json!({"thread": thread}),
        );
        let turn = json!({"id": "heartbeat-turn", "status": "inProgress", "items": []});
        let mut started = exchange("turn/start", json!({"turn": turn}));
        if outcome == "failure" {
            started[1]["msg"].as_object_mut().unwrap().remove("result");
            started[1]["msg"]["error"] = json!({"code": -32000, "message": "model unavailable"});
        }
        lines.extend(started);
        if outcome != "failure" {
            lines.push(json!({"dir": "recv", "msg": {"method": "turn/started", "params": {"threadId": "heartbeat-thread", "turn": turn}}}));
        }
        thread["turns"].as_array_mut().unwrap().push(turn);
        let mut loaded = exchange("thread/read", json!({"thread": thread}));
        loaded[0]["msg"]["params"] = json!({"threadId": "heartbeat-thread", "includeTurns": true});
        lines.extend(loaded);
        let mut reverted = thread.clone();
        reverted["turns"] = json!([]);
        lines.extend(exchange("thread/revert", json!({"thread": reverted})));
        let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
        let bridge = fx.cx.update(|cx| cx.global::<AppServer>().0.clone());
        let host = LocalHost::new(home.path()).with_bridge(bridge);
        heartbeat_definition(&host, "Check the fixture job")?;
        fx.cx.update(|cx| {
            fx.root
                .update(cx, |root, cx| root.set_host(Arc::new(host), cx))
        });
        fx.action(Box::new(GoToRecentChat(0)))?;
        fx.wait_thread("heartbeat-thread")?;
        let composer = fx.composer();
        ensure!(fx.read(|_, cx| {
            composer.read(cx).run_settings().0["settings"]["reasoning_effort"] == "low"
        }));
        fx.cx
            .update(|cx| composer.update(cx, |c, cx| c.confirm_full_access(cx)));
        fx.click(RAIL_SCHEDULED)?;
        fx.wait("heartbeat row", |fx| {
            fx.read(|r, _| {
                r.automations()
                    .is_some_and(|a| a.as_ref().is_ok_and(|a| a["items"][0]["id"] == "heartbeat"))
            })
        })?;
        fx.hover((150., 190.))?;
        fx.click((302., 190.))?;
        fx.cx.update_window(fx.window, |_, window, cx| {
            for at in [
                gpui::point(gpui::px(250.), gpui::px(228.)),
                gpui::point(gpui::px(640.), gpui::px(780.)),
            ] {
                for event in [
                    gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                        position: at,
                        pressed_button: None,
                        modifiers: gpui::Modifiers::default(),
                    }),
                    gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                        button: gpui::MouseButton::Left,
                        position: at,
                        modifiers: gpui::Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    }),
                    gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                        button: gpui::MouseButton::Left,
                        position: at,
                        modifiers: gpui::Modifiers::default(),
                        click_count: 1,
                    }),
                ] {
                    window.dispatch_event(event, cx);
                }
            }
            window.dispatch_action(
                Box::new(RunAutomationNow {
                    id: "heartbeat".into(),
                }),
                cx,
            );
        })?;
        let starting = |fx: &Fx| {
            fx.read(|r, cx| {
                r.scheduled_sidebar()
                    .is_some_and(|s| s.read(cx).is_starting("heartbeat"))
            })
        };
        ensure!(
            starting(&fx),
            "the heartbeat row must show its pending state"
        );
        if let Ok(out) = std::env::var("CGX_HEARTBEAT_SHOTS") {
            fs::create_dir_all(&out)?;
            fx.cx
                .capture_screenshot(fx.window)?
                .save(format!("{out}/{outcome}-pending.png"))?;
        }
        if outcome == "navigated-away" {
            fx.cx.update_window(fx.window, |_, window, cx| {
                window.dispatch_action(Box::new(NewChat), cx)
            })?;
        }
        fx.wait("heartbeat answer", |fx| !starting(fx))?;
        let sent = fx.sent()?;
        let turns: Vec<_> = sent
            .iter()
            .filter(|r| r["method"] == "turn/start")
            .collect();
        ensure!(
            turns.len() == 1,
            "duplicate Run now must not start another heartbeat"
        );
        ensure!(
            !sent
                .iter()
                .any(|r| r["method"] == "thread/start" || r["method"] == "thread/name/set")
        );
        ensure!(turns[0]["params"]["threadId"] == "heartbeat-thread");
        ensure!(turns[0]["params"]["collaborationMode"]["settings"]["model"] == "gpt-6.1-sol");
        ensure!(
            turns[0]["params"]["approvalPolicy"] == "never"
                && turns[0]["params"]["sandboxPolicy"]["type"] == "dangerFullAccess"
        );
        match outcome {
            "success" => {
                fx.wait_thread("heartbeat-thread")?;
                fx.wait("running heartbeat", |fx| {
                    fx.read(|r, cx| {
                        r.thread().is_some_and(|v| {
                            v.read(cx).turns().iter().any(|t| {
                                t.id == "heartbeat-turn"
                                    && t.status == bridge::types::TurnStatus::InProgress
                            })
                        })
                    })
                })?;
                ensure!(fx.composer_focused()?);
                fx.cx.update(|cx| {
                    composer.update(cx, |c, cx| {
                        c.restore_run_settings(
                            &json!({"settings": {"model": "gpt-6.1-sol", "reasoning_effort": "high"}}),
                            ui::composer::Permission::Ask,
                            cx,
                        )
                    })
                });
                fx.action(Box::new(ui::screens::thread::view::RevertTurn(
                    "heartbeat-turn".into(),
                )))?;
                fx.wait("undo reload", |fx| {
                    fx.read(|r, cx| {
                        r.thread()
                            .is_some_and(|view| view.read(cx).turns().is_empty())
                    })
                })?;
                ensure!(fx.read(|_, cx| {
                    let composer = composer.read(cx);
                    composer.permission() == ui::composer::Permission::Ask
                        && composer.run_settings().0["settings"]["reasoning_effort"] == "high"
                }));
                fx.action(Box::new(GoBack))?;
                ensure!(fx.route() == Route::Scheduled);
                fx.action(Box::new(GoForward))?;
                fx.wait_thread("heartbeat-thread")?;
            }
            "failure" => {
                ensure!(fx.route() == Route::Scheduled);
                ensure!(fx.read(|r, cx| {
                    r.toasts(cx).iter().any(|t| {
                        t.contains("Could not start scheduled task")
                            && t.contains("model unavailable")
                    })
                }));
            }
            _ => ensure!(
                fx.route() == Route::Home,
                "a finished run must not take navigation back"
            ),
        }
        if let Ok(out) = std::env::var("CGX_HEARTBEAT_SHOTS") {
            fx.shot()?.save(format!("{out}/{outcome}-finished.png"))?;
        }
    }
    Ok(())
}

#[test]
fn heartbeat_completion_releases_only_its_matching_turn_without_a_cron_row() -> Result<()> {
    for database in [false, true] {
        let home = tempfile::tempdir()?;
        if database {
            fs::create_dir(home.path().join("sqlite"))?;
            rusqlite::Connection::open(home.path().join("sqlite/codex.db"))?
                .execute_batch(&format!("{AUTOMATIONS};{RUNS};"))?;
        }
        let log = home.path().join("requests.jsonl");
        let thread =
            json!({"id": "heartbeat-thread", "cwd": home.path(), "status": {"type": "idle"}});
        let mut server = Command::new("sh");
        server
            .arg("-c")
            .arg(SERVER)
            .env("LOG", &log)
            .env("THREAD", thread.to_string());
        let (bridge, _events) = Bridge::spawn(server)?;
        let bridge = Arc::new(bridge);
        let host = LocalHost::new(home.path()).with_bridge(bridge.clone());
        heartbeat_definition(&host, "Check")?;
        heartbeat_run(&host, heartbeat_params())?;
        let tracked = bridge.automation_turn("heartbeat-thread").unwrap();
        ensure!(!tracked.auto_archive);
        ensure!(!host.finish_run(
            "heartbeat-thread",
            &json!({"id": "other", "status": "completed"})
        )?);
        ensure!(bridge.automation_turn("heartbeat-thread").is_some());
        ensure!(!host.finish_run(
            "heartbeat-thread",
            &json!({"id": "heartbeat-turn", "status": "completed"})
        )?);
        ensure!(bridge.automation_turn("heartbeat-thread").is_none());
        ensure!(
            !all_requests(&log)?
                .iter()
                .any(|r| r["method"] == "thread/archive")
        );
    }
    Ok(())
}
