//! Settings ▸ Computer history through the real `Root`, with a stateful fake of the native
//! recorder (`ComputerUseIPCSkysight*Request`): the off state, the consent dialog, turning it on
//! with all apps allowed, the on state with History read from the memory-summary files, and
//! the permissions dialog.
#[path = "common/mod.rs"]
mod common;

use anyhow::{Context as _, Result, ensure};
use app::host::{Host, Request};
use common::{Fx, exchange};
use serde_json::{Value, json};
use std::sync::Mutex;

#[test]
fn proxy() {
    common::proxy_child();
}

#[derive(Default)]
struct Recorder {
    state: Mutex<String>,
    settings: Mutex<Value>,
    log: Mutex<Vec<String>>,
}

impl Host for Recorder {
    fn fetch(&self, _: &Request) -> Result<Value> {
        anyhow::bail!("no network in this test")
    }

    fn skysight_available(&self) -> bool {
        true
    }

    fn skysight(&self, request_type: &str, payload: &Value, _: u32) -> Result<Value> {
        self.log.lock().unwrap().push(request_type.to_owned());
        let mut state = self.state.lock().unwrap();
        let next = match request_type {
            "ComputerUseIPCSkysightStartRequest" | "ComputerUseIPCSkysightResumeRequest" => {
                "running"
            }
            "ComputerUseIPCSkysightPauseRequest" => "paused",
            "ComputerUseIPCSkysightStopRequest" => "stopped",
            "ComputerUseIPCSkysightGetSettingsRequest" => {
                return Ok(self.settings.lock().unwrap().clone());
            }
            "ComputerUseIPCSkysightUpdateSettingsRequest" => {
                *self.settings.lock().unwrap() = payload["settings"].clone();
                return Ok(payload["settings"].clone());
            }
            _ => state.as_str(),
        }
        .to_owned();
        *state = next;
        Ok(json!({
            "state": *state, "eventStreamRootPath": null, "currentSegmentEventsPath": null,
            "currentSegmentMetadataPath": null, "suppressedEventsPath": null,
            "startedAtMs": null, "endedAtMs": null,
        }))
    }
}

fn click(fx: &mut Fx, selector: &str) -> Result<()> {
    let bounds = fx
        .cx
        .debug_bounds(fx.window, selector)?
        .with_context(|| format!("{selector} is not on screen"))?;
    let center = bounds.center();
    fx.click((f32::from(center.x), f32::from(center.y)))
}

fn visible(fx: &mut Fx, selector: &str) -> bool {
    fx.cx
        .debug_bounds(fx.window, selector)
        .is_ok_and(|b| b.is_some())
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("CGX_COMPUTER_HISTORY_EVIDENCE", name)
}

/// One summary file started `ago` before now.
fn summary(dir: &std::path::Path, ago: chrono::Duration, level: &str, body: &str) -> Result<()> {
    let start = chrono::Utc::now() - ago;
    let name = format!(
        "{}-z-{level}-memory-summary.md",
        start.format("%Y-%m-%dT%H-%M-%S")
    );
    std::fs::write(dir.join(name), format!("---\n{body}---\n"))?;
    Ok(())
}

#[test]
fn turn_on_and_read_history() -> Result<()> {
    let home = tempfile::tempdir()?;
    let resources = home.path().join("memories/extensions/skysight/resources");
    std::fs::create_dir_all(&resources)?;
    summary(
        &resources,
        chrono::Duration::minutes(40),
        "10min",
        "title: Q3 budget follow-up\ndescription: Updated the budget sheet and replied to Sarah in Slack.\napplications: [com.google.Chrome, com.tinyspeck.slackmacgap]\nsuggestion:\n  type: automation\n  name: Weekly budget check\n  description: Every Friday, compare the budget sheet with the meeting notes\n",
    )?;
    summary(
        &resources,
        chrono::Duration::hours(30),
        "6h",
        "title: Launch presentation\ndescription: Revised the launch slides and checked campaign numbers.\napplications:\n  - com.apple.Keynote\n",
    )?;
    // SAFETY: the only other test in this binary is the proxy child, which reads no CODEX_HOME.
    unsafe { std::env::set_var("CODEX_HOME", home.path()) };

    let mut lines = common::script(None, |_| {})?;
    let config = json!({
        "config": { "features": { "memories": true, "chronicle": false }, "memories": {} },
        "layers": []
    });
    for _ in 0..16 {
        lines.extend(exchange("config/read", config.clone()));
        lines.extend(exchange(
            "experimentalFeature/list",
            json!({ "data": [{ "name": "memories", "enabled": true }], "nextCursor": null }),
        ));
        lines.extend(exchange(
            "configRequirements/read",
            json!({ "requirements": null }),
        ));
    }
    for _ in 0..4 {
        lines.extend(exchange("config/value/write", json!({ "status": "ok" })));
        lines.extend(exchange("plugin/list", json!({ "marketplaces": [] })));
    }
    let mut fx = Fx::scripted(lines)?;
    let recorder = std::sync::Arc::new(Recorder {
        state: Mutex::new("stopped".into()),
        settings: Mutex::new(json!({ "observation": {
            "defaultApplicationBehavior": "observe", "defaultURLBehavior": "observe",
            "allowlist": [],
            "blocklist": [{ "scope": "url", "bundleID": null, "urlDomain": "bank.example.com" }]
        }})),
        log: Mutex::default(),
    });
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(recorder.clone(), cx));
    let link = app::deep_link::parse("codex://settings/chronicle").context("link")?;
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| root.open_deep_link(&link, window, cx));
    })?;
    fx.wait("off state", |fx| visible(fx, "computer-history-turn-on"))?;
    fx.advance(std::time::Duration::from_millis(500))?;
    shot(&mut fx, "off")?;

    click(&mut fx, "computer-history-turn-on")?;
    fx.wait("consent", |fx| visible(fx, "computer-history-consent"))?;
    shot(&mut fx, "consent")?;

    click(&mut fx, "computer-history-allow-all")?;
    fx.wait("on state", |fx| visible(fx, "computer-history-ask"))?;
    ensure!(
        recorder
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|r| r == "ComputerUseIPCSkysightStartRequest"),
        "the recorder was not started"
    );
    fx.advance(std::time::Duration::from_millis(300))?;
    shot(&mut fx, "on")?;

    click(&mut fx, "computer-history-permissions")?;
    fx.wait("permissions", |fx| {
        visible(fx, "computer-history-permissions-dialog")
    })?;
    shot(&mut fx, "permissions")?;
    click(&mut fx, "computer-history-permissions-save")?;
    fx.wait("permissions saved", |fx| {
        !visible(fx, "computer-history-permissions-dialog")
    })?;
    ensure!(
        recorder
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|r| r == "ComputerUseIPCSkysightUpdateSettingsRequest")
    );
    Ok(())
}
