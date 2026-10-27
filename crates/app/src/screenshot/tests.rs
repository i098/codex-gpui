use super::*;
use crate::{Root, State};
use gpui::{HeadlessAppContext, WindowHandle, px, size};
use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(unix)]
#[test]
fn capture_distinguishes_cancel_failure_and_selected_bytes() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir()?;
    let program = directory.path().join("capture");
    for (script, expected) in [
        ("exit 1", "cancel"),
        ("exit 0", "cancel"),
        ("echo denied >&2; exit 1", "failure"),
        ("printf selected > \"$6\"", "image"),
    ] {
        fs::write(&program, format!("#!/bin/sh\n{script}\n"))?;
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700))?;
        let result = run_capture(&program, directory.path());
        match expected {
            "cancel" => assert!(result?.is_none()),
            "failure" => assert!(result.is_err()),
            _ => assert_eq!(result?, Some(b"selected".to_vec())),
        }
    }
    Ok(())
}

#[test]
fn screenshot_routes_and_discards_stale_results() -> Result<()> {
    if std::env::var_os("CGX_SCREENSHOT_CHILD").is_none() {
        let home = tempfile::tempdir()?;
        let result = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "screenshot::tests::screenshot_routes_and_discards_stale_results",
                "--nocapture",
            ])
            .env("CGX_SCREENSHOT_CHILD", "1")
            .env("CODEX_HOME", home.path())
            .status()?;
        ensure!(result.success(), "isolated screenshot check failed");
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101)
    }));
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(project.join("refs/interactions/s017.json"))?)?;
    let oracle: serde_json::Value =
        serde_json::from_slice(&fs::read(project.join("tools/route1/oracle.json"))?)?;
    let mut settings = crate::host_settings::Settings::default();
    settings.write(
        "seen-model-upgrade-list",
        oracle["persisted-atoms"]["state"]["seen-model-upgrade-list"].clone(),
    )?;
    cx.update(|cx| {
        theme::load_fonts(cx);
        cx.set_global(theme::Theme::dark());
        cx.set_global(settings);
        crate::init(cx);
    });
    cx.allow_parking();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut recording = tempfile::NamedTempFile::new()?;
    for name in ["readback.jsonl", "sessions/thread-settings.jsonl"] {
        std::io::copy(
            &mut fs::File::open(fixtures.join(name))?,
            recording.as_file_mut(),
        )?;
    }
    {
        use std::io::Write as _;
        for (index, method) in ["fs/createDirectory", "fs/writeFile", "fs/writeFile"]
            .iter()
            .enumerate()
        {
            let id = 95000 + index;
            writeln!(
                recording,
                "{}",
                serde_json::json!({"dir":"send","msg":{"id":id,"method":method,"params":{}}})
            )?;
            writeln!(
                recording,
                "{}",
                serde_json::json!({"dir":"recv","msg":{"id":id,"result":{}}})
            )?;
        }
    }
    let (bridge, events) = bridge::Bridge::replay(recording.path())?;
    let cwd = std::env::var("CODEX_HOME")?;
    let owner = cx.update(|cx| {
        let mut spec = windows::Spec::new(windows::Kind::Primary, "Screenshot check");
        spec.presentation = windows::Presentation::Hidden;
        spec.bounds = Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
            gpui::point(px(0.), px(0.)),
            size(px(1280.), px(832.)),
        )));
        windows::open(spec, cx, |window, cx| {
            cx.new(|cx| {
                Root::new(
                    State {
                        theme: crate::ThemeSetting::Dark,
                        ..Default::default()
                    },
                    bridge,
                    events,
                    cwd,
                    window,
                    cx,
                )
            })
        })
    })?;
    cx.test_window(owner.into())
        .simulate_scale_factor_change(1.);
    let captured = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let captured_when_ready = captured.clone();
    cx.update(|cx| {
        capture_with(
            Shortcut::ScreenshotCurrent,
            move || {
                captured_when_ready.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(None)
            },
            cx,
        )
    });
    assert!(!captured.load(std::sync::atomic::Ordering::SeqCst));
    settle(&mut cx, owner)?;
    assert!(captured.load(std::sync::atomic::Ordering::SeqCst));
    owner.update(&mut cx, |root, _, _| root.logout_pending = true)?;
    cx.update(|cx| {
        capture_with(
            Shortcut::ScreenshotCurrent,
            || panic!("capture while signed out"),
            cx,
        )
    });
    owner.update(&mut cx, |root, _, _| root.logout_pending = false)?;
    owner.update(&mut cx, |root, _, cx| {
        root.composer(cx)
            .update(cx, |composer, cx| composer.set_text("Keep this draft", cx));
    })?;
    cx.update(|cx| capture_with(Shortcut::ScreenshotCurrent, || Ok(None), cx));
    settle(&mut cx, owner)?;
    owner.update(&mut cx, |root, _, cx| {
        assert_eq!(root.composer(cx).read(cx).field_text(), "Keep this draft")
    })?;
    let old_file = Path::new(&std::env::var_os("CODEX_HOME").unwrap()).join("old-draft.txt");
    fs::write(&old_file, "Keep this file out of the new chat")?;
    owner.update(&mut cx, |root, _, cx| {
        root.composer(cx)
            .update(cx, |composer, cx| composer.drop_files(&[old_file], cx));
    })?;
    let thread = Route::Thread("01a13de9-c48a-7b20-8382-db900881cb99".into());
    owner.update(&mut cx, |root, window, cx| {
        root.navigate(thread.clone(), window, cx)
    })?;
    settle(&mut cx, owner)?;
    owner.update(&mut cx, |root, _, cx| {
        root.composer(cx)
            .update(cx, |composer, cx| composer.set_text("Thread draft", cx));
    })?;
    cx.update(|cx| capture_with(Shortcut::ScreenshotCurrent, || Ok(None), cx));
    settle(&mut cx, owner)?;
    owner.update(&mut cx, |root, _, cx| {
        assert_eq!(root.state.route, thread);
        assert_eq!(root.composer(cx).read(cx).field_text(), "Thread draft");
    })?;
    cx.update(|cx| capture_with(Shortcut::ScreenshotNew, || Ok(None), cx));
    settle(&mut cx, owner)?;
    owner.update(&mut cx, |root, _, cx| {
        assert_eq!(root.state.route, Route::Home);
        assert_eq!(root.composer(cx).read(cx).field_text(), "");
        assert!(
            root.composer(cx).read(cx).restart_ready(),
            "new chat retained an old attachment"
        );
    })?;
    let (sender, receiver) = std::sync::mpsc::channel();
    cx.update(|cx| {
        capture_with(
            Shortcut::ScreenshotCurrent,
            move || receiver.recv().map_err(Into::into),
            cx,
        )
    });
    cx.update(|cx| {
        capture_with(
            Shortcut::ScreenshotNew,
            || panic!("second capture started"),
            cx,
        )
    });
    owner.update(&mut cx, |root, window, cx| root.new_chat(window, cx))?;
    sender.send(Some(vec![1, 2, 3]))?;
    settle(&mut cx, owner)?;
    owner.update(&mut cx, |root, _, cx| {
        assert!(root.composer(cx).read(cx).restart_ready())
    })?;
    let attachments = Path::new(&std::env::var_os("CODEX_HOME").unwrap()).join("attachments");
    assert!(!attachments.exists());
    let image: Vec<u8> =
        serde_json::from_value(original["steps"][1]["clipboard"]["pngBytes"].clone())?;
    let submitted = std::rc::Rc::new(std::cell::RefCell::new(None::<ui::composer::Submission>));
    let submission = submitted.clone();
    owner.update(&mut cx, |root, window, cx| {
        let composer = root.composer(cx);
        composer.update(cx, |composer, _| {
            composer.on_submit(move |value, _, _| {
                *submission.borrow_mut() = Some(value.clone());
            })
        });
        window.focus(&composer.focus_handle(cx), cx);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
            original["steps"][0]["clipboard"]["text"]
                .as_str()
                .unwrap()
                .into(),
        ));
        window.dispatch_action(Box::new(ui::Paste), cx);
    })?;
    let selected = image.clone();
    cx.update(|cx| capture_with(Shortcut::ScreenshotCurrent, move || Ok(Some(selected)), cx));
    settle(&mut cx, owner)?;
    assert!(submitted.borrow().is_none(), "screenshot must not submit");
    let directory = std::env::var_os("CGX_SCREENSHOT_PROOF").map(std::path::PathBuf::from);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        cx.run_until_parked();
        let frame = directory
            .as_ref()
            .map(|_| cx.capture_screenshot(owner.into()))
            .transpose()?;
        owner.update(&mut cx, |_, window, cx| {
            window.dispatch_action(Box::new(ui::composer::Submit), cx);
        })?;
        if let Some(submission) = submitted.borrow_mut().take() {
            assert!(submission.text.is_empty());
            assert_eq!(submission.attachments.len(), 2);
            let path = submission
                .attachments
                .iter()
                .find(|path| path.extension().is_some_and(|extension| extension == "png"))
                .unwrap();
            assert_eq!(fs::read(path)?, image);
            if let (Some(directory), Some(frame)) = (&directory, frame) {
                fs::create_dir_all(directory)?;
                frame.save(directory.join("current-chat.png"))?;
            }
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "screenshot attachment did not persist"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn enlarged_images_keep_their_edge_colors() -> Result<()> {
    use gpui::{Context, IntoElement, ParentElement as _, Render, Styled as _, Window};

    struct Thumbnails(Arc<gpui::Image>);
    impl Render for Thumbnails {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
                .flex()
                .gap(px(6.))
                .child(gpui::img(self.0.clone()).size(px(122.)))
                .child(
                    gpui::div()
                        .layer()
                        .child(gpui::img(self.0.clone()).size(px(122.))),
                )
        }
    }

    let recording: serde_json::Value =
        serde_json::from_str(include_str!("../../../../refs/interactions/s017.json"))?;
    let bytes: Vec<u8> =
        serde_json::from_value(recording["steps"][1]["clipboard"]["pngBytes"].clone())?;
    let image = Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, bytes));
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    let window = cx.open_window(size(px(250.), px(122.)), |_, cx| {
        cx.new(|_| Thumbnails(image))
    })?;
    cx.test_window(window.into())
        .simulate_scale_factor_change(1.);
    cx.allow_parking();
    let deadline = Instant::now() + Duration::from_secs(10);
    let frame = loop {
        cx.run_until_parked();
        let frame = cx.capture_screenshot(window.into())?;
        if frame.get_pixel(30, 30).0 == [255, 0, 0, 255] {
            break frame;
        }
        ensure!(Instant::now() < deadline, "thumbnail did not load");
        std::thread::sleep(Duration::from_millis(1));
    };
    for offset in [0, 128] {
        for (x, y, expected) in [
            (0, 0, [255, 0, 0, 255]),
            (121, 0, [0, 255, 0, 255]),
            (0, 121, [0, 0, 255, 255]),
            (121, 121, [255, 255, 255, 255]),
        ] {
            assert_eq!(
                frame.get_pixel(offset + x, y).0,
                expected,
                "image edge at ({}, {y})",
                offset + x,
            );
        }
    }
    Ok(())
}

fn settle(cx: &mut HeadlessAppContext, owner: WindowHandle<Root>) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        cx.run_until_parked();
        if cx.update(|cx| !cx.global::<Runtime>().capturing)
            && owner.read_with(cx, |root, _| {
                root.ready()
                    && (!matches!(root.state.route, Route::Thread(_)) || root.thread.is_some())
            })?
        {
            return Ok(());
        }
        ensure!(Instant::now() < deadline, "screenshot check timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}
