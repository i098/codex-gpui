mod common;

use anyhow::{Result, ensure};
use app::{GoToChat, Route};
use common::{Fx, GREETING, exchange, recorded_thread, script};
use serde_json::json;
use ui::screens::settings::keyboard_shortcuts::{
    Change, KeyboardState, NumberTarget, Preferences, commands,
};

fn panic_hook() {
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
}

fn change(fx: &mut Fx, change: Change) {
    fx.cx.update(|cx| {
        let dispatch = cx.global::<KeyboardState>().dispatch.clone().unwrap();
        dispatch(change, cx);
    });
    fx.cx.run_until_parked();
}

#[test]
fn edits_reject_conflicts_and_reset_restores_the_number_mode() -> Result<()> {
    panic_hook();
    let mut preferences = Preferences::default();
    let command = commands()
        .iter()
        .find(|command| command.id == "newTask")
        .unwrap();
    preferences
        .change(&Change::Bindings {
            command: "newTask".into(),
            keys: vec!["Command+Shift+Y".into()],
        })
        .map_err(anyhow::Error::msg)?;
    let previous = preferences.clone();
    ensure!(
        preferences
            .change(&Change::Bindings {
                command: "newTask".into(),
                keys: vec!["Command+K".into()]
            })
            .is_err()
    );
    ensure!(preferences == previous);
    preferences
        .change(&Change::NumberTarget(NumberTarget::Sidebar))
        .map_err(anyhow::Error::msg)?;
    preferences
        .change(&Change::Reset)
        .map_err(anyhow::Error::msg)?;
    ensure!(preferences.primary_number_shortcut_target == NumberTarget::Sidebar);
    ensure!(
        preferences.keys(command).collect::<Vec<_>>() == vec!["CmdOrCtrl+N", "CmdOrCtrl+Shift+O"]
    );
    let chat = commands()
        .iter()
        .find(|command| command.id == "thread1")
        .unwrap();
    ensure!(preferences.keys(chat).next() == Some("Command+1"));
    preferences
        .change(&Change::NumberTarget(NumberTarget::Tabs))
        .map_err(anyhow::Error::msg)?;
    ensure!(preferences.keys(chat).next() == Some("Control+1"));
    Ok(())
}

#[test]
fn saved_bindings_replace_live_keys_and_survive_reload() -> Result<()> {
    panic_hook();
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("settings.json");
    let settings = app::host_settings::Settings::load(path.clone())?;
    let mut lines = script(None, |_| {})?;
    let reads: std::collections::HashSet<_> = lines
        .iter()
        .filter(|line| line["msg"]["method"] == "thread/read")
        .map(|line| line["msg"]["id"].to_string())
        .collect();
    lines.retain(|line| !reads.contains(&line["msg"]["id"].to_string()));
    for _ in 0..3 {
        lines.extend(exchange(
            "thread/read",
            json!({"thread": recorded_thread(GREETING)?}),
        ));
    }
    let mut fx = Fx::captured_with_host_settings("dark", (1280., 832.), lines.clone(), settings)?;
    change(
        &mut fx,
        Change::Bindings {
            command: "newTask".into(),
            keys: vec!["Command+Shift+Y".into()],
        },
    );
    ensure!(
        fx.cx
            .update(|cx| cx.global::<KeyboardState>().error.is_none())
    );
    fx.action(Box::new(GoToChat(0)))?;
    fx.wait_thread(GREETING)?;
    fx.keys("cmd-n")?;
    ensure!(
        fx.route() == Route::Thread(GREETING.into()),
        "the old key still opens Home"
    );
    fx.keys("cmd-shift-y")?;
    ensure!(fx.route() == Route::Home);
    drop(fx);
    let settings = app::host_settings::Settings::load(path)?;
    let mut fx = Fx::captured_with_host_settings("dark", (1280., 832.), lines, settings)?;
    fx.action(Box::new(GoToChat(0)))?;
    fx.wait_thread(GREETING)?;
    fx.keys("cmd-shift-y")?;
    ensure!(
        fx.route() == Route::Home,
        "the binding did not survive reload"
    );
    change(&mut fx, Change::Reset);
    fx.action(Box::new(GoToChat(0)))?;
    fx.wait_thread(GREETING)?;
    fx.keys("cmd-n")?;
    ensure!(
        fx.route() == Route::Home,
        "reset did not restore the default key"
    );
    Ok(())
}

