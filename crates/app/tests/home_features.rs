//! Home features on constructed ChatGPT backend fixtures: a Cloud start, task suggestions,
//! banners and the account access error page. Set `HOME_CAPTURE_DIR` to
//! save each state as a PNG.

mod common;

use anyhow::{Context as _, Result, bail, ensure};
use app::host::{Host, Identity, LocalHost, Request};
use common::{Fx, exchange, script};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use ui::screens::home::{
    DismissSuggestion, HomeBanner, ReloadAccount, TryMultiAgent, UseSuggestion,
};
use ui::screens::hotkey::Mode;

fn wire(status: u16, body: Value) -> Value {
    json!({"value":{"response":{"__response":{"status":status,"body":body.to_string()}}}})
}

/// A signed-in host with two Git projects; it answers the Home backend reads and records
/// every `Http` request as `(method, url, body)`.
struct HomeHost {
    roots: [PathBuf; 2],
    local: LocalHost,
    requests: Mutex<Vec<(String, String, Value)>>,
    _directory: tempfile::TempDir,
}

impl HomeHost {
    fn new() -> Result<Arc<Self>> {
        let directory = tempfile::tempdir()?;
        let roots = ["alpha", "beta"].map(|name| directory.path().join(name));
        for root in &roots {
            std::fs::create_dir(root)?;
            let status = std::process::Command::new("git")
                .args(["-c", "init.defaultBranch=main", "init", "-q"])
                .arg(root)
                .status()?;
            ensure!(status.success(), "git init failed");
        }
        let local = LocalHost::new(directory.path().join("home"));
        Ok(Arc::new(Self {
            roots,
            local,
            requests: Mutex::default(),
            _directory: directory,
        }))
    }

    fn requests(&self, method: &str, url: &str) -> Vec<Value> {
        let requests = self.requests.lock().unwrap();
        requests
            .iter()
            .filter(|(m, u, _)| m == method && u.starts_with(url))
            .map(|(_, _, body)| body.clone())
            .collect()
    }
}

impl Host for HomeHost {
    fn identity(&self) -> Option<Identity> {
        Some(Identity {
            account_id: "fixture-account".into(),
            user_id: "fixture-user".into(),
            device_id: "fixture-device".into(),
        })
    }

    fn projects(&self) -> Result<Vec<ui::screens::sidebar::Project>> {
        Ok(self
            .roots
            .iter()
            .map(|root| {
                let name = root.file_name().unwrap().to_string_lossy().into_owned();
                ui::screens::sidebar::Project {
                    id: name.clone().into(),
                    label: name.into(),
                    new_chat_label: "New chat".into(),
                    root: Some(root.to_string_lossy().into_owned().into()),
                    roots: vec![root.to_string_lossy().into_owned().into()],
                    worktree: false,
                    missing: false,
                    expanded: true,
                    pinned: false,
                }
            })
            .collect())
    }

    fn fetch(&self, request: &Request) -> Result<Value> {
        let Request::Http(request) = request else {
            return self.local.fetch(request);
        };
        let url = request["url"].as_str().context("fixture URL")?.to_owned();
        let method = request["method"].as_str().unwrap_or("GET").to_owned();
        let body = request["body"]
            .as_str()
            .and_then(|body| serde_json::from_str(body).ok())
            .unwrap_or(Value::Null);
        self.requests
            .lock()
            .unwrap()
            .push((method.clone(), url.clone(), body));
        Ok(match (method.as_str(), url.as_str()) {
            ("GET", "/wham/environments") => wire(
                200,
                json!([{"id":"env_web","label":"web"},{"id":"env_api","label":"api"}]),
            ),
            ("POST", "/wham/tasks") => wire(200, json!({"task":{"id":"task_e_fixture"}})),
            // The new task's page: a finished turn of the posted prompt, so the
            // poller sends one snapshot and stops (a running turn would stream).
            ("GET", "/wham/tasks/task_e_fixture") => wire(
                200,
                json!({
                    "task":{"id":"task_e_fixture","title":"Fix the login bug","created_at":1792080000},
                    "current_user_turn":{"id":"user-1","created_at":1792080000,"input_items":[
                        {"type":"message","role":"user","content":[{"content_type":"text","text":"Fix the login bug"}]}
                    ]},
                    "current_assistant_turn":{"id":"assistant-1","previous_turn_id":"user-1",
                        "turn_status":"completed","created_at":1792080001,"output_items":[
                            {"type":"message","role":"assistant","content":[{"content_type":"text","text":"Fixed."}]}
                        ],"error":null,"thread_events":{"events":[]}},
                    "current_diff_task_turn":null
                }),
            ),
            ("GET", "/wham/tasks/task_e_fixture/turns") => wire(200, json!({"turn_mapping":{}})),
            ("GET", url) if url.starts_with("/composer/items?") => wire(
                200,
                json!({"items":[
                    {"id":"s1","producer":"fixture","item_type":"task_suggestion",
                     "title":"Add tests for the parser","description":"Cover the edge cases",
                     "destination_metadata":{"type":"prompt","prompt":"Add parser tests."}},
                    {"id":"s2","producer":"fixture","item_type":"task_suggestion",
                     "title":"Fix the flaky build","action_metadata":{"dismissal_id":"d2"},
                     "destination_metadata":{"type":"prompt","prompt":"Fix the build."}},
                ]}),
            ),
            ("POST", _) => wire(200, json!({})),
            _ => wire(404, json!({})),
        })
    }
}

fn boot(lines: Vec<Value>) -> Result<(Fx, Arc<HomeHost>)> {
    let host = HomeHost::new()?;
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    Ok((fx, host))
}

