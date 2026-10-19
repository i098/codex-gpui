//! `automation-run-now` over a real stdio JSON-RPC peer: a shell script that logs each request
//! line it receives and answers it, so the test sees exactly what the host sent the app-server.
use anyhow::{Result, ensure};
use app::host::{Host, LocalHost, Request, json_body};
use bridge::Bridge;
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command, sync::Arc};

const SERVER: &str = r#"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$LOG"
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"thread/start"'*) printf '{"id":%s,"result":{"thread":{"id":"thread-%s"}}}\n' "$id" "$id" ;;
    *'"turn/start"'*REFUSE*) printf '{"id":%s,"error":{"code":-32000,"message":"model unavailable"}}\n' "$id" ;;
    *'"turn/start"'*) [ -z "$SLOW" ] || sleep 2; printf '{"id":%s,"result":{}}\n' "$id" ;;
    *) printf '{"id":%s,"result":{}}\n' "$id" ;;
  esac
done
"#;

fn host(home: &Path, log: &Path, slow: bool) -> Result<LocalHost> {
    let mut server = Command::new("sh");
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

fn requests(log: &Path) -> Result<Vec<Value>> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
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
    ensure!(sent[1]["params"] == json!({"threadId": "thread-1", "name": "Review"}));
    let turn = &sent[2]["params"];
    ensure!(turn["threadId"] == "thread-1");
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
    // The original's default environment is a worktree, which needs its git worker.
    let worktree = "version = 1\nid = \"wt\"\nname = \"W\"\nprompt = \"p\"\nstatus = \"ACTIVE\"\ncwds = [\"/tmp\"]\ncreated_at = 1\nupdated_at = 1\n";
    host.write_automation("wt", worktree)?;
    ensure!(
        run(&host, "wt")
            .unwrap_err()
            .to_string()
            .contains("no worktree service")
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
