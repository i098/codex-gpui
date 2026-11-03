//! Constructed protocol fixtures, not recorded original Cloud account data.

mod common;

use anyhow::{Context as _, Result, bail, ensure};
use app::host::{Host, Identity, LocalHost, Request};
use base64::Engine as _;
use common::Fx;
use gpui::{AppContext as _, IntoElement, ParentElement as _, Render, Styled as _, div, px, size};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use ui::screens::cloud_task::{Asset, CloudTaskPage, Phase};

const TASK: &str = "cloud-schema-fixture";
const TURN: &str = "assistant-1";
const IMAGE: &str = "file-service://cloud-fixture-image";
const CLONE_URL: &str = "https://github.com/i098/cgx-fixture-scratch.git";
const PATCH: &str = "diff --git a/hello.py b/hello.py\n--- a/hello.py\n+++ b/hello.py\n@@ -1 +1 @@\n-before\n+after\n";

fn detail(case: &str) -> Value {
    let status = match case {
        "waiting" => "pending",
        "setup-stream" => "in_progress",
        "setup-failed" | "chat-failed" | "chat-error" => "failed",
        "cancelled" => "cancelled",
        _ => "completed",
    };
    let completed = status == "completed";
    let image = case == "image" || case == "image-loading";
    let diff = completed && !image;
    let mut input = vec![
        json!({"type":"message","role":"user","content":[{"content_type":"text","text":"Update the greeting in hello.py."}]}),
    ];
    if image {
        input.push(json!({"type":"image_asset_pointer","asset_pointer":IMAGE,"width":64,"height":64,"size_bytes":100}));
    }
    let mut output = Vec::new();
    if completed {
        output.push(json!({"type":"message","role":"assistant","content":[{"content_type":"text","text":"Updated the greeting in `hello.py`."}]}));
    }
    if diff {
        output.push(json!({"type":"output_diff","output_diff":{"diff":PATCH}}));
    }
    let error = match case {
        "setup-failed" => {
            json!({"code":"startup_script_failed","message":"The environment setup script exited with code 1."})
        }
        "chat-error" => json!({"code":"internal_error","message":"The cloud worker disconnected."}),
        _ => Value::Null,
    };
    let assistant = json!({
        "id":TURN,"previous_turn_id":"user-1","turn_status":status,"created_at":1792080001,
        "output_items":output,"error":error,"thread_events":{"events":[]},
        "environment":{"id":"environment-fixture","label":"Fixture environment","repos":["fixture-repo"],"repo_map":{"fixture-repo":{"clone_url":CLONE_URL}}}
    });
    json!({
        "task":{"id":TASK,"title":"Update the greeting","created_at":1792080000},
        "current_user_turn":{"id":"user-1","created_at":1792080000,"input_items":input},
        "current_assistant_turn":assistant,
        "current_diff_task_turn":if diff { assistant.clone() } else { Value::Null }
    })
}

fn wire(status: u16, body: Value) -> Value {
    json!({"value":{"response":{"__response":{"status":status,"body":body.to_string()}}}})
}

struct FixtureHost {
    case: &'static str,
    root: PathBuf,
    local: LocalHost,
    image: Vec<u8>,
    _directory: tempfile::TempDir,
}

impl FixtureHost {
    fn new(case: &'static str) -> Result<Arc<Self>> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("fixture-repository");
        std::fs::create_dir(&root)?;
        let git = |args: &[&str]| -> Result<()> {
            let result = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()?;
            ensure!(
                result.status.success(),
                "fixture Git: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            Ok(())
        };
        git(&["-c", "init.defaultBranch=main", "init"])?;
        git(&["remote", "add", "origin", CLONE_URL])?;
        std::fs::write(root.join("hello.py"), "before\n")?;
        git(&["add", "hello.py"])?;
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.com",
            "commit",
            "-m",
            "Fixture base",
        ])?;
        let pixels = image::RgbaImage::from_pixel(64, 64, image::Rgba([24, 112, 192, 255]));
        let mut image = std::io::Cursor::new(Vec::new());
        pixels.write_to(&mut image, image::ImageFormat::Png)?;
        let local = LocalHost::new(directory.path().join("home"));
        Ok(Arc::new(Self {
            case,
            root,
            local,
            image: image.into_inner(),
            _directory: directory,
        }))
    }
}

