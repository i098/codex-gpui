mod common;

use anyhow::{Result, ensure};
use app::Route;
use common::{Fx, script};
use serde_json::json;
use ui::keyboard::{Change, KeyboardState, NumberTarget};

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
fn proxy() {
    common::proxy_child();
}

mod native {
    use super::*;
    use gpui::{Modifiers, ModifiersChangedEvent, PlatformInput};
    use ui::keyboard::{LinkModifier, canonical};

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