#[test]
fn failed_preference_write_keeps_the_previous_bindings() -> Result<()> {
    panic_hook();
    let directory = tempfile::tempdir()?;
    let parent = directory.path().join("settings");
    std::fs::create_dir(&parent)?;
    let mut settings = app::host_settings::Settings::load(parent.join("host.json"))?;
    settings.write(
        "codex-command-keybindings",
        json!({"newTask":["Command+N"]}),
    )?;
    std::fs::remove_file(parent.join("host.json"))?;
    std::fs::remove_dir(&parent)?;
    std::fs::write(&parent, "not a directory")?;
    ensure!(
        settings
            .write_batch([
                ("codex-command-keybindings", json!({})),
                ("external-browser-link-shortcut", json!("unassigned"))
            ])
            .is_err()
    );
    ensure!(settings.read_all()["codex-command-keybindings"] == json!({"newTask":["Command+N"]}));
    ensure!(
        !settings
            .read_all()
            .contains_key("external-browser-link-shortcut")
    );
    Ok(())
}

#[test]
fn proxy() {
    common::proxy_child();
}

#[test]
fn bare_app_shortcut_does_not_intercept_composer_text() -> Result<()> {
    panic_hook();
    let mut fx = Fx::new()?;
    change(
        &mut fx,
        Change::Bindings {
            command: "toggleSidebar".into(),
            keys: vec!["X".into()],
        },
    );
    let open = fx.sidebar_open();
    fx.focus_composer()?;
    fx.type_text("x")?;
    ensure!(fx.composer_text()? == "x");
    ensure!(fx.sidebar_open() == open);
    Ok(())
}

#[test]
fn zoom_clamps_scales_input_space_and_holds_the_hovered_banner() -> Result<()> {
    panic_hook();
    let mut fx = Fx::new()?;
    for _ in 0..30 {
        fx.keys("cmd-=")?;
    }
    let (zoom, viewport) = fx.cx.update_window(fx.window, |_, window, _| {
        (window.ui_scale(), window.viewport_size())
    })?;
    ensure!(zoom == 3.);
    ensure!((f32::from(viewport.width) * zoom - 1280.).abs() < 0.01);
    ensure!(fx.read(|root, cx| {
        root.zoom_banner()
            .is_some_and(|banner| banner.read(cx).visible())
    }));
    fx.hover((1160., 25.))?;
    fx.advance(std::time::Duration::from_secs(3))?;
    ensure!(fx.read(|root, cx| {
        root.zoom_banner()
            .is_some_and(|banner| banner.read(cx).visible())
    }));
    fx.hover((500., 600.))?;
    fx.advance(std::time::Duration::from_millis(200))?;
    ensure!(fx.read(|root, cx| {
        root.zoom_banner()
            .is_some_and(|banner| !banner.read(cx).visible())
    }));
    for _ in 0..30 {
        fx.keys("cmd--")?;
    }
    ensure!(
        fx.cx
            .update_window(fx.window, |_, window, _| window.ui_scale())?
            == 0.5
    );
    fx.keys("cmd-0")?;
    ensure!(
        fx.cx
            .update_window(fx.window, |_, window, _| window.ui_scale())?
            == 1.
    );
    Ok(())
}