impl Host for FixtureHost {
    fn identity(&self) -> Option<Identity> {
        (self.case != "load-error").then(|| Identity {
            account_id: "fixture-account".into(),
            user_id: "fixture-user".into(),
            device_id: "fixture-device".into(),
        })
    }

    fn projects(&self) -> Result<Vec<ui::screens::sidebar::Project>> {
        if matches!(
            self.case,
            "load-error"
                | "inaccessible"
                | "waiting"
                | "setup-stream"
                | "setup-failed"
                | "chat-failed"
                | "chat-error"
                | "cancelled"
        ) {
            return Ok(vec![]);
        }
        Ok(vec![ui::screens::sidebar::Project {
            id: "fixture-project".into(),
            label: "Fixture repository".into(),
            new_chat_label: "New chat".into(),
            root: Some(self.root.to_string_lossy().into_owned().into()),
            worktree: false,
            missing: false,
            expanded: true,
            roots: Vec::new(),
            pinned: false,
        }])
    }

    fn fetch(&self, request: &Request) -> Result<Value> {
        match request {
            Request::Fetch { url, .. }
                if url.ends_with("/apply-patch") && self.case == "partial" =>
            {
                Ok(wire(
                    200,
                    json!({
                        "status":"partial-success","appliedPaths":["hello.py"],"conflictedPaths":["conflict.py"],"skippedPaths":["skipped.py"]
                    }),
                ))
            }
            Request::Fetch { url, .. }
                if url.ends_with("/apply-patch") && self.case == "not-git" =>
            {
                Ok(wire(
                    200,
                    json!({
                        "status":"error","errorCode":"not-git-repo","appliedPaths":[],"conflictedPaths":[],"skippedPaths":[]
                    }),
                ))
            }
            Request::Fetch { .. } => self.local.fetch(request),
            Request::Http(request) => {
                let url = request["url"].as_str().context("fixture URL")?;
                if url == "fixture://image" {
                    return Ok(
                        json!({"value":{"response":{"__response":{"status":200,"base64":base64::engine::general_purpose::STANDARD.encode(&self.image)}}}}),
                    );
                }
                if url.starts_with("/files/download/") {
                    return Ok(wire(200, json!({"download_url":"fixture://image"})));
                }
                if url.ends_with("/logs") {
                    return Ok(wire(
                        200,
                        json!({"logs":[{"key":{"type":"UserSetupScript"},"line":"Installing project dependencies"},{"key":{"type":"UserSetupScript"},"line":"Setup exited with code 1"}]}),
                    ));
                }
                if url.ends_with("/turns") {
                    return Ok(wire(200, json!({"turn_mapping":{}})));
                }
                if url.starts_with("/wham/tasks/") {
                    return if self.case == "inaccessible" {
                        Ok(wire(404, json!({})))
                    } else {
                        Ok(wire(200, detail(self.case)))
                    };
                }
                bail!("unexpected fixture request: {url}")
            }
        }
    }

    fn stream(
        &self,
        _: &Request,
        cancelled: &AtomicBool,
        receive: &mut dyn FnMut(Value) -> bool,
    ) -> Result<()> {
        if self.case == "setup-stream" {
            for (id, line) in [
                ("log-1", "Cloning the repository"),
                ("log-2", "Installing project dependencies"),
            ] {
                if !receive(
                    json!({"id":id,"item_type":"log","key":{"type":"UserSetupScript","created_at":"2026-10-15T17:00:00Z"},"line":line}),
                ) {
                    break;
                }
            }
        }
        // The constructed stream ends while its snapshot remains in progress.
        cancelled.store(true, Ordering::Release);
        Ok(())
    }
}

fn page(fx: &Fx) -> Result<gpui::Entity<CloudTaskPage>> {
    fx.read(|root, _| root.remote_page().cloned())
        .context("cloud page is absent")
}

