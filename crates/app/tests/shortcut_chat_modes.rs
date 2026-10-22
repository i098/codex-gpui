mod common;

use anyhow::{Context as _, Result, ensure};
use common::{CWD, Fx, GREETING, GREETING_PROMPT, exchange, script};
use serde_json::{Value, json};
use std::sync::Arc;

fn menu_hidden(menu: &str, item: &str) -> Result<bool> {
    app::menus()
        .into_iter()
        .find(|entry| entry.name == menu)
        .context("menu")?
        .items
        .into_iter()
        .find_map(|entry| match entry {
            gpui::MenuItem::Action { name, hidden, .. } if name == item => Some(hidden),
            _ => None,
        })
        .context("menu item")
}

#[test]
fn hidden_standalone_menu_allocates_its_own_workspace() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let home = directory.path().join("codex");
    let workspaces = directory.path().join("standalone");
    std::fs::create_dir(&home)?;
    let mut recording = script(Some("sessions/greeting.jsonl"), |_| {})?;
    recording.extend(exchange(
        "config/read",
        json!({"config": {"desktop": {"projectlessWorkspaceRoot": workspaces}}}),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), recording)?;
    let bridge = fx
        .cx
        .update(|cx| cx.global::<ui::screens::settings::AppServer>().0.clone());
    let host = app::host::LocalHost::new(&home).with_bridge(bridge);
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(Arc::new(host), cx));
    ensure!(menu_hidden("File", "New standalone chat")?);
    fx.menu("File", "New standalone chat")?;
    fx.type_text(GREETING_PROMPT)?;
    fx.keys("enter")?;
    fx.wait_thread(GREETING)?;
    let requests = fx.sent()?;
    let request = requests
        .iter()
        .find(|request| request["method"] == "thread/start")
        .context("standalone thread/start")?;
    let cwd = request["params"]["cwd"]
        .as_str()
        .context("standalone cwd")?;
    ensure!(cwd != CWD && std::path::Path::new(cwd).starts_with(&workspaces));
    ensure!(std::path::Path::new(cwd).join("outputs").is_dir());
    ensure!(std::path::Path::new(cwd).join("work").is_dir());
    let state: Value =
        serde_json::from_slice(&std::fs::read(home.join(".codex-global-state.json"))?)?;
    ensure!(
        state["projectless-thread-ids"]
            .as_array()
            .is_some_and(|ids| ids.contains(&json!(GREETING)))
    );
    ensure!(
        request["params"]["developerInstructions"]
            .as_str()
            .is_some_and(|text| text.contains("Projectless Chat"))
    );
    Ok(())
}

#[test]
fn temporary_menu_starts_ephemeral_and_never_enters_recents() -> Result<()> {
    const TEMPORARY: &str = "11111111-1111-4111-8111-111111111111";
    let mut recording = script(None, |_| {})?;
    let extended = script(Some("sessions/greeting.jsonl"), |_| {})?;
    // `script` puts the session between readback and the shared thread settings, so the
    // session lines start where the two scripts differ.
    let start = recording
        .iter()
        .zip(&extended)
        .take_while(|(base, with_session)| base == with_session)
        .count();
    let session = extended[start..start + extended.len() - recording.len()].to_vec();
    for line in session {
        let mut line: Value =
            serde_json::from_str(&serde_json::to_string(&line)?.replace(GREETING, TEMPORARY))?;
        for pointer in ["/msg/result/thread", "/msg/params/thread"] {
            if let Some(thread) = line
                .pointer_mut(pointer)
                .filter(|thread| thread["id"] == TEMPORARY)
            {
                thread["ephemeral"] = json!(true);
            }
        }
        recording.push(line);
    }
    let mut fx = Fx::captured("dark", (1280., 832.), recording)?;
    let recents = fx.read(|root, _| root.recents());
    fx.menu("File", "New Temporary Chat")?;
    fx.type_text(GREETING_PROMPT)?;
    fx.keys("enter")?;
    fx.wait_thread(TEMPORARY)?;
    ensure!(fx.read(|root, _| root.recents()) == recents);
    let requests = fx.sent()?;
    ensure!(requests.iter().any(
        |request| request["method"] == "thread/start" && request["params"]["ephemeral"] == true
    ));
    fx.keys("cmd-n")?;
    ensure!(fx.read(|root, _| !root.recents().contains(&TEMPORARY.to_owned())));
    Ok(())
}

#[test]
fn proxy() {
    common::proxy_child();
}
