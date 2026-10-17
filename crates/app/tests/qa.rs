mod common;

use anyhow::Result;
use bridge::Event;
use common::Fx;
use serde_json::json;

fn ready(fx: &mut Fx) -> Result<bool> {
    fx.cx.update_window(fx.window, |_, window, cx| {
        fx.root.read(cx).qa_restart_ready(window, cx)
    })
}

#[test]
fn drafts_and_running_turns_defer_restart() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = Fx::new()?;
    assert!(
        ready(&mut fx)?,
        "idle Home must permit a background restart"
    );
    fx.composer().update(&mut fx.cx, |composer, cx| {
        composer.set_text("unsaved QA draft", cx)
    });
    assert!(!ready(&mut fx)?, "the real draft must prevent a restart");
    fx.composer()
        .update(&mut fx.cx, |composer, cx| composer.set_text("", cx));
    assert!(
        ready(&mut fx)?,
        "clearing the draft must restore idle readiness"
    );
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(Event::Notification {
            method: "turn/started".into(),
            params: json!({"threadId": "qa-busy", "turn": {"id": "qa-turn", "status": "inProgress", "items": []}}),
        }, cx);
    });
    assert!(!ready(&mut fx)?, "a running turn must prevent a restart");
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(Event::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId": "qa-busy", "turn": {"id": "qa-turn", "status": "completed", "items": []}}),
        }, cx);
    });
    assert!(
        !ready(&mut fx)?,
        "a used session requires a manual close; preservation is not proved"
    );
    Ok(())
}

#[test]
fn an_active_window_defers_restart() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let mut fx = Fx::new()?;
    assert!(ready(&mut fx)?);
    fx.cx
        .update_window(fx.window, |_, window, _| window.activate_window())?;
    fx.cx.run_until_parked();
    assert!(!ready(&mut fx)?, "an active QA window must not restart");
    Ok(())
}

#[test]
fn normal_profile_keeps_existing_state_and_settings() -> Result<()> {
    if std::env::var_os("QA_NORMAL_PROFILE_CHILD").is_none() {
        let directory = tempfile::tempdir()?;
        let status = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "normal_profile_keeps_existing_state_and_settings",
                "--nocapture",
            ])
            .env("HOME", directory.path())
            .env("QA_NORMAL_PROFILE_CHILD", "1")
            .status()?;
        assert!(status.success());
        return Ok(());
    }
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let directory = home.join("Library/Application Support/codex-gpui");
    std::fs::create_dir_all(&directory)?;
    std::fs::write(
        directory.join("state.json"),
        br#"{"width":777,"height":666}"#,
    )?;
    let state = app::State::load();
    assert_eq!(state.width, 777.);
    assert_eq!(state.height, 666.);
    assert_eq!(state.path, Some(directory.join("state.json")));
    let settings_path = directory.join("host-settings.json");
    let mut settings = app::host_settings::Settings::load(settings_path.clone())?;
    settings.write("qa-normal-proof", json!(true))?;
    assert_eq!(
        app::host_settings::Settings::load(settings_path)?.read_all()["qa-normal-proof"],
        true
    );
    assert!(
        !home
            .join("Library/Application Support/codex-gpui-qa")
            .exists()
    );
    Ok(())
}