fn open(case: &'static str) -> Result<(Fx, Arc<FixtureHost>)> {
    let host = FixtureHost::new(case)?;
    let mut fx = Fx::new()?;
    fx.root
        .update(&mut fx.cx, |root, cx| root.set_host(host.clone(), cx));
    let root = fx.root.clone();
    fx.cx.update_window(fx.window, |_, window, cx| {
        root.update(cx, |root, cx| {
            root.open_remote_task(TASK.into(), case == "hotkey", window, cx)
        })
    })?;
    fx.wait("constructed cloud response", |fx| {
        if case == "inaccessible" {
            return fx.route() == app::Route::Home;
        }
        fx.read(|root, cx| {
            root.remote_page().is_some_and(|page| {
                let page = page.read(cx);
                page.data.is_some() || page.error.is_some()
            })
        })
    })?;
    if case == "setup-stream" {
        fx.wait("constructed setup log stream", |fx| {
            fx.read(|root, cx| {
                root.remote_page()
                    .and_then(|page| page.read(cx).data.as_ref())
                    .is_some_and(|data| data.turns[0].logs.len() == 2)
            })
        })?;
    }
    Ok((fx, host))
}

fn capture(fx: &mut Fx, name: &str) -> Result<()> {
    fx.capture_evidence("CLOUD_TASK_CAPTURE_DIR", name)
}

fn activate(fx: &mut Fx) -> Result<()> {
    fx.keys("enter")?;
    fx.input(gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter")?,
    }))
}

