mod common;

use anyhow::{Context as _, Result, ensure};
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
    let Some(out) = std::env::var_os("CGX_SETTINGS_ACTIONS_EVIDENCE") else {
        return Ok(());
    };
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out)?;
    fx.reference_shot()?.save(out.join(format!("{name}.png")))?;
    Ok(())
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
    lines.extend(exchange("config/read", initial));
    lines.extend(exchange("config/read", enabled));
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

    fn fetch(&self, request: &app::host::Request) -> Result<Value> {
        let app::host::Request::Http(request) = request else {
            anyhow::bail!("unexpected local request");
        };
        let url = request["url"].as_str().context("HTTP URL")?;
        let body = if url.starts_with("/ps/plugins/canonical-app/") {
            json!({"plugin_id": "writing-plugin"})
        } else if url.starts_with("/ps/plugins/installed?") {
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
    }
    lines.extend(exchange(
        "app/list",
        json!({"data": [], "nextCursor": null}),
    ));
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
