mod common;

use anyhow::{Context as _, Result, ensure};
use common::*;
use serde_json::{Value, json};
use std::sync::Arc;
use theme::ActiveTheme as _;

#[test]
fn proxy() {
    proxy_child();
}

#[test]
fn only_nonempty_override_files_take_precedence_over_personal_instructions() -> Result<()> {
    use app::host::{Host as _, LocalHost, Request, json_body};
    let home = tempfile::tempdir()?;
    let host = LocalHost::new(home.path());
    let read = || {
        json_body(&host.fetch(&Request::post(
            "codex-agents-md",
            Some(json!({"hostId": "local"})),
        ))?)
    };
    let override_path = home.path().join("AGENTS.override.md");
    assert_eq!(read()?["hasOverride"], false);
    std::fs::write(&override_path, " \t\r\n")?;
    assert_eq!(read()?["hasOverride"], false);
    std::fs::write(&override_path, "Use the project rules.\n")?;
    assert_eq!(read()?["hasOverride"], true);
    std::fs::remove_file(&override_path)?;
    std::fs::create_dir(&override_path)?;
    assert!(read().is_err());
    Ok(())
}

fn press(fx: &mut Fx, keys: &str) -> Result<()> {
    for key in keys.split(' ') {
        fx.keys(key)?;
        fx.input(gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse(key)?,
        }))?;
    }
    Ok(())
}

fn script(session: Option<&str>, edit: fn(&mut Value)) -> Result<Vec<Value>> {
    let mut lines = common::script(session, edit)?;
    for method in ["config/read", "experimentalFeature/list"] {
        let id = lines
            .iter()
            .find(|line| line["dir"] == "send" && line["msg"]["method"] == method)
            .context("recorded preference read")?["msg"]["id"]
            .clone();
        let result = lines
            .iter()
            .find(|line| line["dir"] == "recv" && line["msg"]["id"] == id)
            .context("recorded preference result")?["msg"]["result"]
            .clone();
        for _ in 0..4 {
            lines.extend(exchange(method, result.clone()));
        }
    }
    Ok(lines)
}

fn signed_out(methods: Value) -> Result<Vec<Value>> {
    let mut lines = script(None, |line| {
        if line["dir"] == "recv" && line["msg"]["result"].get("requiresOpenaiAuth").is_some() {
            line["msg"]["result"] = json!({"account": null, "requiresOpenaiAuth": true});
        }
    })?;
    for line in &mut lines {
        if line["dir"] == "recv" && line["msg"]["result"].get("requirements").is_some() {
            line["msg"]["result"] = json!({"requirements": {"allowedLoginMethods": methods}});
        }
    }
    Ok(lines)
}

fn follow_up_read(lines: &mut Vec<Value>, method: &str, result: Value) {
    let params = lines
        .iter()
        .find(|line| line["dir"] == "send" && line["msg"]["method"] == method)
        .map(|line| line["msg"]["params"].clone())
        .expect("recorded startup read");
    let mut follow_up = exchange(method, result);
    follow_up[0]["msg"]["params"] = params;
    lines.extend(follow_up);
}

fn drain(fx: &mut Fx) -> Result<()> {
    fx.sent()?;
    fx.cx.run_until_parked();
    Ok(())
}

fn evidence(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("CGX_SETTINGS_ACTIONS_EVIDENCE", name)
}

#[test]
fn a_successful_api_key_reply_enters_the_app_without_a_browser() -> Result<()> {
    let mut lines = signed_out(json!(["api"]))?;
    lines.extend(exchange("account/login/start", json!({"type": "apiKey"})));
    follow_up_read(
        &mut lines,
        "account/read",
        json!({"account": {"type": "apiKey"}, "requiresOpenaiAuth": true}),
    );
    follow_up_read(
        &mut lines,
        "thread/list",
        json!({"data": [], "nextCursor": null}),
    );
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    drain(&mut fx)?;
    ensure!(fx.read(|root, _| root.signed_out()));
    // API-only sign-in shows `Sign in with an API key`, which opens the key field.
    fx.keys("tab tab enter")?;
    fx.type_text("sk-owned-fixture-not-a-real-key")?;
    evidence(&mut fx, "api-key-form")?;
    fx.keys("enter")?;
    fx.wait("API key sign-in completion", |fx| {
        !fx.read(|root, _| root.signed_out())
    })?;
    assert_eq!(fx.route(), app::Route::Home);
    assert!(fx.cx.take_opened_url().is_none());
    Ok(())
}

#[test]
fn foreign_and_cancelled_browser_logins_cannot_authenticate() -> Result<()> {
    let mut lines = signed_out(json!(["api", "chatgpt"]))?;
    lines.extend(exchange("account/login/start", json!({
        "type": "chatgpt", "loginId": "owned-login", "authUrl": "https://example.invalid/owned-login"
    })));
    lines.extend(exchange(
        "account/login/cancel",
        json!({"status": "canceled"}),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    drain(&mut fx)?;
    fx.click((640., 411.))?;
    fx.wait("browser sign-in request", |fx| {
        fx.cx.take_opened_url().is_some()
    })?;
    let completed = |id: &str| bridge::Event::Notification {
        method: "account/login/completed".into(),
        params: json!({"loginId": id, "success": true, "error": null}),
    };
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(completed("foreign-login"), cx)
    });
    ensure!(fx.read(|root, _| root.signed_out()));
    evidence(&mut fx, "pending-login")?;
    fx.click((640., 454.))?;
    drain(&mut fx)?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(completed("owned-login"), cx)
    });
    ensure!(fx.read(|root, _| root.signed_out()));
    Ok(())
}

#[test]
fn a_policy_with_no_login_methods_never_starts_authentication() -> Result<()> {
    let mut fx = Fx::captured("dark", (1280., 832.), signed_out(json!([]))?)?;
    drain(&mut fx)?;
    fx.keys("tab")?;
    fx.type_text("sk-owned-fixture-not-a-real-key")?;
    fx.keys("enter")?;
    ensure!(fx.read(|root, _| root.signed_out()));
    assert!(
        !fx.sent()?
            .iter()
            .any(|request| request["method"] == "account/login/start")
    );
    evidence(&mut fx, "login-policy-blocked")?;
    Ok(())
}

#[test]
fn settings_search_and_theme_work_from_the_keyboard() -> Result<()> {
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.keys("cmd-,")?;
    evidence(&mut fx, "settings-command")?;
    fx.click((200., 118.))?;
    fx.type_text("Appearance")?;
    press(&mut fx, "tab enter")?;
    evidence(&mut fx, "settings-navigation-focus")?;
    press(&mut fx, "tab tab enter")?;
    assert_eq!(
        fx.read(|_, cx| cx.theme().surface),
        theme::Theme::light().surface
    );
    evidence(&mut fx, "settings-theme-keyboard")?;
    Ok(())
}

fn host_value(fx: &Fx, key: &str) -> Value {
    fx.read(|_, cx| {
        cx.global::<app::host_settings::Settings>()
            .read_all()
            .get(key)
            .cloned()
            .unwrap_or(Value::Null)
    })
}