fn capture(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("HOME_CAPTURE_DIR", name)
}

fn select_project(fx: &mut Fx, host: &HomeHost, index: usize) -> Result<()> {
    let root = host.roots[index].to_string_lossy().into_owned();
    fx.action(Box::new(ui::screens::sidebar::SelectProject(root)))
}

#[test]
fn proxy() {
    common::proxy_child()
}

#[test]
fn cloud_start_posts_a_task_in_the_chosen_environment() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let (mut fx, host) = boot(script(None, |_| {})?)?;
    select_project(&mut fx, &host, 0)?;
    fx.wait("cloud environments", |fx| {
        fx.read(|root, cx| root.home().read(cx).location.cloud_environments.len() == 2)
    })?;
    let home = fx.read(|root, _| root.home().clone());
    home.update(&mut fx.cx, |home, cx| {
        home.location.mode = Mode::Cloud;
        home.location.cloud_environment = Some("env_api".into());
        cx.notify();
    });
    capture(&mut fx, "cloud-start")?;
    fx.focus_composer()?;
    fx.type_text("Fix the login bug")?;
    fx.keys("enter")?;
    let opened = fx.wait("cloud task page", |fx| {
        fx.read(|root, _| root.remote_page().is_some())
    });
    if let Err(error) = opened {
        let toasts = fx.read(|root, cx| root.toasts(cx));
        let composer = fx.composer_text()?;
        let posts = host.requests("POST", "/wham/tasks");
        bail!("{error:#}; posts {posts:?}; toasts {toasts:?}; composer {composer:?}");
    }
    let posts = host.requests("POST", "/wham/tasks");
    ensure!(posts.len() == 1, "expected one task POST: {posts:?}");
    let post = posts[0].to_string();
    for part in ["env_api", "Fix the login bug", "main"] {
        ensure!(post.contains(part), "task POST lacks {part}: {post}");
    }
    Ok(())
}

#[test]
fn task_suggestions_fill_the_composer_and_dismiss() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let (mut fx, host) = boot(script(None, |_| {})?)?;
    fx.wait("suggestions", |fx| {
        fx.read(|root, cx| root.home().read(cx).suggestions.len() == 2)
    })?;
    capture(&mut fx, "suggestions")?;
    fx.action(Box::new(UseSuggestion("s1".into())))?;
    ensure!(
        fx.composer_text()? == "Add parser tests.",
        "prompt not filled"
    );
    fx.action(Box::new(DismissSuggestion("s2".into())))?;
    fx.wait("dismiss reports", |_| {
        !host
            .requests("POST", "/task_suggestions/dismiss")
            .is_empty()
    })?;
    let ids: Vec<_> = fx.read(|root, cx| {
        let home = root.home().read(cx);
        home.suggestions.iter().map(|s| s.id.clone()).collect()
    });
    ensure!(ids == ["s1"], "dismissed card stayed: {ids:?}");
    let interactions = host.requests("POST", "/composer/items/interactions");
    let kinds: Vec<_> = interactions
        .iter()
        .map(|body| body["interaction_type"].clone())
        .collect();
    ensure!(kinds == [json!("clicked"), json!("dismissed")], "{kinds:?}");
    ensure!(
        host.requests("POST", "/task_suggestions/dismiss")[0]["task_suggestion_id"] == "d2",
        "wrong dismissal id"
    );
    Ok(())
}

#[test]
fn subagents_banner_try_now_enables_multi_agent() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "experimentalFeature/list",
        json!({"data":[{"name":"multi_agent","enabled":false}],"nextCursor":null}),
    ));
    // Startup reads `account/rateLimits/read`, then `experimentalFeature/list`.
    let (mut fx, _host) = boot(lines)?;
    fx.wait("subagents banner", |fx| {
        fx.read(|root, cx| root.home().read(cx).banner == Some(HomeBanner::Subagents))
    })?;
    capture(&mut fx, "subagents-banner")?;
    fx.action(Box::new(TryMultiAgent))?;
    ensure!(
        fx.composer_text()? == "Spawn a subagent to explore this repo.",
        "starter prompt missing"
    );
    fx.wait("multi_agent write", |fx| {
        fx.sent().is_ok_and(|sent| {
            sent.iter().any(|message| {
                message["method"] == "config/batchWrite"
                    && message["params"]["edits"][0]["keyPath"] == "features.multi_agent"
            })
        })
    })?;
    ensure!(
        fx.read(|root, cx| root.home().read(cx).banner.is_none()),
        "banner stayed after Try now"
    );
    Ok(())
}

#[test]
fn account_read_failure_shows_the_access_error_page() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut lines = script(None, |_| {})?;
    for line in &mut lines {
        if line["dir"] == "recv" && line["msg"]["result"].get("requiresOpenaiAuth").is_some() {
            let id = line["msg"]["id"].clone();
            line["msg"] = json!({"id": id, "error": {"code": -32603, "message": "fixture"}});
        }
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.wait("account error page", |fx| {
        fx.read(|root, cx| root.home().read(cx).account_error)
    })?;
    capture(&mut fx, "account-error")?;
    let reads = |fx: &mut Fx| {
        fx.sent().map(|sent| {
            sent.iter()
                .filter(|message| message["method"] == "account/read")
                .count()
        })
    };
    let before = reads(&mut fx)?;
    fx.action(Box::new(ReloadAccount))?;
    fx.wait("reload reads the account", |fx| {
        reads(fx).is_ok_and(|count| count > before)
    })?;
    Ok(())
}
