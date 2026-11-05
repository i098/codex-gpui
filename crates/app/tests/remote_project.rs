//! `File > New Project...` on a connected SSH host through the real `Root`: the remote folder
//! picker lists the host's folders over `fs/readDirectory`, and `Choose` adds the folder.
mod common;

use anyhow::{Result, ensure};
use bridge::Bridge;
use common::Fx;
use serde_json::{Value, json};
use std::sync::Arc;
use ui::screens::connections::{Connection, ConnectionsState};

const HOST: &str = "ssh:dev";

fn entries(names: &[(&str, bool)]) -> Value {
    let entries: Vec<_> = names
        .iter()
        .map(|(name, dir)| json!({"fileName": name, "isDirectory": dir, "isFile": !dir}))
        .collect();
    json!({"entries": entries})
}

/// The remote app-server: its Codex home is `/home/me/.codex`, so the picker starts in `/home/me`.
fn remote() -> Result<Arc<Bridge>> {
    let exchange = |id: u64, method: &str, params: Value, result: Value| {
        [
            json!({"dir": "send", "msg": {"id": id, "method": method, "params": params}}),
            json!({"dir": "recv", "msg": {"id": id, "result": result}}),
        ]
    };
    let lines: Vec<Value> = [
        exchange(
            1,
            "initialize",
            json!({}),
            json!({"userAgent": "remote", "codexHome": "/home/me/.codex", "platformOs": "linux"}),
        ),
        exchange(
            2,
            "fs/readDirectory",
            json!({"path": "/home/me"}),
            entries(&[(".codex", true), ("notes.txt", false), ("repo", true)]),
        ),
        exchange(
            3,
            "fs/readDirectory",
            json!({"path": "/home/me/repo"}),
            entries(&[("src", true)]),
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    let recording = tempfile::NamedTempFile::new()?;
    let script: String = lines.iter().map(|line| format!("{line}\n")).collect();
    std::fs::write(recording.path(), script)?;
    Ok(Arc::new(Bridge::replay(recording.path())?.0))
}

fn folders(fx: &Fx) -> Option<(String, Vec<String>)> {
    fx.read(|root, cx| {
        let dialog = root.new_project()?.read(cx);
        let browser = dialog.browser().filter(|browser| !browser.loading)?;
        Some((browser.path.clone(), browser.folders.clone()))
    })
}

#[test]
fn new_remote_project_browses_the_host_and_adds_the_folder() -> Result<()> {
    let mut fx = Fx::new()?;
    fx.root.update(&mut fx.cx, |_, cx| {
        app::connections::connect_for_test(HOST, remote().unwrap(), cx);
        cx.default_global::<ConnectionsState>().connections = Some(vec![Connection {
            host_id: HOST.into(),
            display_name: "dev".into(),
            state: "connected".into(),
            ..Default::default()
        }]);
    });
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_action(Box::new(ui::screens::new_project::NewProject), cx)
    })?;
    fx.cx.run_until_parked();
    let dialog = fx
        .read(|root, _| root.new_project().cloned())
        .ok_or_else(|| anyhow::anyhow!("no New Project dialog"))?;
    fx.capture_evidence("CGX_REMOTE_PROJECT_SHOTS", "dialog-local")?;
    dialog.update(&mut fx.cx, |dialog, cx| {
        dialog.select_host(HOST, cx);
        dialog.browse(None, cx);
    });
    fx.wait("home listed", |fx| folders(fx).is_some())?;
    fx.capture_evidence("CGX_REMOTE_PROJECT_SHOTS", "remote-picker")?;
    let listed = folders(&fx);
    ensure!(
        listed == Some(("/home/me".into(), vec!["repo".into(), ".codex".into()])),
        "{listed:?}"
    );
    dialog.update(&mut fx.cx, |dialog, cx| {
        dialog.browse(Some("/home/me/repo".into()), cx)
    });
    fx.wait("repo listed", |fx| {
        folders(fx).is_some_and(|(path, _)| path == "/home/me/repo")
    })?;
    dialog.update(&mut fx.cx, |dialog, cx| dialog.choose(cx));
    ensure!(dialog.read_with(&fx.cx, |dialog, _| dialog.sources() == ["/home/me/repo"]));
    Ok(())
}