#[test]
fn zoom_reloads_from_saved_window_state() -> Result<()> {
    panic_hook();
    let directory = tempfile::tempdir()?;
    let state_path = directory.path().join("state.json");
    let recording = directory.path().join("replay.jsonl");
    std::fs::write(
        &recording,
        script(None, |_| {})?
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )?;
    let open = |state: app::State| -> Result<Fx> {
        let (bridge, events) = bridge::Bridge::replay(&recording)?;
        let mut fx = Fx::new()?;
        fx.root = fx.cx.update_window(fx.window, |_, window, cx| {
            window.replace_root(cx, |window, cx| {
                app::Root::new(state, bridge, events, common::CWD.into(), window, cx)
            })
        })?;
        fx.wait("restored window", |fx| fx.read(|root, _| root.ready()))?;
        Ok(fx)
    };
    let mut fx = open(app::State {
        theme: app::ThemeSetting::Dark,
        path: Some(state_path.clone()),
        ..Default::default()
    })?;
    fx.keys("cmd-=")?;
    let mut saved: app::State = serde_json::from_slice(&std::fs::read(&state_path)?)?;
    ensure!(saved.zoom == 1.1);
    drop(fx);
    saved.path = Some(state_path);
    let mut restored = open(saved)?;
    let zoom = restored
        .cx
        .update_window(restored.window, |_, window, _| window.ui_scale())?;
    ensure!(zoom == 1.1);
    Ok(())
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use gpui::{Modifiers, ModifiersChangedEvent, PlatformInput};
    use ui::screens::settings::keyboard_shortcuts::{LinkModifier, canonical};

    fn capture(fx: &mut Fx, name: &str) -> Result<()> {
        if let Some(path) = std::env::var_os("CGX_SHORTCUT_PROOF") {
            let path = std::path::Path::new(&path);
            std::fs::create_dir_all(path)?;
            fx.reference_shot()?
                .save(path.join(format!("keyboard-{name}.png")))?;
        }
        Ok(())
    }

    #[test]
    fn binding_capture_blocks_conflicts_and_applies_the_new_key() -> Result<()> {
        panic_hook();
        let mut settings = common::recorded_host_settings()?;
        settings.write("primary-number-shortcut-target", json!("tabs"))?;
        let mut fx = Fx::captured_with_host_settings(
            "dark",
            (1280., 832.),
            script(None, |_| {})?,
            settings,
        )?;
        fx.keys("cmd-,")?;
        fx.click(fx.at("settings", "Keyboard shortcuts")?)?;
        capture(&mut fx, "list")?;
        fx.click((1090.7, 278.3))?;
        capture(&mut fx, "number-menu-default")?;
        fx.click((1090.7, 278.3))?;
        fx.click((550., 203.))?;
        fx.type_text("Open web link in default browser")?;
        capture(&mut fx, "search-link")?;
        fx.click((804.3, 286.3))?;
        capture(&mut fx, "link-menu-default")?;
        fx.click((804.3, 286.3))?;
        fx.click((1117.25, 203.))?;
        fx.click((825.24, 378.57))?;
        capture(&mut fx, "capture")?;
        fx.keys("cmd-k")?;
        ensure!(fx.read(|root, _| root.command_menu().is_none()));
        ensure!(fx.cx.update(|cx| {
            cx.global::<KeyboardState>()
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Open command menu"))
        }));
        capture(&mut fx, "conflict")?;
        fx.keys("cmd-shift-y")?;
        ensure!(fx.cx.update(|cx| {
            let state = cx.global::<KeyboardState>();
            state.error.is_none()
                && state
                    .preferences
                    .bindings
                    .get("newTask")
                    .is_some_and(|keys| canonical(&keys[0]) == "cmd-shift-y")
        }));
        fx.keys("cmd-shift-y")?;
        ensure!(fx.route() == Route::Home);
        fx.keys("cmd-,")?;
        fx.click(fx.at("settings", "Keyboard shortcuts")?)?;
        change(&mut fx, Change::Reset);
        fx.click((550., 203.))?;
        fx.type_text("chat")?;
        capture(&mut fx, "search-chat")?;
        fx.click((1117.25, 203.))?;
        fx.type_text("zzzzzz")?;
        capture(&mut fx, "no-matches")?;
        fx.keys("escape")?;
        capture(&mut fx, "query-cleared")?;
        fx.click((1147.25, 203.))?;
        fx.keys("cmd-k")?;
        ensure!(fx.read(|root, _| root.command_menu().is_none()));
        capture(&mut fx, "search-keystrokes")?;
        Ok(())
    }

    #[test]
    fn dictation_capture_keeps_modifiers_released_before_the_last_key() -> Result<()> {
        panic_hook();
        let mut fx = Fx::new()?;
        fx.keys("cmd-,")?;
        fx.click(fx.at("settings", "Keyboard shortcuts")?)?;
        fx.click((550., 203.))?;
        fx.type_text("Dictation shortcut")?;
        capture(&mut fx, "dictation-search")?;
        fx.keys("tab tab tab space")?;
        ensure!(
            fx.cx
                .update(|cx| cx.global::<KeyboardState>().capture_owner.is_some())
        );
        let control = Modifiers {
            control: true,
            ..Default::default()
        };
        for modifiers in [
            control,
            Modifiers {
                shift: true,
                ..control
            },
            control,
            Modifiers::default(),
        ] {
            fx.input(PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                modifiers,
                ..Default::default()
            }))?;
        }
        ensure!(fx.cx.update(|cx| {
            cx.global::<KeyboardState>()
                .preferences
                .bindings
                .get("globalDictationHold")
                .and_then(|keys| keys.first())
                .is_some_and(|key| canonical(key) == "ctrl-shift")
        }));
        capture(&mut fx, "dictation-modifiers")?;
        Ok(())
    }

    #[test]
    fn reset_preserves_number_target_and_clears_shortcut_overrides() -> Result<()> {
        panic_hook();
        let mut fx = Fx::new()?;
        change(&mut fx, Change::NumberTarget(NumberTarget::Tabs));
        change(
            &mut fx,
            Change::Bindings {
                command: "newTask".into(),
                keys: vec!["Command+Shift+Y".into()],
            },
        );
        fx.keys("cmd-,")?;
        fx.click(fx.at("settings", "Keyboard shortcuts")?)?;
        fx.click((1100., 278.))?;
        capture(&mut fx, "number-menu")?;
        fx.click((1100., 341.))?;
        ensure!(fx.cx.update(|cx| {
            cx.global::<KeyboardState>()
                .preferences
                .primary_number_shortcut_target
                == NumberTarget::Sidebar
        }));
        fx.click((550., 203.))?;
        fx.type_text("Open web link in default browser")?;
        fx.click((804.3, 286.3))?;
        capture(&mut fx, "link-menu")?;
        fx.click((790., 379.))?;
        ensure!(fx.cx.update(|cx| {
            cx.global::<KeyboardState>()
                .preferences
                .external_browser_link_shortcut
                == LinkModifier::Alt
        }));
        fx.click((1117.25, 203.))?;
        fx.click((1100., 136.))?;
        capture(&mut fx, "reset-confirm")?;
        fx.keys("escape")?;
        ensure!(fx.cx.update(|cx| {
            cx.global::<KeyboardState>()
                .preferences
                .bindings
                .contains_key("newTask")
        }));
        fx.click((1100., 136.))?;
        fx.keys("tab enter")?;
        ensure!(fx.cx.update(|cx| {
            let preferences = &cx.global::<KeyboardState>().preferences;
            preferences.bindings.is_empty()
                && preferences.primary_number_shortcut_target == NumberTarget::Sidebar
                && preferences.external_browser_link_shortcut == LinkModifier::Primary
        }));
        capture(&mut fx, "reset-result")?;
        Ok(())
    }
}