/// Copy theme, Import theme (a Dracula share string saves its theme and code theme together),
/// the Advanced rows, and Reset to default, all from the keyboard.
#[test]
fn appearance_theme_share_advanced_rows_and_reset() -> Result<()> {
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    // Blur listeners see focus paths only in an active window.
    fx.cx
        .update_window(fx.window, |_, window, _| window.activate_window())?;
    fx.keys("cmd-,")?;
    fx.click((200., 118.))?;
    fx.type_text("Appearance")?;
    press(&mut fx, "tab enter")?;
    // Mode's three segments, Import, then Copy.
    press(&mut fx, "tab tab tab tab tab tab enter")?;
    let copied = fx
        .cx
        .update(|cx| cx.read_from_clipboard())
        .and_then(|item| item.text());
    let copied = copied.context("theme share string")?;
    let share: Value = serde_json::from_str(
        copied
            .strip_prefix("codex-theme-v1:")
            .context("share prefix")?,
    )?;
    ensure!(
        share["variant"] == "dark" && share["codeThemeId"] == "codex",
        "{share}"
    );
    ensure!(share["theme"]["surface"] == "#181818", "{share}");
    fx.wait("copied toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|t| t.as_ref() == "Dark theme copied")
        })
    })?;

    let dracula = json!({
        "codeThemeId": "dracula",
        "variant": "dark",
        "theme": {
            "accent": "#ff79c6", "contrast": 60, "fonts": {"code": null, "ui": null},
            "ink": "#f8f8f2", "opaqueWindows": false, "surface": "#282a36",
            "semanticColors": {"diffAdded": "#50fa7b", "diffRemoved": "#ff5555", "skill": "#ff79c6"},
            "accentSource": "custom"
        }
    });
    press(&mut fx, "shift-tab enter")?;
    fx.type_text("codex-theme-v1:{\"variant\":\"light\"}")?;
    press(&mut fx, "enter")?;
    fx.wait("import error", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|t| t.as_ref() == "Couldn’t import Dark theme")
        })
    })?;
    press(&mut fx, "cmd-a")?;
    fx.type_text(&format!("codex-theme-v1:{dracula}"))?;
    press(&mut fx, "enter")?;
    fx.wait("imported theme", |fx| {
        host_value(fx, "appearanceDarkCodeThemeId") == "dracula"
    })?;
    ensure!(host_value(&fx, "appearanceDarkChromeTheme")["surface"] == "#282a36");

    // Advanced: UI font size accepts 11-16 and restores anything else.
    // Keys dispatch against the last frame, and blur commits run when a frame draws.
    let frame = |fx: &mut Fx| fx.advance(std::time::Duration::from_millis(16));
    frame(&mut fx)?;
    // Closing the dialog refocuses Import; Dracula's custom accent adds a hex field.
    let advanced = "tab ".repeat(8);
    press(&mut fx, &format!("{advanced}enter"))?;
    frame(&mut fx)?;
    // Tab out of a size field commits it, like blur in the original.
    press(&mut fx, "tab cmd-a")?;
    fx.type_text("15")?;
    press(&mut fx, "tab")?;
    frame(&mut fx)?;
    fx.wait("UI font size", |fx| host_value(fx, "sansFontSize") == 15)?;
    press(&mut fx, "shift-tab cmd-a")?;
    fx.type_text("40")?;
    press(&mut fx, "tab")?;
    frame(&mut fx)?;
    ensure!(
        host_value(&fx, "sansFontSize") == 15,
        "out-of-range size saved"
    );
    // From Code font size: Reduce motion's `System`, then `On`.
    press(&mut fx, "tab tab enter")?;
    fx.wait("reduced motion", |fx| {
        host_value(fx, "reduced-motion-preference") == "on"
    })?;
    ensure!(
        fx.cx.update(|cx| cx.reduce_motion()),
        "reduced motion not applied"
    );
    frame(&mut fx)?;

    // Reset restores the Advanced defaults and keeps the imported preset.
    press(&mut fx, "shift-tab shift-tab shift-tab shift-tab enter")?;
    fx.wait("reset", |fx| host_value(fx, "sansFontSize") == 14)?;
    ensure!(host_value(&fx, "reduced-motion-preference") == "system");
    ensure!(!fx.cx.update(|cx| cx.reduce_motion()));
    ensure!(host_value(&fx, "appearanceDarkCodeThemeId") == "dracula");
    Ok(())
}

#[test]
fn a_failed_setting_write_keeps_the_saved_value_and_shows_an_error() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    let mut write = exchange("config/value/write", Value::Null);
    write[1]["msg"].as_object_mut().unwrap().remove("result");
    write[1]["msg"]["error"] =
        json!({"code": -32000, "message": "owned read-only settings fixture"});
    lines.extend(write);
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    let saved = fx.read(|root, _| root.config()["desktop"]["defaultTerminalLocation"].clone());
    fx.keys("cmd-,")?;
    fx.click((200., 118.))?;
    fx.type_text("General")?;
    press(&mut fx, "tab enter")?;
    press(&mut fx, "tab tab tab tab tab tab tab tab space")?;
    fx.wait("settings save failure", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|title| title.as_ref() == "Couldn't save your changes. Try again")
        })
    })?;
    assert_eq!(
        fx.read(|root, _| root.config()["desktop"]["defaultTerminalLocation"].clone()),
        saved
    );
    evidence(&mut fx, "settings-save-failure")?;
    Ok(())
}

