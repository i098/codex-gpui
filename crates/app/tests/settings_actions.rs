mod common;

use anyhow::{Result, ensure};
use common::*;
use serde_json::{Value, json};
use theme::ActiveTheme as _;

#[test]
fn proxy() {
    proxy_child();
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
    fx.keys("tab")?;
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
    press(fx, "tab tab")?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn show_instructions(fx: &mut Fx) -> Result<()> {
    prepare_instructions(fx)?;
    press(fx, "enter")
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
    let mut fx = Fx::new()?;
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
fn the_instructions_grip_resizes_the_field_down_to_its_minimum() -> Result<()> {
    let home = tempfile::tempdir()?;
    std::fs::write(home.path().join("AGENTS.md"), "Use spaces.\n")?;
    let host = std::sync::Arc::new(app::host::LocalHost::new(home.path()));
    let mut fx = Fx::new()?;
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
    let mut fx = Fx::new()?;
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
    let mut fx = Fx::new()?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    prepare_instructions(&mut fx)?;
    let key = gpui::Keystroke::parse("enter")?;
    fx.cx.update_window(fx.window, |_, window, cx| {
        window.dispatch_keystroke(key.clone(), cx);
        window.dispatch_event(
            gpui::PlatformInput::KeyUp(gpui::KeyUpEvent { keystroke: key }),
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
