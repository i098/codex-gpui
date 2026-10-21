mod common;

use anyhow::{Context as _, Result, ensure};
use common::{CWD, Fx, GREETING, GREETING_PROMPT, script};
use serde_json::{Value, json};
use std::sync::Arc;

#[test]
fn standalone_menu_allocates_and_records_its_own_workspace() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let home = directory.path().join("codex");
    let workspaces = directory.path().join("standalone");
    std::fs::create_dir(&home)?;
    let settings_path = directory.path().join("host.json");
    let mut settings = app::host_settings::Settings::load(settings_path.clone())?;
    settings.write("projectlessWorkspaceRoot", json!(workspaces))?;
    let host = app::host::LocalHost::new(&home).with_host_settings_path(settings_path);
    let mut fx = Fx::captured(
        "dark",
        (1280., 832.),
        script(Some("sessions/greeting.jsonl"), |_| {})?,
    )?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(Arc::new(host), cx));
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
fn proxy() {
    common::proxy_child();
}