// These coordinates require macOS text layout; Linux uses the no-op text system.
#[cfg(target_os = "macos")]
fn prepare_instructions(fx: &mut Fx) -> Result<()> {
    fx.keys("cmd-,")?;
    fx.click((200., 118.))?;
    fx.keys("escape")?;
    fx.type_text("Personalization")?;
    press(fx, "tab enter")?;
    fx.click((200., 118.))?;
    fx.keys("escape")?;
    fx.wait("memory settings ready", |fx| {
        fx.read(|root, cx| {
            root.personal_instructions(cx)
                .is_some_and(|view| view.read(cx).memory().is_some())
        })
    })?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn show_instructions(fx: &mut Fx) -> Result<()> {
    prepare_instructions(fx)?;
    fx.click((800., 540.))
}

#[cfg(target_os = "macos")]
fn open_instructions(fx: &mut Fx) -> Result<()> {
    show_instructions(fx)?;
    fx.wait("instructions editor focus", |fx| {
        fx.read(|root, cx| {
            root.personal_instructions(cx)
                .is_some_and(|view| view.read(cx).editor_focus(cx).is_some())
        })
    })?;
    fx.click((200., 118.))?;
    fx.keys("escape")?;
    fx.click((449., 225.))?;
    fx.keys("left")?;
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn instructions_save_cancel_and_navigation_guard_preserve_the_document() -> Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join("AGENTS.md");
    std::fs::write(&path, "Use spaces.\n")?;
    let host = std::sync::Arc::new(app::host::LocalHost::new(home.path()));
    let mut fx = Fx::scripted(script(None, |_| {})?)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    open_instructions(&mut fx)?;
    evidence(&mut fx, "instructions-loaded")?;
    fx.keys("cmd-a")?;
    fx.type_text("Use tabs.")?;
    evidence(&mut fx, "instructions-edited")?;
    press(&mut fx, "shift-tab shift-tab enter")?;
    fx.wait("instructions saved", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|text| text.as_ref() == "Instructions updated")
        })
    })?;
    evidence(&mut fx, "instructions-saved")?;
    assert_eq!(std::fs::read_to_string(&path)?, "Use tabs.");

    open_instructions(&mut fx)?;
    fx.keys("cmd-a")?;
    fx.type_text("Do not save this.")?;
    press(&mut fx, "shift-tab shift-tab shift-tab enter")?;
    assert_eq!(std::fs::read_to_string(&path)?, "Use tabs.");

    open_instructions(&mut fx)?;
    fx.keys("cmd-a cmd-c")?;
    assert_eq!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|value| value.text()))
            .as_deref(),
        Some("Use tabs.")
    );
    fx.type_text("Unsaved instructions.")?;
    fx.keys("cmd-q")?;
    assert_eq!(
        fx.read(|root, cx| root.dialog().map(|dialog| dialog.read(cx).kind())),
        Some(ui::dialog::Kind::DiscardInstructions)
    );
    fx.keys("escape")?;
    fx.keys("cmd-[")?;
    assert_eq!(
        fx.read(|root, cx| root.dialog().map(|dialog| dialog.read(cx).kind())),
        Some(ui::dialog::Kind::DiscardInstructions)
    );
    evidence(&mut fx, "instructions-discard-dialog")?;
    fx.keys("enter")?;
    assert!(fx.read(|root, _| root.dialog().is_none()));
    fx.keys("cmd-a cmd-c")?;
    assert_eq!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|value| value.text()))
            .as_deref(),
        Some("Unsaved instructions.")
    );
    fx.keys("cmd-[ tab enter")?;
    assert!(fx.read(|root, _| root.dialog().is_none()));
    assert_eq!(std::fs::read_to_string(&path)?, "Use tabs.");
    fx.keys("cmd-[")?;
    assert_eq!(fx.route(), app::Route::Home);
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn an_override_notice_keeps_the_personal_document_editable() -> Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join("AGENTS.md");
    std::fs::write(&path, "Use spaces.\n")?;
    std::fs::write(home.path().join("AGENTS.override.md"), "Prefer tabs.\n")?;
    let host = std::sync::Arc::new(app::host::LocalHost::new(home.path()));
    let mut fx = Fx::scripted(script(None, |_| {})?)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    open_instructions(&mut fx)?;
    fx.click((800., 380.))?;
    evidence(&mut fx, "instructions-override")?;
    fx.keys("cmd-a")?;
    fx.type_text("Keep instructions editable.")?;
    press(&mut fx, "shift-tab shift-tab enter")?;
    fx.wait("overridden personal instructions saved", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|text| text.as_ref() == "Instructions updated")
        })
    })?;
    assert_eq!(
        std::fs::read_to_string(path)?,
        "Keep instructions editable."
    );
    assert_eq!(
        std::fs::read_to_string(home.path().join("AGENTS.override.md"))?,
        "Prefer tabs.\n"
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn source_and_preview_keep_the_same_unsaved_instructions() -> Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join("AGENTS.md");
    std::fs::write(&path, "# Project rules\n\nUse spaces.\n")?;
    let host = std::sync::Arc::new(app::host::LocalHost::new(home.path()));
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    prepare_instructions(&mut fx)?;
    drain(&mut fx)?;
    let view = fx.read(|root, cx| root.personal_instructions(cx).unwrap().clone());
    view.update(&mut fx.cx, |view, cx| {
        view.set_unified_instructions(true, cx)
    });
    fx.click((800., 540.))?;
    fx.wait("Markdown instructions loaded", |fx| {
        fx.read(|root, cx| {
            root.personal_instructions(cx)
                .is_some_and(|view| !view.read(cx).loading())
        })
    })?;
    fx.click((700., 67.))?;
    evidence(&mut fx, "instructions-preview")?;
    fx.click((1080., 67.))?;
    evidence(&mut fx, "instructions-source-initial")?;
    fx.keys("cmd-a")?;
    fx.type_text("# Updated rules\n\nKeep this **draft**.")?;
    evidence(&mut fx, "instructions-source")?;
    fx.click((1080., 67.))?;
    evidence(&mut fx, "instructions-preview-draft")?;
    fx.click((1080., 67.))?;
    fx.keys("cmd-a cmd-c")?;
    assert_eq!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
            .as_deref(),
        Some("# Updated rules\n\nKeep this **draft**.")
    );
    fx.click((1230., 67.))?;
    fx.wait("Markdown instructions saved", |_| {
        std::fs::read_to_string(&path).ok().as_deref()
            == Some("# Updated rules\n\nKeep this **draft**.")
    })?;
    fx.wait("Markdown editor closed", |fx| {
        fx.read(|root, cx| {
            root.personal_instructions(cx)
                .is_some_and(|view| !view.read(cx).opened())
        })
    })?;
    std::fs::write(&path, "# Project rules\r\n\r\nUse spaces.\r\n")?;
    fx.click((800., 540.))?;
    fx.wait("CRLF instructions loaded", |fx| {
        fx.read(|root, cx| {
            root.personal_instructions(cx)
                .is_some_and(|view| view.read(cx).opened() && !view.read(cx).loading())
        })
    })?;
    fx.click((1080., 67.))?;
    fx.keys("cmd-a")?;
    fx.type_text("# Updated rules\n\nKeep this **draft**.\n")?;
    fx.click((1230., 67.))?;
    fx.wait("CRLF instructions saved", |_| {
        std::fs::read_to_string(&path).ok().as_deref()
            == Some("# Updated rules\r\n\r\nKeep this **draft**.\r\n")
    })?;
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn the_instructions_grip_resizes_the_field_down_to_its_minimum() -> Result<()> {
    let home = tempfile::tempdir()?;
    std::fs::write(home.path().join("AGENTS.md"), "Use spaces.\n")?;
    let host = std::sync::Arc::new(app::host::LocalHost::new(home.path()));
    let mut fx = Fx::scripted(script(None, |_| {})?)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    open_instructions(&mut fx)?;
    // The focused field's blue bottom border, at a device row of the 2x shot.
    let border = |fx: &mut Fx, css_y: f32| -> Result<bool> {
        let pixel = *fx.shot()?.get_pixel(1600, (css_y * 2.) as u32);
        Ok(pixel[0] == 58 && pixel[1] == 131 && pixel[2] == 247)
    };
    ensure!(
        border(&mut fx, 471.5)?,
        "the field starts at its loaded height"
    );
    // Far above the field: the 96 px `min-h-24` box ends at 212.59 + 96 CSS px.
    fx.drag((1164., 464.), (1164., 100.), 4)?;
    ensure!(!border(&mut fx, 471.5)? && border(&mut fx, 308.)?);
    // From the minimum, 100 px down: the box ends at 308.59 + 100 CSS px.
    fx.drag((1164., 300.), (1164., 400.), 4)?;
    ensure!(border(&mut fx, 408.)? && !border(&mut fx, 308.)?);
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn a_failed_instruction_save_keeps_the_draft_for_retry() -> Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join("AGENTS.md");
    let backup = home.path().join("saved-instructions");
    std::fs::write(&path, "Original instructions.")?;
    let mut fx = Fx::scripted(script(None, |_| {})?)?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(
            std::sync::Arc::new(app::host::LocalHost::new(home.path())),
            cx,
        )
    });
    open_instructions(&mut fx)?;
    fx.keys("cmd-a")?;
    fx.type_text("Retain this draft.")?;
    std::fs::rename(&path, &backup)?;
    std::fs::create_dir(&path)?;
    press(&mut fx, "shift-tab shift-tab enter")?;
    fx.wait("instructions save refusal", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|text| text.as_ref() == "Unable to save instructions")
        })
    })?;
    assert_eq!(std::fs::read_to_string(&backup)?, "Original instructions.");
    evidence(&mut fx, "instructions-save-failure")?;
    std::fs::remove_dir(&path)?;
    std::fs::rename(&backup, &path)?;
    press(&mut fx, "enter")?;
    fx.wait("instructions retry", |_| {
        std::fs::read_to_string(&path).ok().as_deref() == Some("Retain this draft.")
    })?;
    Ok(())
}

