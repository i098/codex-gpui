mod common;

use anyhow::{Context as _, Result, ensure};
use common::{CWD, Fx, GREETING, GREETING_PROMPT, exchange, script};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use ui::screens::settings::keyboard_shortcuts::{Change, KeyboardState, NumberTarget};

fn menu_hidden(fx: &Fx, menu: &str, item: &str) -> Result<bool> {
    fx.read(|_, cx| cx.get_menus())
        .context("installed menus")?
        .into_iter()
        .find(|entry| entry.name == menu)
        .context("menu")?
        .items
        .into_iter()
        .find_map(|entry| match entry {
            gpui::OwnedMenuItem::Action { name, hidden, .. } if name == item => Some(hidden),
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
    ensure!(menu_hidden(&fx, "File", "New standalone chat")?);
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
    let original_length = recording.len();
    let extended = script(Some("sessions/greeting.jsonl"), |_| {})?;
    for line in extended.into_iter().skip(original_length) {
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
fn settings_sidebar_visibility_returns_after_back_navigation() -> Result<()> {
    let mut fx = Fx::new()?;
    let sidebar_open = |fx: &Fx| fx.read(|root, cx| root.shell().read(cx).sidebar_open);
    let was_open = sidebar_open(&fx);
    ensure!(!menu_hidden(&fx, "View", "Toggle Sidebar")?);
    fx.menu("Codex", "Settings…")?;
    fx.advance(Duration::ZERO)?;
    ensure!(fx.route() == app::Route::Settings);
    ensure!(menu_hidden(&fx, "View", "Toggle Sidebar")?);
    fx.cx.update(|cx| {
        let dispatch = cx.global::<KeyboardState>().dispatch.clone().unwrap();
        dispatch(Change::NumberTarget(NumberTarget::Tabs), cx);
    });
    fx.advance(Duration::ZERO)?;
    ensure!(menu_hidden(&fx, "View", "Toggle Sidebar")?);
    fx.menu("View", "Toggle Sidebar")?;
    fx.menu("View", "Back")?;
    fx.advance(Duration::ZERO)?;
    ensure!(fx.route() == app::Route::Home);
    ensure!(!menu_hidden(&fx, "View", "Toggle Sidebar")?);
    ensure!(sidebar_open(&fx) == was_open);
    fx.menu("View", "Toggle Sidebar")?;
    ensure!(sidebar_open(&fx) != was_open);
    Ok(())
}

#[test]
fn proxy() {
    common::proxy_child();
}
