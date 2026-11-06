//! The `@` list's `Plugins` section lists the installed, enabled plugins of `plugin/list`;
//! picking one inserts a chip and sends `[@title](plugin://id)`, as the original does.

mod common;

use anyhow::{Context as _, Result, ensure};
use common::{Fx, exchange, script};
use gpui::Focusable as _;
use serde_json::json;
use std::io::{BufRead as _, Write as _};
use std::sync::{Arc, Mutex, mpsc};
use std::{cell::RefCell, rc::Rc};

/// The app-server of [`Fx::captured`].
#[test]
fn proxy() {
    common::proxy_child();
}

fn plugin(id: &str, title: &str, installed: bool) -> serde_json::Value {
    json!({
        "id": id, "name": id, "installed": installed, "enabled": installed,
        "interface": { "displayName": title, "shortDescription": format!("{title} plugin") },
    })
}

#[test]
fn mention_lists_and_inserts_plugins() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let list = json!({ "marketplaces": [{ "name": "openai-curated", "plugins": [
        plugin("github@openai-curated", "GitHub", true),
        plugin("gmail@openai-curated", "Gmail", false),
    ] }] });
    let mut lines = script(None, |_| {})?;
    // The readback's own `plugin/list` would answer first (the replay falls back to recording
    // order), so this list replaces it.
    lines.retain(|l| l["msg"]["method"] != "plugin/list");
    for _ in 0..2 {
        lines.extend(exchange("plugin/list", list.clone()));
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.focus(&composer.focus_handle(cx), cx)
    })?;
    let sent = Rc::new(RefCell::new(None));
    let slot = sent.clone();
    composer.update(&mut fx.cx, |c, _| {
        c.on_submit(move |s, _, _| *slot.borrow_mut() = Some(s.text.clone()))
    });
    fx.type_text("@")?;
    fx.wait("plugin rows", |fx| {
        fx.read(|root, cx| root.composer(cx).read(cx).mention_state())
            .is_some_and(|s| {
                s.iter()
                    .flat_map(|(_, rows)| rows)
                    .any(|r| r.0.starts_with("GitHub"))
            })
    })?;
    let sections = fx
        .read(|root, cx| root.composer(cx).read(cx).mention_state())
        .context("mention list")?;
    let plugins = &sections
        .iter()
        .find(|(h, _)| h.as_deref() == Some("Plugins"))
        .context("Plugins section")?
        .1;
    ensure!(
        plugins
            .iter()
            .map(|r| r.0.as_str())
            .eq(["GitHub | GitHub plugin"]),
        "Plugins must list only the installed plugin: {plugins:?}"
    );
    fx.capture_evidence("CGX_PLUGIN_MENTION_EVIDENCE", "plugins-listed")?;
    fx.type_text("git")?;
    fx.keys("enter")?;
    let chips = fx.read(|root, cx| root.composer(cx).read(cx).chips());
    ensure!(
        chips.first().is_some_and(|c| c.0 == "GitHub"),
        "no GitHub chip: {chips:?}"
    );
    fx.keys("enter")?;
    let text = sent.borrow().clone().context("the prompt was not sent")?;
    ensure!(
        text.contains("[@GitHub](plugin://github@openai-curated)"),
        "sent {text:?}"
    );
    Ok(())
}

#[test]
fn mention_apps_follow_the_bound_thread() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.retain(|line| {
        !matches!(
            line["msg"]["method"].as_str(),
            Some("app/installed" | "app/read")
        )
    });
    for (id, name) in [
        ("linear-first", "First Linear"),
        ("linear-second", "Second Linear"),
        ("linear-refreshed", "Refreshed Linear"),
    ] {
        lines.extend(exchange("app/installed", json!({ "apps": [
            { "id": id, "runtimeName": name, "callable": true, "enabled": true },
            { "id": "disabled", "runtimeName": "Disabled", "callable": true, "enabled": false },
            { "id": "unavailable", "runtimeName": "Unavailable", "callable": false, "enabled": true }
        ] })));
        if id == "linear-refreshed" {
            lines.extend(common::refusal("app/read", "metadata unavailable"));
        } else {
            lines.extend(exchange(
                "app/read",
                json!({ "apps": [
                    { "id": id, "name": name, "description": "Track issues" }
                ] }),
            ));
        }
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let composer = fx.composer();
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.focus(&composer.focus_handle(cx), cx)
    })?;
    for (thread, title) in [
        ("first-thread", "First Linear"),
        ("second-thread", "Second Linear"),
    ] {
        composer.update(&mut fx.cx, |composer, cx| {
            composer.bind_thread(thread);
            composer.set_text("", cx);
        });
        fx.type_text("@")?;
        fx.wait("thread apps", |fx| {
            fx.read(|root, cx| root.composer(cx).read(cx).mention_state())
                .is_some_and(|sections| {
                    sections.iter().any(|(heading, rows)| {
                        *heading == Some("Apps") && rows.iter().any(|row| row.0.starts_with(title))
                    })
                })
        })?;
        let sections = fx
            .read(|root, cx| root.composer(cx).read(cx).mention_state())
            .context("mention sections")?;
        let rows = &sections
            .iter()
            .find(|(heading, _)| *heading == Some("Apps"))
            .context("Apps section")?
            .1;
        ensure!(
            rows.iter()
                .map(|row| row.0.as_str())
                .eq([format!("{title} | Track issues")]),
            "wrong thread apps: {rows:?}"
        );
    }
    fx.type_text("second")?;
    fx.capture_evidence("CGX_APP_MENTION_EVIDENCE", "apps-listed")?;
    fx.keys("enter")?;
    ensure!(
        fx.read(|root, cx| root.composer(cx).read(cx).prompt())
            .contains("[$Second Linear](app://linear-second)"),
        "app selection lost its app link"
    );
    fx.capture_evidence("CGX_APP_MENTION_EVIDENCE", "app-chip")?;
    composer.update(&mut fx.cx, |composer, cx| composer.set_text("", cx));
    let recording: serde_json::Value =
        serde_json::from_str(include_str!("../../../refs/interactions/s018.json"))?;
    let trigger = &recording["trigger"];
    fx.click((
        (trigger["x"].as_f64().unwrap() + trigger["w"].as_f64().unwrap() / 2.) as f32,
        (trigger["y"].as_f64().unwrap() + trigger["h"].as_f64().unwrap() / 2.) as f32,
    ))?;
    fx.wait("refreshed add-menu apps", |fx| {
        fx.read(|root, cx| root.composer(cx).read(cx).mention_state())
            .is_some_and(|sections| {
                sections
                    .iter()
                    .flat_map(|(_, rows)| rows)
                    .any(|row| row.0.starts_with("Refreshed Linear"))
            })
    })?;
    let requests = fx.sent()?;
    let installed: Vec<_> = requests
        .iter()
        .filter(|request| request["method"] == "app/installed")
        .map(|request| request["params"].clone())
        .collect();
    ensure!(
        installed
            == [
                json!({ "threadId": "first-thread", "forceRefresh": true }),
                json!({ "threadId": "second-thread", "forceRefresh": true }),
                json!({ "threadId": "second-thread", "forceRefresh": true }),
            ],
        "installed apps did not follow the bound thread: {installed:?}"
    );
    Ok(())
}