#[test]
fn workspace_skip_creates_a_distinct_project_before_entering_home() -> Result<()> {
    use app::host::Host as _;
    let home = tempfile::tempdir()?;
    let original = home.path().join("projects/New project");
    std::fs::create_dir_all(&original)?;
    std::fs::write(original.join("keep.txt"), "keep")?;
    let host = std::sync::Arc::new(app::host::LocalHost::new(home.path()));
    let mut fx = Fx::on_route(app::Route::SelectWorkspace)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    evidence(&mut fx, "workspace-select")?;
    press(&mut fx, "tab tab enter")?;
    fx.wait("workspace created", |fx| fx.route() == app::Route::Home)?;
    let root = home.path().join("projects/New project 2").canonicalize()?;
    assert_eq!(std::fs::read_to_string(original.join("keep.txt"))?, "keep");
    assert!(
        host.projects()?
            .iter()
            .any(|project| project.root.as_deref() == root.to_str())
    );
    Ok(())
}

#[test]
fn workspace_picker_cancel_stays_then_selection_enters_home() -> Result<()> {
    use app::host::Host as _;
    let home = tempfile::tempdir()?;
    let folder = home.path().join("chosen-project");
    std::fs::create_dir(&folder)?;
    let canonical_folder = folder.canonicalize()?;
    let host = std::sync::Arc::new(app::host::LocalHost::new(home.path()));
    let mut fx = Fx::on_route(app::Route::SelectWorkspace)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.click((640., 496.))?;
    evidence(&mut fx, "workspace-add-menu")?;
    press(&mut fx, "tab tab tab enter")?;
    assert!(fx.cx.did_prompt_for_paths());
    fx.cx.simulate_path_prompt_response(|_| None);
    fx.cx.run_until_parked();
    assert_eq!(fx.route(), app::Route::SelectWorkspace);
    fx.keys("cmd-o")?;
    assert!(fx.cx.did_prompt_for_paths());
    fx.cx.simulate_path_prompt_response(|options| {
        assert!(options.directories && !options.files && !options.multiple);
        Some(vec![folder.clone()])
    });
    fx.wait("selected workspace", |fx| fx.route() == app::Route::Home)?;
    assert!(
        host.projects()?
            .iter()
            .any(|project| project.root.as_deref() == canonical_folder.to_str())
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn instruction_load_failure_can_retry_without_losing_the_page() -> Result<()> {
    let home = tempfile::tempdir()?;
    let path = home.path().join("AGENTS.md");
    std::fs::create_dir(&path)?;
    let host = std::sync::Arc::new(app::host::LocalHost::new(home.path()));
    let mut fx = Fx::scripted(script(None, |_| {})?)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    prepare_instructions(&mut fx)?;
    let position = gpui::point(gpui::px(800.), gpui::px(540.));
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_event(
            gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                button: gpui::MouseButton::Left,
                position,
                modifiers: gpui::Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                button: gpui::MouseButton::Left,
                position,
                modifiers: gpui::Modifiers::default(),
                click_count: 1,
            }),
            cx,
        );
    })?;
    assert!(fx.read(|root, cx| root.personal_instructions(cx).unwrap().read(cx).loading()));
    if let Some(out) = std::env::var_os("CGX_SETTINGS_ACTIONS_EVIDENCE") {
        let out = std::path::PathBuf::from(out);
        std::fs::create_dir_all(&out)?;
        fx.cx
            .capture_screenshot(fx.window)?
            .save(out.join("instructions-loading.png"))?;
    }
    fx.cx.run_until_parked();
    fx.wait("instructions load error", |fx| {
        fx.read(|root, cx| !root.personal_instructions(cx).unwrap().read(cx).loading())
    })?;
    evidence(&mut fx, "instructions-load-error")?;
    std::fs::remove_dir(&path)?;
    std::fs::write(&path, "Recovered instructions.")?;
    fx.click((1150., 218.))?;
    fx.wait("instructions retry ready", |fx| {
        fx.read(|root, cx| {
            root.personal_instructions(cx)
                .unwrap()
                .read(cx)
                .editor_focus(cx)
                .is_some()
        })
    })?;
    fx.keys("cmd-a cmd-c")?;
    assert_eq!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|value| value.text()))
            .as_deref(),
        Some("Recovered instructions.")
    );
    Ok(())
}

#[test]
fn a_failed_browser_login_preserves_the_error_and_allows_retry() -> Result<()> {
    let mut lines = signed_out(json!(["api", "chatgpt"]))?;
    for id in ["failed-login", "retried-login"] {
        lines.extend(exchange(
            "account/login/start",
            json!({
                "type": "chatgpt", "loginId": id, "authUrl": format!("https://example.invalid/{id}")
            }),
        ));
    }
    follow_up_read(
        &mut lines,
        "account/read",
        json!({"account": {"type": "chatgpt", "email": "user@example.com", "planType": "plus"}, "requiresOpenaiAuth": true}),
    );
    follow_up_read(
        &mut lines,
        "thread/list",
        json!({"data": [], "nextCursor": null}),
    );
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.click((640., 411.))?;
    fx.wait("first browser request", |fx| {
        fx.cx.take_opened_url().is_some()
    })?;
    fx.root.update(&mut fx.cx, |root, cx| root.on_event(bridge::Event::Notification {
        method: "account/login/completed".into(),
        params: json!({"loginId": "failed-login", "success": false, "error": "owned sign-in failure"}),
    }, cx));
    assert!(fx.read(|root, _| root.signed_out()));
    evidence(&mut fx, "login-error")?;
    fx.click((640., 385.))?;
    fx.wait("retried browser request", |fx| {
        fx.cx.take_opened_url().is_some()
    })?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.on_event(
            bridge::Event::Notification {
                method: "account/login/completed".into(),
                params: json!({"loginId": "retried-login", "success": true, "error": null}),
            },
            cx,
        )
    });
    assert!(!fx.read(|root, _| root.signed_out()));
    Ok(())
}

