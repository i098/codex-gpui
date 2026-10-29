#[cfg(not(target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("native_lifecycle requires macOS")
}

#[cfg(target_os = "macos")]
mod native {
    use anyhow::{Context as _, Result, ensure};
    use app::{Root, State, windows};
    use gpui::{AsyncApp, Focusable as _, Keystroke, MenuItem, Window, WindowHandle};
    use objc2::{
        class, msg_send,
        runtime::{AnyObject, Imp, Method, Sel},
        sel,
    };
    use objc2_foundation::{NSRect, NSSize};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{
        cell::Cell,
        rc::Rc,
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant},
    };

    static HIDE_OTHERS: AtomicUsize = AtomicUsize::new(0);
    static SHOW_ALL: AtomicUsize = AtomicUsize::new(0);

    // These two selectors must not change other applications during the smoke.
    struct OtherApps([(&'static Method, Imp); 2]);

    impl OtherApps {
        fn intercept() -> Result<Self> {
            unsafe extern "C" fn record(_: &AnyObject, selector: Sel, _: *mut AnyObject) {
                if selector == sel!(hideOtherApplications:) {
                    HIDE_OTHERS.fetch_add(1, Ordering::Relaxed);
                } else if selector == sel!(unhideAllApplications:) {
                    SHOW_ALL.fetch_add(1, Ordering::Relaxed);
                }
            }
            let hide = class!(NSApplication)
                .instance_method(sel!(hideOtherApplications:))
                .context("missing hideOtherApplications:")?;
            let show = class!(NSApplication)
                .instance_method(sel!(unhideAllApplications:))
                .context("missing unhideAllApplications:")?;
            // Both AppKit methods have the same void(id, SEL, id) ABI as record.
            let replacement = unsafe {
                std::mem::transmute::<unsafe extern "C" fn(&AnyObject, Sel, *mut AnyObject), Imp>(
                    record,
                )
            };
            Ok(Self(unsafe {
                [
                    (hide, hide.set_implementation(replacement)),
                    (show, show.set_implementation(replacement)),
                ]
            }))
        }
    }

    impl Drop for OtherApps {
        fn drop(&mut self) {
            for (method, original) in self.0 {
                // Restore only this smoke process's AppKit method table.
                unsafe {
                    method.set_implementation(original);
                }
            }
        }
    }

    #[derive(Debug)]
    struct Snapshot {
        number: isize,
        minimized: bool,
        zoomed: bool,
        visible: bool,
        minimum: NSSize,
        content: NSRect,
    }

    fn snapshot(window: &Window) -> Result<Snapshot> {
        let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window)?.as_raw()
        else {
            anyhow::bail!("not an AppKit window");
        };
        // GPUI owns the view and NSWindow throughout this main-thread update.
        unsafe {
            let view = handle.ns_view.cast::<AnyObject>().as_ref();
            let native: *mut AnyObject = msg_send![view, window];
            let native = native.as_ref().context("missing NSWindow")?;
            let frame: NSRect = msg_send![native, frame];
            let key: bool = msg_send![native, isKeyWindow];
            ensure!(!key, "the owned window became the key window");
            Ok(Snapshot {
                number: msg_send![native, windowNumber],
                minimized: msg_send![native, isMiniaturized],
                zoomed: msg_send![native, isZoomed],
                visible: msg_send![native, isVisible],
                minimum: msg_send![native, contentMinSize],
                content: msg_send![native, contentRectForFrameRect: frame],
            })
        }
    }

    fn frontmost() -> i32 {
        unsafe {
            let workspace: &AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
            let front: &AnyObject = msg_send![workspace, frontmostApplication];
            msg_send![front, processIdentifier]
        }
    }

    fn background(expected_front: i32) -> Result<()> {
        ensure!(
            !objc2_app_kit::NSRunningApplication::currentApplication().isActive(),
            "smoke activated its application"
        );
        let current = frontmost();
        if current != expected_front {
            println!(
                "Other application changed in front: {expected_front} -> {current}; smoke remains inactive"
            );
        }
        Ok(())
    }

    fn menu(name: &str, item: &str) -> Result<Box<dyn gpui::Action>> {
        app::menus()
            .into_iter()
            .filter(|menu| menu.name == name)
            .flat_map(|menu| menu.items)
            .find_map(|entry| match entry {
                MenuItem::Action { name, action, .. } if name == item => Some(action),
                _ => None,
            })
            .with_context(|| format!("missing menu {name} > {item}"))
    }

    async fn wait(
        handle: WindowHandle<Root>,
        cx: &mut AsyncApp,
        label: &str,
        predicate: impl Fn(&Snapshot) -> bool,
    ) -> Result<Snapshot> {
        let started = Instant::now();
        loop {
            let state = handle.update(cx, |_, window, _| snapshot(window))??;
            if predicate(&state) {
                println!("PASS {label}: {state:?}");
                return Ok(state);
            }
            ensure!(
                started.elapsed() < Duration::from_secs(10),
                "{label} timed out: {state:?}"
            );
            cx.background_executor()
                .timer(Duration::from_millis(25))
                .await;
        }
    }

    fn focus(handle: WindowHandle<Root>, cx: &mut AsyncApp) -> Result<()> {
        let focus = handle.update(cx, |root, _, cx| root.composer(cx).focus_handle(cx))?;
        gpui::AnyWindowHandle::from(handle).update(cx, |_, window, cx| {
            focus.focus(window, cx);
            window.draw(cx).clear(cx);
        })?;
        Ok(())
    }

    pub fn run() -> Result<()> {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        ensure!(
            std::env::var("CGX_NATIVE_LIFECYCLE_ISOLATED").is_ok_and(|value| value == "1"),
            "native_lifecycle creates visible windows; use only an isolated GUI session and set CGX_NATIVE_LIFECYCLE_ISOLATED=1"
        );
        ensure!(app::qa::ENABLED, "compile with CODEX_GPUI_QA=1");
        std::env::var_os("CODEX_BIN").context("set CODEX_BIN to the real codex executable")?;
        app::qa::prepare()?;
        let _other_apps = OtherApps::intercept()?;
        let front = frontmost();
        let use_menus = std::env::var_os("CGX_LIFECYCLE_MENUS").is_some();
        let completed = Rc::new(Cell::new(false));
        let received = completed.clone();
        gpui_platform::application().with_assets(theme::Assets).run(move |cx| {
            theme::load_fonts(cx);
            cx.set_global(theme::Theme::dark());
            app::init(cx);
            cx.on_app_quit(move |_| {
                assert!(completed.get(), "application quit before the checks completed");
                background(front).unwrap();
                println!("PASS Quit key/menu: the real handler reached application shutdown");
                async {}
            }).detach();
            let handle = windows::open_primary(State::default(), std::env::var("CODEX_GPUI_CWD").unwrap(), cx).unwrap();
            let surface = gpui::AnyWindowHandle::from(handle);
            cx.spawn(async move |cx| {
                let outcome = async {
                    let started = Instant::now();
                    while !handle.update(cx, |root, _, _| root.ready())? {
                        ensure!(started.elapsed() < Duration::from_secs(30), "real app-server initialization timed out");
                        cx.background_executor().timer(Duration::from_millis(25)).await;
                    }
                    focus(handle, cx)?;
                    background(front)?;
                    let initial = wait(handle, cx, "native window ready", |state| state.visible).await?;
                    ensure!(initial.minimum == NSSize::new(480., 600.), "wrong native content minimum");
                    let bounds = gpui::Bounds::new(gpui::point(gpui::px(50.), gpui::px(50.)), gpui::size(gpui::px(200.), gpui::px(200.)));
                    cx.update(|cx| windows::set_bounds(handle.into(), bounds, cx))?;
                    wait(handle, cx, "minimum resize", |state| state.content.size.width >= 480. && state.content.size.height >= 600.).await?;
                    background(front)?;

                    surface.update(cx, |_, window, cx| {
                        if use_menus {
                            window.dispatch_action(menu("Window", "Minimize").unwrap(), cx);
                        } else {
                            window.dispatch_keystroke(Keystroke::parse("cmd-m").unwrap(), cx);
                        }
                    })?;
                    wait(handle, cx, "minimize key/menu", |state| state.minimized).await?;
                    cx.update(|cx| windows::show(handle.into(), windows::Presentation::Background, cx))?;
                    wait(handle, cx, "background restore", |state| !state.minimized && state.visible).await?;
                    focus(handle, cx)?;
                    background(front)?;

                    surface.update(cx, |_, window, cx| window.dispatch_action(menu("Window", "Zoom").unwrap(), cx))?;
                    wait(handle, cx, "menu zooms", |state| state.zoomed).await?;
                    surface.update(cx, |_, window, cx| window.dispatch_action(menu("Window", "Zoom").unwrap(), cx))?;
                    wait(handle, cx, "menu restores zoom", |state| !state.zoomed).await?;
                    background(front)?;

                    surface.update(cx, |_, window, cx| { window.dispatch_keystroke(Keystroke::parse("alt-cmd-h").unwrap(), cx); })?;
                    surface.update(cx, |_, window, cx| window.dispatch_action(menu("Codex", "Hide Others").unwrap(), cx))?;
                    surface.update(cx, |_, window, cx| window.dispatch_action(menu("Codex", "Show All").unwrap(), cx))?;
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                    ensure!(HIDE_OTHERS.load(Ordering::Relaxed) == 2, "Hide Others did not reach its AppKit selector twice");
                    ensure!(SHOW_ALL.load(Ordering::Relaxed) == 1, "Show All did not reach its AppKit selector");
                    println!("PASS Hide Others key/menu and Show All menu: AppKit calls intercepted; other apps unchanged");
                    background(front)?;

                    surface.update(cx, |_, window, cx| {
                        if use_menus {
                            window.dispatch_action(menu("Codex", "Hide Codex").unwrap(), cx);
                        } else {
                            window.dispatch_keystroke(Keystroke::parse("cmd-h").unwrap(), cx);
                        }
                    })?;
                    let hidden = wait(handle, cx, "hide owned window", |state| !state.visible).await?;
                    ensure!(hidden.number == initial.number, "window identity changed");
                    background(front)?;
                    println!("PASS all native states retained NSWindow {} and frontmost PID {front}", initial.number);
                    Ok(())
                }.await;
                if let Err(error) = outcome {
                    eprintln!("FAIL: {error:#}");
                    std::process::exit(101);
                }
                received.set(true);
                focus(handle, cx).unwrap();
                    // The event loop must return through the application's real guarded Quit action.
                    cx.spawn(async move |cx| {
                        cx.background_executor().timer(Duration::from_secs(10)).await;
                        eprintln!("FAIL: Quit did not stop the native event loop");
                        std::process::exit(101);
                    }).detach();
                    surface.update(cx, |_, window, cx| {
                        if use_menus {
                            window.dispatch_action(menu("Codex", "Quit Codex").unwrap(), cx);
                        } else {
                            window.dispatch_keystroke(Keystroke::parse("cmd-q").unwrap(), cx);
                        }
                    }).unwrap();
            }).detach();
        });
        anyhow::bail!("the event loop returned without native application termination")
    }
}

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    native::run()
}