#[test]
fn late_app_reply_cannot_replace_the_current_threads_apps() -> Result<()> {
    let (client_read, server_write) = std::io::pipe()?;
    let (server_read, client_write) = std::io::pipe()?;
    let writer = Arc::new(Mutex::new(server_write));
    let server_writer = writer.clone();
    let (held, pending) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for line in std::io::BufReader::new(server_read).lines() {
            let request: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
            let method = request["method"].as_str().unwrap();
            if method == "app/installed" && request["params"]["threadId"] == "first-thread" {
                held.send(request["id"].clone()).unwrap();
                continue;
            }
            let result = match method {
                "app/installed" => json!({ "apps": [{
                    "id": "second-thread", "callable": true, "enabled": true
                }] }),
                "app/read" => {
                    let id = &request["params"]["appIds"][0];
                    json!({ "apps": [{ "id": id, "name": id }] })
                }
                _ => json!({}),
            };
            let mut out = server_writer.lock().unwrap();
            writeln!(out, "{}", json!({ "id": request["id"], "result": result })).unwrap();
            if method == "app/read" && request["params"]["appIds"][0] == "first-thread" {
                writeln!(
                    out,
                    "{}",
                    json!({ "method": "test/lateRead", "params": {} })
                )
                .unwrap();
            }
            out.flush().unwrap();
        }
    });
    let (bridge, events) = bridge::Bridge::from_io(client_read, client_write, None);
    let bridge = Arc::new(bridge);
    let mut fx = Fx::open("dark", (1280., 832.), script(None, |_| {})?)?;
    let composer = fx.composer();
    composer.update(&mut fx.cx, |composer, _| {
        composer.bind_connection(bridge.clone());
        composer.bind_thread("first-thread");
    });
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.focus(&composer.focus_handle(cx), cx)
    })?;
    fx.type_text("@")?;
    let mut first_id = None;
    fx.wait("held first-thread app request", |_| {
        if let Ok(id) = pending.try_recv() {
            first_id = Some(id);
        }
        first_id.is_some()
    })?;
    composer.update(&mut fx.cx, |composer, cx| {
        composer.bind_thread("second-thread");
        composer.set_text("", cx);
    });
    fx.type_text("@")?;
    fx.wait("second-thread apps", |fx| {
        fx.read(|root, cx| root.composer(cx).read(cx).mention_state())
            .is_some_and(|sections| {
                sections
                    .iter()
                    .flat_map(|(_, rows)| rows)
                    .any(|row| row.0 == "second-thread")
            })
    })?;
    {
        let mut out = writer.lock().unwrap();
        writeln!(
            out,
            "{}",
            json!({ "id": first_id, "result": { "apps": [{
            "id": "first-thread", "callable": true, "enabled": true
        }] } })
        )?;
        out.flush()?;
    }
    fx.wait("late app response delivered", |_| {
        matches!(events.try_recv(), Ok(bridge::Event::Notification { method, .. })
            if method == "test/lateRead")
    })?;
    fx.cx.run_until_parked();
    let sections = fx
        .read(|root, cx| root.composer(cx).read(cx).mention_state())
        .context("current mention list")?;
    let apps = &sections
        .iter()
        .find(|(heading, _)| *heading == Some("Apps"))
        .context("current apps")?
        .1;
    ensure!(
        apps.iter().map(|row| row.0.as_str()).eq(["second-thread"]),
        "late reply replaced the current apps: {apps:?}"
    );
    drop(composer);
    drop(fx);
    drop(bridge);
    server.join().expect("app response server");
    Ok(())
}