#[cfg(unix)]
#[test]
fn saving_instructions_preserves_a_symlink_and_private_permissions() -> Result<()> {
    use app::host::Host as _;
    use std::os::unix::fs::{PermissionsExt, symlink};
    let home = tempfile::tempdir()?;
    let target = home.path().join("dotfile-instructions");
    let link = home.path().join("AGENTS.md");
    std::fs::write(&target, "Before")?;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))?;
    symlink(&target, &link)?;
    let host = app::host::LocalHost::new(home.path());
    host.fetch(&app::host::Request::post(
        "codex-agents-md-save",
        Some(json!({"contents": "After"})),
    ))?;
    assert!(std::fs::symlink_metadata(&link)?.file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(&target)?, "After");
    assert_eq!(
        std::fs::metadata(&target)?.permissions().mode() & 0o777,
        0o600
    );
    Ok(())
}

#[test]
fn a_failed_policy_read_never_starts_authentication() -> Result<()> {
    let mut lines = signed_out(Value::Null)?;
    let id = lines
        .iter()
        .find(|line| line["dir"] == "send" && line["msg"]["method"] == "configRequirements/read")
        .expect("recorded requirements read")["msg"]["id"]
        .clone();
    for line in &mut lines {
        if line["dir"] == "recv" && line["msg"]["id"] == id {
            line["msg"].as_object_mut().unwrap().remove("result");
            line["msg"]["error"] = json!({"code": -32000, "message": "owned policy read failure"});
        }
    }
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    evidence(&mut fx, "login-requirements-error")?;
    fx.keys("tab enter")?;
    assert!(fx.read(|root, _| root.signed_out()));
    assert!(
        !fx.sent()?
            .iter()
            .any(|message| message["method"] == "account/login/start")
    );
    Ok(())
}

#[test]
fn failed_workspace_registration_removes_only_the_new_empty_folder() -> Result<()> {
    use app::host::Host as _;
    let home = tempfile::tempdir()?;
    let state = home.path().join(".codex-global-state.json");
    std::fs::write(&state, "[]")?;
    let host = app::host::LocalHost::new(home.path());
    assert!(
        host.fetch(&app::host::Request::post(
            "electron-onboarding-skip-workspace",
            None
        ))
        .is_err()
    );
    assert!(!home.path().join("projects/New project").exists());
    assert_eq!(std::fs::read_to_string(state)?, "[]");
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn resizing_instructions_keeps_the_document_and_scrolls_within_the_field() -> Result<()> {
    let home = tempfile::tempdir()?;
    std::fs::write(home.path().join("AGENTS.md"), "Resize me")?;
    let mut fx = Fx::scripted(script(None, |_| {})?)?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(
            std::sync::Arc::new(app::host::LocalHost::new(home.path())),
            cx,
        );
    });
    open_instructions(&mut fx)?;
    let span = |image: &image::RgbaImage| -> Result<(u32, u32)> {
        let rows: Vec<u32> = (0..image.height())
            .filter(|&y| {
                let pixel = image.get_pixel(1800, y);
                pixel[0] < 100 && pixel[1] > 80 && pixel[1] < 180 && pixel[2] > 180
            })
            .collect();
        Ok((
            *rows.first().context("field top border")?,
            *rows.last().context("field bottom border")?,
        ))
    };
    let initial = span(&fx.reference_shot()?)?;
    fx.drag((1168., 468.), (1168., 618.), 4)?;
    let enlarged = span(&fx.reference_shot()?)?;
    assert!((298..=302).contains(&(enlarged.1 - initial.1)));
    fx.drag((1168., 618.), (1168., 250.), 4)?;
    let minimum = span(&fx.reference_shot()?)?;
    assert!((190..=194).contains(&(minimum.1 - minimum.0)));
    let document = (0..40)
        .map(|line| format!("Instruction line {line}\n"))
        .collect::<String>()
        + "LAST_VISIBLE_LINE";
    fx.keys("cmd-a")?;
    fx.type_text(&document)?;
    let filled = span(&fx.reference_shot()?)?;
    assert_eq!(filled, minimum);
    evidence(&mut fx, "instructions-resized-scroll")?;
    fx.keys("cmd-a cmd-c")?;
    assert_eq!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some(document)
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn enabling_memory_updates_the_persisted_state_and_the_switch() -> Result<()> {
    let mut lines = common::script(None, |_| {})?;
    let initial = lines
        .iter()
        .find_map(|line| {
            (line["dir"] == "recv" && line["msg"]["result"]["config"].is_object())
                .then(|| line["msg"]["result"].clone())
        })
        .context("initial memory config")?;
    let mut enabled = initial.clone();
    enabled["config"]["memories"] = json!({"generate_memories": true, "use_memories": true});
    enabled["config"]["features"]["memories"] = true.into();
    let features = lines
        .iter()
        .find_map(|line| {
            (line["dir"] == "recv"
                && line["msg"]["result"]["data"]
                    .as_array()
                    .is_some_and(|features| {
                        features.iter().any(|feature| feature["name"] == "memories")
                    }))
            .then(|| line["msg"]["result"].clone())
        })
        .context("initial memory features")?;
    // Opening Settings reads the configuration once before the Memory read.
    lines.extend(exchange("config/read", initial.clone()));
    lines.extend(exchange("config/read", initial));
    lines.extend(exchange("config/read", enabled));
    lines.extend(exchange("experimentalFeature/list", features));
    lines.extend(exchange(
        "experimentalFeature/list",
        json!({
            "data": [{"name": "memories", "enabled": true}], "nextCursor": null
        }),
    ));
    lines.extend(exchange("config/batchWrite", Value::Null));
    lines.extend(exchange("config/batchWrite", Value::Null));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    prepare_instructions(&mut fx)?;
    ensure!(fx.read(|root, cx| {
        root.personal_instructions(cx)
            .and_then(|view| view.read(cx).memory())
            .is_some_and(|memory| !memory.enabled())
    }));
    fx.click((1140., 270.))?;
    fx.wait("enabled memory", |fx| {
        fx.read(|root, cx| {
            root.personal_instructions(cx)
                .and_then(|view| view.read(cx).memory())
                .is_some_and(|memory| memory.enabled())
        })
    })?;
    assert_eq!(
        fx.read(|root, _| root.config()["features"]["memories"].clone()),
        json!(true)
    );
    assert_eq!(
        fx.read(|root, _| root.config()["memories"]["generate_memories"].clone()),
        json!(true)
    );
    assert_eq!(
        fx.read(|root, _| root.config()["memories"]["use_memories"].clone()),
        json!(true)
    );
    let writes: Vec<_> = fx
        .sent()?
        .into_iter()
        .filter(|request| request["method"] == "config/batchWrite")
        .map(|request| request["params"]["edits"].clone())
        .collect();
    assert_eq!(
        writes,
        [
            json!([{"keyPath": "features.memories", "value": true, "mergeStrategy": "upsert"}]),
            json!([
                {"keyPath": "memories.generate_memories", "value": true, "mergeStrategy": "upsert"},
                {"keyPath": "memories.use_memories", "value": true, "mergeStrategy": "upsert"}
            ])
        ]
    );
    evidence(&mut fx, "memory-enabled")?;
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn deleting_memory_requires_confirmation_and_cancel_sends_no_reset() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("memory/reset", Value::Null));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    prepare_instructions(&mut fx)?;
    fx.click((1125., 392.))?;
    assert_eq!(
        fx.read(|root, cx| root.dialog().map(|dialog| dialog.read(cx).kind())),
        Some(ui::dialog::Kind::DeleteMemories)
    );
    evidence(&mut fx, "memory-delete-confirmation")?;
    fx.keys("enter")?;
    assert!(fx.read(|root, _| root.dialog().is_none()));
    assert!(
        !fx.sent()?
            .iter()
            .any(|request| request["method"] == "memory/reset")
    );
    fx.click((1125., 392.))?;
    press(&mut fx, "tab enter")?;
    fx.wait("memory reset", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|message| message.as_ref() == "Codex memories deleted")
        })
    })?;
    assert!(fx.read(|root, _| root.dialog().is_none()));
    Ok(())
}

