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
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"model/list"'*) printf '{"id":%s,"result":{"data":%s}}\n' "$id" "$MODELS" ;;
    *'"config/read"'*) printf '{"id":%s,"result":{"config":%s}}\n' "$id" "$CONFIG" ;;
    *'"configRequirements/read"'*) printf '{"id":%s,"result":{"requirements":%s}}\n' "$id" "$REQUIREMENTS" ;;
    *'"thread/start"'*) [ -z "$FAIL_THREAD" ] || { printf '{"id":%s,"error":{"code":-32000,"message":"no thread"}}\n' "$id"; continue; }; printf '{"id":%s,"result":{"thread":{"id":"thread-%s"}}}\n' "$id" "$id" ;;
    *'"turn/start"'*REFUSE*) printf '{"id":%s,"error":{"code":-32000,"message":"model unavailable"}}\n' "$id" ;;
    *'"turn/start"'*)
      [ -z "$SLOW" ] || sleep 2
      if [ -n "$TURN_SIGNAL" ]; then
        thread=$(printf '%s' "$line" | sed -n 's/.*"threadId":"\([^"]*\)".*/\1/p')
        printf '{"method":"turn/completed","params":{"threadId":"%s","turn":{"id":"turn-%s","status":"completed"}}}\n' "$thread" "$id"
        touch "$TURN_SIGNAL"
        while [ ! -e "$TURN_RELEASE" ]; do sleep 0.01; done
      fi
      printf '{"id":%s,"result":{"turn":{"id":"turn-%s"}}}\n' "$id" "$id" ;;
    *'"thread/archive"'*)
      [ -z "$ARCHIVE_SIGNAL" ] || { touch "$ARCHIVE_SIGNAL"; while [ ! -e "$ARCHIVE_RELEASE" ]; do sleep 0.01; done; }
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
        .env("SLOW", if slow { "1" } else { "" });
    let (bridge, _events) = Bridge::spawn(server)?;
    Ok(LocalHost::new(home).with_bridge(Arc::new(bridge)))
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
    host.write_automation("free", &task("free", "[target]\ntype = \"projectless\"\n"))?;
    for id in ["lost", "free"] {
        ensure!(run(&host, id).is_err());
    }
    ensure!(requests(&log)?.is_empty());
    ensure!(run(&host, "known")? == json!({"success": true}));
    ensure!(requests(&log)?[0]["params"]["cwd"] == json!(root));
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

struct ArchiveRun {
    _directory: tempfile::TempDir,
    home: std::path::PathBuf,
    log: std::path::PathBuf,
    cwd: std::path::PathBuf,
    host: Arc<LocalHost>,
    bridge: Arc<Bridge>,
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
        let (bridge, _) = Bridge::spawn(server)?;
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

fn wait_for_file(path: &Path) -> Result<()> {
    let start = std::time::Instant::now();
    while !path.exists() {
        ensure!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "peer did not reach {path:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    Ok(())
}

#[test]
fn autoarchive_restores_an_inflight_archive_before_sending_the_user_request() -> Result<()> {
    let sync = tempfile::tempdir()?;
    let signal = sync.path().join("archiving");
    let release = sync.path().join("release");
    let run = ArchiveRun::start(
        true,
        false,
        &[
            ("ARCHIVE_SIGNAL", signal.to_str().unwrap()),
            ("ARCHIVE_RELEASE", release.to_str().unwrap()),
        ],
    )?;
    let host = run.host.clone();
    let (thread, turn) = (run.thread.clone(), run.turn.clone());
    let finishing = std::thread::spawn(move || host.finish_run(&thread, &turn));
    wait_for_file(&signal)?;
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
    let signal = directory.path().join("starting");
    let release = directory.path().join("release");
    let log = directory.path().join("requests.jsonl");
    let mut server = Command::new("sh");
    server
        .arg("-c")
        .arg(SERVER)
        .env("LOG", &log)
        .env("TURN_SIGNAL", &signal)
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
    wait_for_file(&signal)?;
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
