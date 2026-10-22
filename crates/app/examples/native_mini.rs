#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("The native Mini smoke requires macOS.");
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context as _, ensure};
    use app::{
        Root, State,
        windows::{self, Kind},
    };
    use gpui::WindowHandle;
    use std::{
        cell::RefCell,
        rc::Rc,
        time::{Duration, Instant},
    };

    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    ensure!(app::qa::ENABLED, "compile with CODEX_GPUI_QA=1");
    std::env::var_os("CODEX_BIN").context("set an owned recorded CODEX_BIN executable")?;
    app::qa::prepare()?;
    let output = std::env::var_os("CGX_MINI_EVIDENCE").map(std::path::PathBuf::from);
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
        let primary = windows::open_primary(State { theme: app::ThemeSetting::Dark, ..Default::default() }, std::env::var("CODEX_GPUI_CWD").unwrap(), cx).unwrap();
        cx.spawn(async move |cx| {
            async fn ready(handle: WindowHandle<Root>, cx: &mut gpui::AsyncApp) -> anyhow::Result<()> {
                let started = Instant::now();
                while !handle.update(cx, |root, _, _| root.ready())? {
                    ensure!(started.elapsed() < Duration::from_secs(30), "Mini startup timed out");
                    cx.background_executor().timer(Duration::from_millis(25)).await;
                }
                Ok(())
            }
            async fn visible(handle: WindowHandle<Root>, expected: bool, cx: &mut gpui::AsyncApp) -> anyhow::Result<()> {
                let started = Instant::now();
                while cx.update(|cx| windows::is_visible(handle.into(), cx))? != expected {
                    ensure!(started.elapsed() < Duration::from_secs(30), "Mini visibility did not become {expected}");
                    cx.background_executor().timer(Duration::from_millis(25)).await;
                }
                Ok(())
            }
            let outcome = async {
                ready(primary, cx).await?;
                primary.update(cx, |root, window, cx| {
                    root.composer(cx).update(cx, |composer, cx| composer.set_text("Keep the primary draft", cx));
                    window.dispatch_action(Box::new(ui::OpenMini), cx);
                })?;
                let started = Instant::now();
                let mini = loop {
                    if let Some(handle) = cx.update(|cx| windows::handles(Kind::AvatarOverlay, cx)).into_iter().next() {
                        break handle.downcast::<Root>().context("Mini root type changed")?;
                    }
                    ensure!(started.elapsed() < Duration::from_secs(30), "Mini did not open");
                    cx.background_executor().timer(Duration::from_millis(25)).await;
                };
                ready(mini, cx).await?;
                visible(mini, true, cx).await?;
                if let Some(output) = &output {
                    std::fs::create_dir_all(output)?;
                    gpui::AnyWindowHandle::from(mini).update(cx, |_, window, cx| {
                        window.draw(cx).clear(cx);
                        window.render_to_image()?.save(output.join("native-mini-pet.png"))?;
                        anyhow::Ok(())
                    })??;
                }
                mini.update(cx, |root, window, cx| {
                    assert!(!window.is_window_active());
                    assert_eq!(root.composer(cx).read(cx).field_text(), "");
                    root.composer(cx).update(cx, |composer, cx| composer.set_text("Keep the Mini draft", cx));
                    window.dispatch_action(Box::new(app::hotkey::OpenMiniFromHotkey), cx);
                })?;
                cx.background_executor().timer(Duration::from_millis(100)).await;
                if let Some(output) = &output {
                    std::fs::create_dir_all(output)?;
                    gpui::AnyWindowHandle::from(mini).update(cx, |_, window, cx| {
                        ui::composer::Composer::set_caret_transparent(cx, true);
                        window.draw(cx).clear(cx);
                        window.render_to_image()?.save(output.join("native-mini-expanded.png"))?;
                        anyhow::Ok(())
                    })??;
                }
                primary.update(cx, |_, window, cx| window.dispatch_action(Box::new(ui::OpenMini), cx))?;
                visible(mini, false, cx).await?;
                primary.update(cx, |_, window, cx| window.dispatch_action(Box::new(ui::OpenMini), cx))?;
                visible(mini, true, cx).await?;
                assert_eq!(cx.update(|cx| windows::handles(Kind::AvatarOverlay, cx)).len(), 1);
                mini.update(cx, |root, _, cx| assert_eq!(root.composer(cx).read(cx).field_text(), "Keep the Mini draft"))?;
                primary.update(cx, |root, window, cx| {
                    assert_eq!(root.composer(cx).read(cx).field_text(), "Keep the primary draft");
                    assert!(!window.is_window_active());
                    assert_eq!(cx.global::<app::host_settings::Settings>().read_all()["electron-avatar-overlay-open"], true);
                })?;
                ensure!(!objc2_app_kit::NSRunningApplication::currentApplication().isActive(), "Mini smoke activated the application");
                println!("PASS: Mini opens without activation, preserves independent drafts, hides, and reuses its native window; no audio capture");
                anyhow::Ok(())
            }.await;
            *received.borrow_mut() = Some(outcome);
            cx.update(|cx| cx.quit());
        }).detach();
    });
    result
        .borrow_mut()
        .take()
        .context("Mini smoke ended without a result")?
}