#[cfg(target_os = "macos")]
struct WritingSequenceHost {
    states: std::sync::Mutex<std::collections::VecDeque<bool>>,
    enables: std::sync::atomic::AtomicUsize,
}

#[cfg(target_os = "macos")]
impl app::host::Host for WritingSequenceHost {
    fn identity(&self) -> Option<app::host::Identity> {
        Some(app::host::Identity {
            account_id: "account".into(),
            user_id: "user".into(),
            device_id: "device".into(),
        })
    }

    fn projects(&self) -> Result<Vec<ui::screens::sidebar::Project>> {
        Ok(Vec::new())
    }

    fn fetch(&self, request: &app::host::Request) -> Result<Value> {
        let app::host::Request::Http(request) = request else {
            anyhow::bail!("unexpected local request");
        };
        let url = request["url"].as_str().context("HTTP URL")?;
        let body = if url.starts_with("/ps/plugins/canonical-app/") {
            json!({"plugin_id": "writing-plugin"})
        } else if url == "/ps/plugins/installed?limit=1000" {
            let mut states = self.states.lock().unwrap();
            let enabled = if states.len() > 1 {
                states.pop_front().unwrap()
            } else {
                states[0]
            };
            json!({"plugins": [{
                "id": "writing-plugin", "name": "writing", "enabled": enabled,
                "canonical_app_id": "connector_openai_write_like_me", "status": "ENABLED",
                "installation_policy": "AVAILABLE", "release": {"display_name": "Writing style"}
            }], "pagination": {"next_page_token": null}})
        } else if url.ends_with("/enable") {
            self.enables
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.states.lock().unwrap() = [true].into();
            Value::Null
        } else if url == "/aip/connectors/links/list_accessible" {
            json!({"links": []})
        } else if url.starts_with("/ps/plugins/plugin_") {
            return Ok(json!({"status": 404, "bodyJsonString": "{}"}));
        } else {
            anyhow::bail!("unexpected Writing request");
        };
        Ok(json!({"status": 200, "bodyJsonString": body.to_string()}))
    }
}