#[test]
fn constructed_cloud_states_and_actions() -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    common::install_panic_hook();
    for (case, phase) in [
        ("waiting", Some(Phase::SettingUp)),
        ("setup-stream", Some(Phase::SettingUp)),
        ("setup-failed", Some(Phase::SetupFailed)),
        ("chat-failed", Some(Phase::Failed)),
        ("chat-error", Some(Phase::Failed)),
        ("cancelled", Some(Phase::Cancelled)),
        ("load-error", None),
        ("inaccessible", None),
    ] {
        let (mut fx, _host) = open(case)?;
        if let Some(phase) = phase {
            ensure!(
                page(&fx)?.read_with(&fx.cx, |page, _| page.data.as_ref().unwrap().turns[0].phase
                    == phase),
                "wrong native phase for {case}"
            );
        }
        if case == "load-error" {
            ensure!(
                page(&fx)?.read_with(&fx.cx, |page, _| page.error.is_some()),
                "load error was hidden"
            );
        }
        if case == "inaccessible" {
            ensure!(
                fx.route() == app::Route::Home,
                "inaccessible task did not return Home"
            );
        }
        capture(&mut fx, case)?;
    }

    let (mut fx, host) = open("complete")?;
    let cloud = page(&fx)?;
    capture(&mut fx, "complete")?;
    fx.click((1200., 22.))?;
    ensure!(
        fx.cx.take_opened_url().as_deref()
            == Some("https://chatgpt.com/codex/tasks/cloud-schema-fixture"),
        "Open did not use the original web URL"
    );
    fx.click((1118., 22.))?;
    ensure!(
        cloud.read_with(&fx.cx, |page, _| page.apply_menu_open()),
        "Apply menu did not open"
    );
    capture(&mut fx, "apply-menu")?;
    let focused = fx.cx.update_window(fx.window, |_, window, cx| {
        cloud.read(cx).apply_button_focused(window)
    })?;
    ensure!(
        focused,
        "opening the Apply menu did not focus its primary action"
    );
    activate(&mut fx)?;
    if let Err(error) = fx.wait("real local apply", |fx| {
        std::fs::read_to_string(host.root.join("hello.py")).is_ok_and(|text| text == "after\n")
            && cloud.read_with(&fx.cx, |page, cx| page.is_applied(cx))
    }) {
        capture(&mut fx, "apply-failure")?;
        let state = cloud.read_with(&fx.cx, |page, cx| {
            (
                page.applying,
                page.apply_menu_open(),
                page.is_applied(cx),
                page.apply_results().cloned(),
            )
        });
        bail!("{error}; applying/menu/applied/result: {state:?}");
    }
    ensure!(
        cloud.read_with(&fx.cx, |page, cx| page.is_applied(cx)),
        "successful apply did not switch to Revert"
    );
    capture(&mut fx, "applied")?;
    fx.click((1118., 22.))?;
    activate(&mut fx)?;
    fx.wait("real local revert", |fx| {
        std::fs::read_to_string(host.root.join("hello.py")).is_ok_and(|text| text == "before\n")
            && !cloud.read_with(&fx.cx, |page, cx| page.is_applied(cx))
    })?;
    ensure!(
        !cloud.read_with(&fx.cx, |page, cx| page.is_applied(cx)),
        "successful revert did not clear the applied state"
    );
    capture(&mut fx, "reverted")?;

    for case in ["partial", "not-git"] {
        let (mut fx, _host) = open(case)?;
        let cloud = page(&fx)?;
        fx.click((1118., 22.))?;
        activate(&mut fx)?;
        fx.wait("constructed apply result", |fx| {
            cloud.read_with(&fx.cx, |page, _| page.apply_results().is_some())
        })?;
        capture(&mut fx, case)?;
        fx.keys("escape")?;
        ensure!(
            cloud.read_with(&fx.cx, |page, _| page.apply_results().is_none()),
            "Escape did not close results"
        );
    }

    let (mut fx, _host) = open("image")?;
    let cloud = page(&fx)?;
    fx.wait("fixture image", |fx| {
        cloud.read_with(&fx.cx, |page, _| {
            matches!(page.image_state(IMAGE), Some(Asset::Ready(_)))
        })
    })?;
    capture(&mut fx, "image-closed")?;
    fx.click((1144., 84.))?;
    ensure!(
        cloud.read_with(&fx.cx, |page, _| page.preview_image() == Some(IMAGE)),
        "image preview did not open"
    );
    capture(&mut fx, "image-open")?;
    fx.keys("escape")?;
    ensure!(
        cloud.read_with(&fx.cx, |page, _| page.preview_image().is_none()),
        "Escape did not close image preview"
    );
    capture(&mut fx, "image-closed-again")?;
    let mut response = detail("image-loading");
    response["current_user_turn"]["input_items"][1]["asset_pointer"] =
        "file-service://loading-image".into();
    let data = app::parse_task_response(&response, &json!({}))?;
    let frame = fx.cx.update_window(fx.window, |_, window, cx| {
        cloud.update(cx, |page, cx| page.set_data(data, cx));
        let _ = window.draw(cx);
        window.render_to_image()
    })??;
    if let Some(directory) = std::env::var_os("CLOUD_TASK_CAPTURE_DIR") {
        frame.save(Path::new(&directory).join("image-loading.png"))?;
    }

    let (mut fx, _host) = open("hotkey")?;
    let cloud = page(&fx)?;
    fx.cx
        .update_window(fx.window, |_, window, _| window.remove_window())?;
    struct Compact {
        page: gpui::Entity<CloudTaskPage>,
    }
    impl Render for Compact {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(div().flex_1().min_h_0().child(self.page.clone()))
        }
    }
    let compact = fx.cx.open_window(size(px(470.), px(640.)), |_, cx| {
        cx.new(|_| Compact { page: cloud })
    })?;
    fx.cx.run_until_parked();
    if let Some(directory) = std::env::var_os("CLOUD_TASK_CAPTURE_DIR") {
        fx.cx
            .capture_screenshot(compact.into())?
            .save(Path::new(&directory).join("hotkey-content.png"))?;
    }
    Ok(())
}

#[test]
fn constructed_response_cases_use_the_original_terminal_states() -> Result<()> {
    for (case, phase) in [
        ("waiting", Phase::SettingUp),
        ("setup-failed", Phase::SetupFailed),
        ("chat-failed", Phase::Failed),
        ("cancelled", Phase::Cancelled),
        ("complete", Phase::Chat),
    ] {
        let data = app::parse_task_response(&detail(case), &json!({}))?;
        ensure!(
            data.turns[0].phase == phase,
            "wrong fixture phase for {case}"
        );
    }
    Ok(())
}
