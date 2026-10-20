//! Run with CODEX_GPUI_QA=1 and an owned recorded CODEX_BIN fixture.

#[cfg(not(target_os = "macos"))]
fn main() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    eprintln!("The native window smoke requires macOS.");
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    use anyhow::{Context as _, ensure};
    use app::{
        ConfirmBeforeClose, Root, Route, State,
        windows::{self, Kind, Presentation},
    };
    use gpui::WindowHandle;
    use std::{
        cell::RefCell,
        rc::Rc,
        time::{Duration, Instant},
    };

    const THREAD: &str = "01a13de9-6fff-78e1-84fc-ebb3a2dc873f";
    ensure!(app::qa::ENABLED, "compile this smoke with CODEX_GPUI_QA=1");
    std::env::var_os("CODEX_BIN").context("set CODEX_BIN to an owned fixture executable")?;
    app::qa::prepare()?;
    let result = Rc::new(RefCell::new(None));
    let received = result.clone();
    let oracle: serde_json::Value = serde_json::from_slice(&std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/route1/oracle.json"
    ))?)?;
    let mut settings = app::host_settings::Settings::default();
    settings.write(
        "seen-model-upgrade-list",
        oracle["persisted-atoms"]["state"]["seen-model-upgrade-list"].clone(),
    )?;
    gpui_platform::application().with_assets(theme::Assets).run(move |cx| {
        theme::load_fonts(cx);
        cx.set_global(theme::Theme::dark());
        cx.set_global(settings);
        app::init(cx);
        let state = State {
            route: Route::Thread(THREAD.into()),
            theme: app::ThemeSetting::Dark,
            confirm_before_close: ConfirmBeforeClose::Never,
            ..Default::default()
        };
        let first = windows::open_primary(state, std::env::var("CODEX_GPUI_CWD").unwrap(), cx).unwrap();
        cx.spawn(async move |cx| {
            async fn ready(handle: WindowHandle<Root>, cx: &mut gpui::AsyncApp) -> anyhow::Result<()> {
                let started = Instant::now();
                loop {
                    if handle.update(cx, |root, _, cx| root.ready() && root.thread().is_some_and(|thread| thread.read(cx).id() == THREAD))? {
                        let window: gpui::AnyWindowHandle = handle.into();
                        window.update(cx, |_, window, cx| window.draw(cx).clear(cx))?;
                        return Ok(());
                    }
                    ensure!(started.elapsed() < Duration::from_secs(30), "thread did not load");
                    cx.background_executor().timer(Duration::from_millis(25)).await;
                }
            }
            async fn primary_count(count: usize, cx: &mut gpui::AsyncApp) -> anyhow::Result<Vec<gpui::AnyWindowHandle>> {
                let started = Instant::now();
                loop {
                    let handles = cx.update(|cx| windows::handles(Kind::Primary, cx));
                    if handles.len() == count { return Ok(handles); }
                    ensure!(started.elapsed() < Duration::from_secs(30), "expected {count} primary windows, got {}", handles.len());
                    cx.background_executor().timer(Duration::from_millis(25)).await;
                }
            }
            fn native_visible(window: &gpui::Window) -> bool {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).unwrap().as_raw() else { panic!("not an AppKit window"); };
                unsafe {
                    let view = handle.ns_view.cast::<objc2::runtime::AnyObject>().as_ref();
                    let native: *mut objc2::runtime::AnyObject = objc2::msg_send![view, window];
                    objc2::msg_send![native, isVisible]
                }
            }
            async fn visibility(handle: WindowHandle<Root>, expected: bool, cx: &mut gpui::AsyncApp) -> anyhow::Result<()> {
                let started = Instant::now();
                while handle.update(cx, |_, window, _| native_visible(window))? != expected {
                    ensure!(started.elapsed() < Duration::from_secs(30), "native visibility did not become {expected}");
                    cx.background_executor().timer(Duration::from_millis(25)).await;
                }
                Ok(())
            }
            let outcome = async {
                ready(first, cx).await?;
                first.update(cx, |root, window, cx| {
                    assert!(!window.is_window_active());
                    root.composer(cx).update(cx, |composer, cx| composer.set_text("owner draft", cx));
                    window.dispatch_action(Box::new(windows::OpenThreadWindow), cx);
                })?;
                let second = primary_count(2, cx).await?.into_iter()
                    .find(|handle| handle.window_id() != first.window_id())
                    .unwrap().downcast::<Root>().unwrap();
                ready(second, cx).await?;
                second.update(cx, |root, window, cx| {
                    assert!(!window.is_window_active());
                    assert_eq!(root.route(), &Route::Thread(THREAD.into()));
                    assert_eq!(root.composer(cx).read(cx).field_text(), "");
                    root.composer(cx).update(cx, |composer, cx| composer.set_text("second draft", cx));
                })?;
                let bounds = gpui::Bounds::new(gpui::point(gpui::px(100.), gpui::px(100.)), gpui::size(gpui::px(700.), gpui::px(650.)));
                let placed = cx.update(|cx| windows::set_bounds(second.into(), bounds, cx))?;
                cx.background_executor().timer(Duration::from_millis(100)).await;
                second.update(cx, |_, window, cx| {
                    assert_eq!(window.bounds(), placed);
                    assert_eq!(cx.global::<app::host_settings::Settings>().read_all()["electron-main-window-bounds"]["width"], f32::from(placed.size.width));
                })?;
                let mut invalid = bounds;
                invalid.size.width = gpui::px(f32::NAN);
                assert!(cx.update(|cx| windows::set_bounds(second.into(), invalid, cx)).is_err());
                first.update(cx, |root, window, cx| {
                    assert_eq!(root.composer(cx).read(cx).field_text(), "owner draft");
                    assert!(root.dialog().is_none(), "close is blocked by a dialog");
                    assert!(root.side_panel().is_none(), "close would close a side tab");
                    window.dispatch_action(Box::new(app::CloseWindow), cx);
                })?;
                primary_count(1, cx).await?;
                second.update(cx, |root, window, cx| {
                    assert!(root.dialog().is_none(), "close is blocked by a dialog");
                    assert!(root.side_panel().is_none(), "close would close a side tab");
                    window.dispatch_action(Box::new(app::CloseWindow), cx);
                })?;
                visibility(second, false, cx).await?;
                second.update(cx, |root, window, cx| {
                    assert!(!native_visible(window));
                    assert_eq!(root.composer(cx).read(cx).field_text(), "second draft");
                })?;
                cx.update(|cx| windows::show(second.into(), Presentation::Background, cx))?;
                visibility(second, true, cx).await?;
                second.update(cx, |root, window, cx| {
                    assert!(native_visible(window));
                    assert!(!window.is_window_active());
                    assert_eq!(root.composer(cx).read(cx).field_text(), "second draft");
                })?;
                ensure!(!objc2_app_kit::NSRunningApplication::currentApplication().isActive(), "the smoke activated the application");
                println!("PASS: native Open in new window, independent drafts, close-one survival, last-window hide and background restore");
                anyhow::Ok(())
            }.await;
            *received.borrow_mut() = Some(outcome);
            cx.update(|cx| cx.quit());
        }).detach();
    });
    result
        .borrow_mut()
        .take()
        .context("smoke ended without a result")?
}