#[cfg(target_os = "macos")]
#[test]
fn retrying_a_failed_writing_disable_does_not_reenable_a_saved_change() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    let mut failed = exchange("plugin/uninstall", Value::Null);
    failed[1]["msg"].as_object_mut().unwrap().remove("result");
    failed[1]["msg"]["error"] = json!({"code": -32000, "message": "response lost"});
    lines.extend(failed);
    lines.extend(exchange("plugin/uninstall", Value::Null));
    lines.extend(exchange("skills/list", json!({"data": []})));
    let host = std::sync::Arc::new(WritingSequenceHost {
        states: std::sync::Mutex::new([true, false, false].into()),
        enables: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    prepare_instructions(&mut fx)?;
    fx.wait("Writing preference", |fx| {
        fx.read(|root, cx| {
            root.personal_instructions(cx)
                .and_then(|view| view.read(cx).writing_preference())
                .is_some_and(|preference| preference.enabled)
        })
    })?;
    fx.click((1140., 689.))?;
    fx.wait("Writing retry", |fx| {
        fx.read(|root, cx| {
            root.personal_instructions(cx)
                .is_some_and(|view| view.read(cx).can_retry_writing())
        })
    })?;
    assert!(!fx.read(|root, cx| {
        root.personal_instructions(cx)
            .unwrap()
            .read(cx)
            .writing_preference()
            .unwrap()
            .enabled
    }));
    fx.click((800., 737.))?;
    fx.wait("second Writing removal", |fx| {
        fx.sent().is_ok_and(|requests| {
            requests
                .iter()
                .filter(|request| request["method"] == "plugin/uninstall")
                .count()
                == 2
        })
    })?;
    fx.wait("Writing retry complete", |fx| {
        fx.read(|root, cx| {
            root.personal_instructions(cx)
                .is_some_and(|view| !view.read(cx).can_retry_writing())
        })
    })?;
    assert_eq!(host.enables.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(!fx.read(|root, cx| {
        root.personal_instructions(cx)
            .unwrap()
            .read(cx)
            .writing_preference()
            .unwrap()
            .enabled
    }));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn writing_setup_opens_the_sample_draft_without_starting_a_turn() -> Result<()> {
    let mut lines = script(None, |_| {})?;
    for _ in 0..2 {
        lines.extend(exchange("skills/list", json!({"data": []})));
        // Opening Settings also lists apps before the Writing setup reads them.
        lines.extend(exchange(
            "app/list",
            json!({"data": [], "nextCursor": null}),
        ));
    }
    lines.extend(exchange(
        "plugin/installed",
        json!({"marketplaces": [], "marketplaceLoadErrors": []}),
    ));
    let host = std::sync::Arc::new(WritingSequenceHost {
        states: std::sync::Mutex::new([false].into()),
        enables: std::sync::atomic::AtomicUsize::new(0),
    });
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    prepare_instructions(&mut fx)?;
    fx.wait("Writing preference", |fx| {
        fx.read(|root, cx| {
            root.personal_instructions(cx)
                .and_then(|view| view.read(cx).writing_preference())
                .is_some()
        })
    })?;
    fx.click((1140., 689.))?;
    fx.wait("Writing setup", |fx| {
        fx.read(|root, cx| {
            root.dialog()
                .is_some_and(|dialog| dialog.read(cx).kind() == ui::dialog::Kind::WritingStyle)
        })
    })?;
    drain(&mut fx)?;
    assert_eq!(fx.route(), app::Route::Home);
    evidence(&mut fx, "writing-setup")?;
    fx.click((640., 680.))?;
    fx.wait("Writing draft", |fx| fx.route() == app::Route::Home)?;
    assert!(fx.read(|root, _| root.dialog().is_none()));
    assert!(
        !fx.sent()?
            .iter()
            .any(|request| request["method"] == "turn/start")
    );
    assert_eq!(host.enables.load(std::sync::atomic::Ordering::SeqCst), 1);
    evidence(&mut fx, "writing-draft")?;
    Ok(())
}

struct WritingRaceHost {
    local: app::host::LocalHost,
    installed: bool,
    enabled: std::sync::atomic::AtomicBool,
}

impl WritingRaceHost {
    fn new(home: &std::path::Path, installed: bool) -> Self {
        Self {
            local: app::host::LocalHost::new(home),
            installed,
            enabled: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl app::host::Host for WritingRaceHost {
    fn identity(&self) -> Option<app::host::Identity> {
        Some(app::host::Identity {
            account_id: "account".into(),
            user_id: "user".into(),
            device_id: "device".into(),
        })
    }

    fn fetch(&self, request: &app::host::Request) -> Result<Value> {
        let app::host::Request::Http(request) = request else {
            return app::host::Host::fetch(&self.local, request);
        };
        let url = request["url"].as_str().context("HTTP URL")?;
        if url.ends_with("/enable") {
            self.enabled
                .store(true, std::sync::atomic::Ordering::SeqCst);
            return Ok(json!({"status": 204, "bodyJsonString": ""}));
        }
        let plugin = json!({
            "id": "writing-plugin", "name": "writing",
            "canonical_app_id": "connector_openai_write_like_me",
            "status": "ENABLED", "installation_policy": "AVAILABLE",
            "enabled": self.enabled.load(std::sync::atomic::Ordering::SeqCst),
            "release": {"display_name": "Writing style"}
        });
        let body = if url.starts_with("/ps/plugins/canonical-app/") {
            json!({"plugin_id": "writing-plugin"})
        } else if url.starts_with("/ps/plugins/installed?") {
            json!({
                "plugins": if self.installed { vec![plugin] } else { vec![] },
                "pagination": {"next_page_token": null}
            })
        } else if url == "/ps/plugins/writing-plugin" {
            plugin
        } else {
            anyhow::bail!("unexpected Writing request: {url}");
        };
        Ok(json!({"status": 200, "bodyJsonString": body.to_string()}))
    }
}

#[test]
fn another_dialog_leaves_writing_authorization_retryable() -> Result<()> {
    use ui::screens::settings::personalization::{Event, WritingPlugin, WritingPreference};
    let home = tempfile::tempdir()?;
    let host = std::sync::Arc::new(WritingRaceHost::new(home.path(), false));
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange(
        "plugin/install",
        json!({
            "appsNeedingAuth": [{
                "id": "mail", "name": "Mail", "installUrl": "https://example.invalid/connect"
            }]
        }),
    ));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    fx.keys("cmd-,")?;
    let view = fx.read(|root, cx| root.personal_instructions(cx).unwrap().clone());
    view.update(&mut fx.cx, |view, cx| {
        view.writing_loaded(
            Ok(Some(WritingPreference {
                account_id: "account".into(),
                user_id: "user".into(),
                enabled: false,
                installed: false,
                can_install: true,
                plugin: Some(WritingPlugin {
                    id: "writing-plugin".into(),
                }),
            })),
            cx,
        );
        cx.emit(Event::DeleteMemory);
    });
    fx.wait("memory confirmation", |fx| {
        fx.read(|root, cx| {
            root.dialog()
                .is_some_and(|dialog| dialog.read(cx).kind() == ui::dialog::Kind::DeleteMemories)
        })
    })?;
    view.update(&mut fx.cx, |_, cx| cx.emit(Event::Writing(true)));
    fx.wait("Writing authorization retry", |fx| {
        fx.read(|_, cx| view.read(cx).can_retry_writing())
    })?;
    assert_eq!(
        fx.read(|root, cx| root.dialog().map(|dialog| dialog.read(cx).kind())),
        Some(ui::dialog::Kind::DeleteMemories)
    );
    fx.keys("escape")?;
    assert!(fx.read(|_, cx| view.read(cx).can_retry_writing()));
    assert!(fx.cx.take_opened_url().is_none());
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn writing_setup_preserves_an_existing_instruction_draft() -> Result<()> {
    let home = tempfile::tempdir()?;
    std::fs::write(home.path().join("AGENTS.md"), "Saved instructions.")?;
    let host = std::sync::Arc::new(WritingRaceHost::new(home.path(), true));
    let mut lines = script(None, |_| {})?;
    lines.extend(exchange("skills/list", json!({"data": []})));
    let mut fx = Fx::captured("dark", (1280., 832.), lines)?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    prepare_instructions(&mut fx)?;
    fx.wait("Writing preference", |fx| {
        fx.read(|root, cx| {
            root.personal_instructions(cx)
                .and_then(|view| view.read(cx).writing_preference())
                .is_some()
        })
    })?;
    fx.click((800., 540.))?;
    fx.wait("instructions editor", |fx| {
        fx.read(|root, cx| {
            root.personal_instructions(cx)
                .is_some_and(|view| view.read(cx).editor_focus(cx).is_some())
        })
    })?;
    fx.click((800., 225.))?;
    fx.keys("cmd-a")?;
    fx.type_text("Unsaved instruction draft.")?;
    assert!(fx.read(|root, cx| { root.personal_instructions(cx).unwrap().read(cx).dirty(cx) }));
    let view = fx.read(|root, cx| root.personal_instructions(cx).unwrap().clone());
    view.update(&mut fx.cx, |_, cx| {
        cx.emit(ui::screens::settings::personalization::Event::Writing(true))
    });
    fx.wait("Writing discard confirmation", |fx| {
        fx.read(|root, cx| {
            root.dialog().is_some_and(|dialog| {
                dialog.read(cx).kind() == ui::dialog::Kind::DiscardInstructions
            })
        })
    })?;
    fx.keys("escape")?;
    assert_eq!(fx.route(), app::Route::Settings);
    fx.keys("cmd-a cmd-c")?;
    assert_eq!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
            .as_deref(),
        Some("Unsaved instruction draft.")
    );
    assert_eq!(
        std::fs::read_to_string(home.path().join("AGENTS.md"))?,
        "Saved instructions."
    );
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn quit_from_a_clean_window_protects_another_windows_draft() -> Result<()> {
    use gpui::AppContext as _;
    use std::io::Write as _;
    let home = tempfile::tempdir()?;
    std::fs::write(home.path().join("AGENTS.md"), "Saved instructions.")?;
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    fx.root.update(&mut fx.cx, |root, cx| {
        root.set_host(
            std::sync::Arc::new(app::host::LocalHost::new(home.path())),
            cx,
        )
    });
    open_instructions(&mut fx)?;
    fx.keys("cmd-a")?;
    fx.type_text("Draft in the first window.")?;

    let mut recording = tempfile::NamedTempFile::new()?;
    for line in script(None, |_| {})? {
        writeln!(recording, "{line}")?;
    }
    let (bridge, events) = bridge::Bridge::replay(recording.path())?;
    let second_host = std::sync::Arc::new(app::host::LocalHost::new(home.path().join("second")));
    let second = fx
        .cx
        .open_window(gpui::size(gpui::px(1280.), gpui::px(832.)), |window, cx| {
            let root = cx.new(|cx| {
                app::Root::new(
                    app::State::default(),
                    bridge,
                    events,
                    CWD.into(),
                    window,
                    cx,
                )
            });
            root.update(cx, |root, cx| root.set_host(second_host, cx));
            root
        })?;
    let second_root = second.root(&mut fx.cx)?;
    fx.wait("second window startup", |fx| {
        fx.cx.update(|cx| second_root.read(cx).ready())
    })?;
    second.update(&mut fx.cx, |_, window, cx| {
        window.activate_window();
        window.dispatch_action(Box::new(app::Quit), cx);
    })?;
    fx.wait("first window discard confirmation", |fx| {
        fx.read(|root, cx| {
            root.dialog().is_some_and(|dialog| {
                dialog.read(cx).kind() == ui::dialog::Kind::DiscardInstructions
            })
        })
    })?;
    fx.keys("escape")?;
    fx.keys("cmd-a cmd-c")?;
    assert_eq!(
        fx.cx
            .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
            .as_deref(),
        Some("Draft in the first window.")
    );
    assert!(second.read_with(&fx.cx, |_, _| ()).is_ok());
    assert_eq!(
        std::fs::read_to_string(home.path().join("AGENTS.md"))?,
        "Saved instructions."
    );
    Ok(())
}

/// Serves the ChatGPT account check, notification channels and contact methods.
struct ChannelHost {
    structure: &'static str,
    fail_patch: std::sync::atomic::AtomicBool,
    account_checks: std::sync::atomic::AtomicUsize,
    patches: std::sync::Mutex<Vec<Value>>,
}

impl app::host::Host for ChannelHost {
    fn fetch(&self, request: &app::host::Request) -> Result<Value> {
        let app::host::Request::Http(http) = request else {
            anyhow::bail!("unexpected request");
        };
        let body = match (http["method"].as_str(), http["url"].as_str()) {
            (Some("GET"), Some("/accounts/check/v4-2023-04-27")) => {
                self.account_checks
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                json!({
                    "accounts": { "acct": { "account": { "structure": self.structure } } }
                })
            }
            (Some("GET"), Some("/notifications/settings")) => json!({ "settings": [{
                "category": "tasks", "name": "Tasks", "description": "Task updates",
                "options": [
                    { "channel": "push", "name": "Push", "enabled": true },
                    { "channel": "email", "name": "Email", "enabled": false },
                ],
            }] }),
            (Some("GET"), Some("/me")) => json!({ "email": "me@example.com" }),
            (Some("PATCH"), Some("/notifications/settings")) => {
                self.patches.lock().unwrap().push(serde_json::from_str(
                    http["body"].as_str().unwrap_or("null"),
                )?);
                if self.fail_patch.load(std::sync::atomic::Ordering::SeqCst) {
                    return Ok(json!({ "status": 500, "body": "{}" }));
                }
                json!({})
            }
            _ => anyhow::bail!("unexpected request {http}"),
        };
        Ok(json!({ "status": 200, "body": body.to_string() }))
    }

    fn identity(&self) -> Option<app::host::Identity> {
        Some(app::host::Identity {
            account_id: "acct".into(),
            user_id: "user".into(),
            device_id: "device".into(),
        })
    }
}

fn notification_state(fx: &Fx) -> ui::screens::settings::notifications::State {
    fx.read(|_, cx| {
        cx.try_global::<ui::screens::settings::notifications::State>()
            .cloned()
            .unwrap_or_default()
    })
}

fn channel_enabled(fx: &Fx, channel: &str) -> Option<bool> {
    let ui::screens::computer_use::Load::Ready(settings) = notification_state(fx).channels else {
        return None;
    };
    settings["settings"][0]["options"]
        .as_array()?
        .iter()
        .find(|option| option["channel"] == channel)?["enabled"]
        .as_bool()
}

fn notification_fixture(structure: &'static str) -> Result<(Fx, Arc<ChannelHost>)> {
    let mut fx = Fx::captured("dark", (1280., 832.), script(None, |_| {})?)?;
    let host = Arc::new(ChannelHost {
        structure,
        fail_patch: false.into(),
        account_checks: 0.into(),
        patches: Default::default(),
    });
    let root = fx.root.clone();
    let shared: Arc<dyn app::host::Host> = host.clone();
    fx.cx
        .update(|cx| root.update(cx, |root, cx| root.set_host(shared, cx)));
    // Opening the page starts the account check.
    fx.keys("cmd-,")?;
    fx.click((200., 118.))?;
    fx.type_text("Notifications")?;
    press(&mut fx, "tab enter")?;
    Ok((fx, host))
}

#[test]
fn chatgpt_channels_load_for_a_personal_account_and_roll_back_a_failed_update() -> Result<()> {
    use ui::screens::computer_use::Load;
    use ui::screens::settings::notifications::{Account, Action, dispatch};
    let (mut fx, host) = notification_fixture("personal")?;
    fx.wait("ChatGPT notification channels", |fx| {
        let state = notification_state(fx);
        state.account == Account::Personal
            && matches!(state.channels, Load::Ready(_))
            && matches!(state.contacts, Load::Ready(_))
    })?;
    evidence(&mut fx, "notifications-chatgpt")?;
    let set = |enabled| Action::SetChannel {
        category: "tasks".into(),
        channel: "email".into(),
        enabled,
    };
    fx.cx.update(|cx| dispatch(set(true), cx));
    assert_eq!(channel_enabled(&fx, "email"), Some(true));
    fx.wait("channel update sent", |_| {
        host.patches.lock().unwrap().len() == 1
    })?;
    assert_eq!(
        host.patches.lock().unwrap()[0],
        json!({ "updates": { "tasks": { "email": true } } })
    );

    host.fail_patch
        .store(true, std::sync::atomic::Ordering::SeqCst);
    fx.cx.update(|cx| dispatch(set(false), cx));
    fx.wait("failed channel update toast", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|title| title.as_ref() == "Failed to update ChatGPT notification settings")
        })
    })?;
    assert_eq!(channel_enabled(&fx, "email"), Some(true));
    assert_eq!(channel_enabled(&fx, "push"), Some(true));
    Ok(())
}

#[test]
fn a_workspace_account_hides_the_chatgpt_notification_group() -> Result<()> {
    use ui::screens::settings::notifications::Account;
    let (mut fx, host) = notification_fixture("workspace")?;
    fx.wait("account gate", |fx| {
        let state = notification_state(fx);
        state.account == Account::Hidden
            && !state.fetching
            && host
                .account_checks
                .load(std::sync::atomic::Ordering::SeqCst)
                > 0
    })?;
    assert!(matches!(
        notification_state(&fx).channels,
        ui::screens::computer_use::Load::Loading
    ));
    assert!(host.patches.lock().unwrap().is_empty());
    Ok(())
}
