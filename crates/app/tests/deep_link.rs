//! Deep-link navigation (SH-151..154): a `codex://` link navigates the primary window
//! like the original's deep-link flush, and a prompt prefill focuses the composer.

mod common;

use anyhow::Result;
use common::{Fx, GREETING, SUBTRACT};

fn open(fx: &mut Fx, url: &str) -> Result<()> {
    let link = app::deep_link::parse(url).expect("parses");
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    fx.cx.run_until_parked();
    Ok(())
}

#[test]
fn new_thread_link_prefills_and_focuses_the_composer() -> Result<()> {
    let mut fx = Fx::new()?;
    open(&mut fx, "codex://threads/new?prompt=Fix%20the%20greeting")?;
    assert_eq!(fx.route(), app::Route::Home);
    assert_eq!(fx.composer_text()?, "Fix the greeting");
    assert!(fx.composer_focused()?);
    Ok(())
}

#[test]
fn thread_link_opens_the_conversation() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.cx
        .update(|cx| app::deep_link::open(&format!("codex://threads/{SUBTRACT}"), cx));
    fx.wait_thread(SUBTRACT)?;
    #[cfg(target_os = "macos")]
    {
        fx.parity("thread")?;
        fx.capture_evidence("CGX_DEEP_LINK_EVIDENCE", "thread-after")?;
    }
    Ok(())
}

#[test]
fn settings_automations_and_launch_links_route() -> Result<()> {
    let mut fx = Fx::new()?;
    open(&mut fx, "codex://settings/connections")?;
    fx.wait("settings", |fx| fx.route() == app::Route::Settings)?;

    let mut fx = Fx::new()?;
    open(&mut fx, "codex://automations")?;
    fx.wait("scheduled", |fx| fx.route() == app::Route::Scheduled)?;

    // `launch` navigates nowhere.
    let mut fx = Fx::new()?;
    let route = fx.route();
    open(&mut fx, "codex://launch")?;
    assert_eq!(fx.route(), route);
    Ok(())
}

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn project_link_starts_the_prefilled_chat_in_the_selected_folder() -> Result<()> {
    let project = tempfile::tempdir()?;
    let folder = project.path().canonicalize()?;
    let mut fx = Fx::captured(
        "dark",
        (1280., 832.),
        common::script(Some("sessions/greeting.jsonl"), |_| {})?,
    )?;
    let mut url = url::Url::parse("codex://threads/new")?;
    url.query_pairs_mut()
        .append_pair("path", folder.to_str().unwrap())
        .append_pair("prompt", common::GREETING_PROMPT);
    open(&mut fx, url.as_str())?;
    fx.wait("linked project draft", |fx| {
        fx.composer_text()
            .is_ok_and(|text| text == common::GREETING_PROMPT)
    })?;
    assert!(fx.composer_focused()?);
    fx.keys("enter")?;
    fx.wait_thread(GREETING)?;
    let requests = fx.sent()?;
    let start = requests
        .iter()
        .find(|request| request["method"] == "thread/start")
        .unwrap();
    assert_eq!(start["params"]["cwd"], folder.to_str().unwrap());
    Ok(())
}

#[test]
fn shared_access_stays_on_thread_requests_and_private_reopening_clears_it() -> Result<()> {
    let mut fx = Fx::captured("dark", (1280., 832.), common::script(None, |_| {})?)?;
    open(
        &mut fx,
        &format!("codex://threads/{GREETING}?threadAccess=shared"),
    )?;
    fx.wait_thread(GREETING)?;
    let bridge = fx
        .cx
        .update(|cx| cx.global::<ui::screens::settings::AppServer>().0.clone());
    let params = serde_json::json!({"threadId": GREETING, "includeTurns": false});
    pollster::block_on(bridge.request::<_, serde_json::Value>("thread/read", params.clone()))?;
    let requests = fx.sent()?;
    assert!(
        requests
            .iter()
            .filter(|request| request["method"] == "thread/read")
            .all(|request| request["params"]["threadAccess"] == "shared")
    );
    open(&mut fx, &format!("codex://threads/{GREETING}"))?;
    pollster::block_on(bridge.request::<_, serde_json::Value>("thread/read", params))?;
    let requests = fx.sent()?;
    let read = requests
        .iter()
        .rfind(|request| request["method"] == "thread/read")
        .unwrap();
    assert!(read["params"].get("threadAccess").is_none());
    Ok(())
}

#[test]
fn task_links_keep_same_id_cloud_and_local_editors_separate() -> Result<()> {
    use app::host::{Host, LocalHost, Request};
    use serde_json::{Value, json};
    use std::sync::Arc;

    struct Tasks(LocalHost);
    impl Host for Tasks {
        fn fetch(&self, request: &Request) -> Result<Value> {
            if let Request::Fetch { url, .. } = request
                && matches!(
                    url.as_str(),
                    "vscode://codex/list-automations" | "vscode://codex/cloud-list-automations"
                )
            {
                let source = if url.ends_with("/cloud-list-automations") {
                    "cloud"
                } else {
                    "local"
                };
                let body = json!({"items": [{
                    "id": "same-id", "name": source, "prompt": "Review changes",
                    "rrule": "FREQ=DAILY;BYHOUR=9", "status": "ACTIVE", "cwds": []
                }]});
                return Ok(json!({"status": 200, "body": body.to_string()}));
            }
            self.0.fetch(request)
        }
    }
    let profile = tempfile::tempdir()?;
    let mut fx = Fx::new()?;
    fx.cx.update(|cx| {
        fx.root.update(cx, |root, cx| {
            root.set_host(Arc::new(Tasks(LocalHost::new(profile.path()))), cx);
        })
    });
    for source in ["cloud", "local"] {
        open(
            &mut fx,
            &format!("codex://automations/same-id?automationSource={source}&edit=true"),
        )?;
        fx.wait("source-owned task editor", |fx| {
            fx.read(|root, _| {
                root.automation_editor().is_some()
                    && root
                        .automations()
                        .and_then(|list| list.as_ref().ok())
                        .is_some_and(|list| list["items"][0]["name"] == source)
            })
        })?;
        assert_eq!(fx.route(), app::Route::Automation("same-id".into()));
    }
    open(
        &mut fx,
        "codex://automations/same-id?automationSource=cloud&edit=true",
    )?;
    fx.wait("cloud task before local navigation", |fx| {
        fx.read(|root, _| {
            root.automations()
                .and_then(|list| list.as_ref().ok())
                .is_some_and(|list| list["items"][0]["name"] == "cloud")
        })
    })?;
    fx.action(Box::new(ui::OpenScheduled))?;
    fx.wait("local scheduled page", |fx| {
        fx.read(|root, _| {
            root.automations()
                .and_then(|list| list.as_ref().ok())
                .is_some_and(|list| list["items"][0]["name"] == "local")
        })
    })?;
    Ok(())
}
