use gpui::{
    AnyWindowHandle, AppContext as _, Context, Entity, Focusable, HeadlessAppContext, IntoElement,
    KeyUpEvent, Keystroke, ParentElement, PlatformInput, Render, Styled, Window, div, px, size,
};
use std::{cell::RefCell, rc::Rc, sync::Arc};
use ui::{
    Shell,
    composer::{Composer, EnterBehavior, Preferences},
    screens::settings::{self, SettingsPage},
};

fn context() -> HeadlessAppContext {
    #[cfg(target_os = "macos")]
    {
        static HOOK: std::sync::Once = std::sync::Once::new();
        HOOK.call_once(|| {
            std::panic::set_hook(Box::new(|info| {
                eprintln!("{info}\n{}", std::backtrace::Backtrace::force_capture());
                std::process::exit(101);
            }))
        });
    }
    let mut context = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    context.update(|cx| {
        cx.set_global(theme::Theme::dark());
        theme::load_fonts(cx);
        ui::composer::bind_keys(cx);
        #[cfg(target_os = "macos")]
        if std::env::var_os("CGX_GENERAL_EVIDENCE").is_some() {
            Composer::set_caret_transparent(cx, true);
        }
    });
    context
}

fn press(cx: &mut HeadlessAppContext, window: AnyWindowHandle, key: &str) {
    cx.update_window(window, |_, window, cx| {
        window.dispatch_keystroke(Keystroke::parse(key).unwrap(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.dispatch_event(
            PlatformInput::KeyUp(KeyUpEvent {
                keystroke: Keystroke::parse(key).unwrap(),
            }),
            cx,
        );
    })
    .unwrap();
    present(cx, window, std::time::Duration::ZERO);
}

fn present(cx: &mut HeadlessAppContext, window: AnyWindowHandle, duration: std::time::Duration) {
    cx.advance_clock(duration);
    for _ in 0..3 {
        cx.test_window(window)
            .simulate_frame_request(gpui::RequestFrameOptions {
                require_presentation: true,
                ..Default::default()
            });
        cx.run_until_parked();
    }
}

#[cfg(target_os = "macos")]
fn click(cx: &mut HeadlessAppContext, window: AnyWindowHandle, x: f32, y: f32) {
    use gpui::{MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, point};
    let position = point(px(x), px(y));
    cx.update_window(window, |_, window, cx| {
        window.dispatch_event(
            PlatformInput::MouseMove(MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
            }),
            cx,
        );
    })
    .unwrap();
    present(cx, window, std::time::Duration::ZERO);
}

#[cfg(target_os = "macos")]
fn capture(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    path: &std::path::Path,
    name: &str,
) {
    cx.update_window(window, |_, window, cx| {
        window.dispatch_event(
            PlatformInput::MouseMove(gpui::MouseMoveEvent {
                position: gpui::point(px(768.), px(6.)),
                pressed_button: None,
                modifiers: Default::default(),
            }),
            cx,
        );
    })
    .unwrap();
    present(cx, window, std::time::Duration::from_millis(500));
    cx.capture_screenshot(window)
        .unwrap()
        .save(path.join(name))
        .unwrap();
}

#[test]
fn send_shortcut_uses_the_saved_mode_for_plain_and_multiline_drafts() {
    for (behavior, draft, key, sends) in [
        (EnterBehavior::Enter, "one", "enter", true),
        (EnterBehavior::Enter, "one", "cmd-enter", false),
        (EnterBehavior::CmdIfMultiline, "one", "enter", true),
        (EnterBehavior::CmdIfMultiline, "one\ntwo", "enter", false),
        (EnterBehavior::CmdIfMultiline, "one\ntwo", "cmd-enter", true),
        (EnterBehavior::CmdAlways, "one", "enter", false),
        (EnterBehavior::CmdAlways, "one", "cmd-enter", true),
    ] {
        let mut cx = context();
        let sent = Rc::new(RefCell::new(Vec::new()));
        let received = sent.clone();
        cx.update(|cx| {
            cx.set_global(Preferences {
                enter: behavior,
                ..Default::default()
            })
        });
        let window = cx
            .open_window(size(px(640.), px(480.)), |window, cx| {
                let composer = cx.new(|cx| Composer::home(window, cx));
                let models: serde_json::Value =
                    ui::fixture::response("readback.jsonl", "model/list", 0);
                let models: Vec<bridge::types::Model> =
                    serde_json::from_value(models["data"].clone()).unwrap();
                composer.update(cx, |composer, cx| {
                    composer.set_models(&models, &serde_json::json!({}), cx);
                    composer.set_text(draft, cx);
                    composer.on_submit(move |submission, _, _| {
                        received.borrow_mut().push(submission.text.clone())
                    });
                });
                composer.focus_handle(cx).focus(window, cx);
                composer
            })
            .unwrap();
        cx.run_until_parked();
        press(&mut cx, window.into(), key);
        if sends {
            assert_eq!(*sent.borrow(), [draft], "{behavior:?} {key}");
        } else {
            assert!(sent.borrow().is_empty(), "{behavior:?} {key}");
            cx.update_window(window.into(), |view, _, cx| {
                let composer = view.downcast::<Composer>().unwrap();
                assert_eq!(composer.read(cx).field_text(), format!("{draft}\n"));
            })
            .unwrap();
        }
    }
}

#[test]
fn numeric_field_steps_within_bounds_and_rejects_invalid_saved_values() {
    let mut cx = context();
    let window = cx
        .open_window(size(px(400.), px(200.)), |window, cx| {
            let field = cx.new(|cx| Composer::settings_number(4., 0., 5., cx));
            field.focus_handle(cx).focus(window, cx);
            field
        })
        .unwrap();
    cx.run_until_parked();
    for key in ["up", "up"] {
        press(&mut cx, window.into(), key);
    }
    cx.update_window(window.into(), |view, _, cx| {
        let field = view.downcast::<Composer>().unwrap();
        assert_eq!(field.read(cx).number_value(), Some(5.));
        field.update(cx, |field, cx| field.set_text("6", cx));
        assert_eq!(field.read(cx).number_value(), None);
        field.update(cx, |field, cx| field.set_text("NaN", cx));
        assert_eq!(field.read(cx).number_value(), None);
    })
    .unwrap();
}

struct Screen {
    shell: Entity<Shell>,
    page: Entity<SettingsPage>,
}
impl Render for Screen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .child(self.shell.clone())
            .children(self.page.read(cx).modal())
    }
}

fn settings_fixture(
    section: Option<&'static str>,
    cx: &mut gpui::App,
) -> (Entity<Shell>, Entity<SettingsPage>) {
    let config: serde_json::Value = ui::fixture::response("readback.jsonl", "config/read", 0);
    let skills: serde_json::Value = ui::fixture::response("readback.jsonl", "skills/list", 0);
    let (shell, page) = settings::shell(&config["config"], &skills, section, cx);
    shell.update(cx, |shell, _| shell.can_go_back = true);
    page.update(cx, |page, cx| {
        page.set_targets(
            Ok(ui::fixture::response(
                "settings-general.jsonl",
                "openIn.getTargets",
                0,
            )),
            cx,
        );
        page.set_projectless_root(Ok("/Users/user/Documents/Codex".into()), cx);
        page.set_requirements(
            Ok(ui::fixture::response(
                "readback.jsonl",
                "configRequirements/read",
                0,
            )),
            true,
            cx,
        );
    });
    (shell, page)
}

#[test]
fn settings_navigation_keeps_unsaved_changes_until_discard_is_confirmed() {
    let mut cx = context();
    let page_ref = Rc::new(RefCell::new(None));
    let saved = page_ref.clone();
    let window = cx
        .open_window(size(px(1280.), px(832.)), |_, cx| {
            let (shell, page) = settings_fixture(None, cx);
            *saved.borrow_mut() = Some(page.clone());
            cx.new(|cx| {
                cx.observe(&page, |_, _, cx| cx.notify()).detach();
                Screen { shell, page }
            })
        })
        .unwrap();
    present(&mut cx, window.into(), std::time::Duration::ZERO);
    #[cfg(target_os = "macos")]
    if let Some(path) = std::env::var_os("CGX_GENERAL_EVIDENCE") {
        std::fs::create_dir_all(&path).unwrap();
        cx.capture_screenshot(window.into())
            .unwrap()
            .save(std::path::Path::new(&path).join("settings-default.png"))
            .unwrap();
        let path = std::path::Path::new(&path);
        click(&mut cx, window.into(), 1090., 658.);
        capture(&mut cx, window.into(), path, "settings-language-menu.png");
        for key in ["F", "r", "e", "n", "c", "h"] {
            press(&mut cx, window.into(), key);
        }
        capture(&mut cx, window.into(), path, "settings-language-filter.png");
        press(&mut cx, window.into(), "escape");
        click(&mut cx, window.into(), 1139., 356.);
        click(&mut cx, window.into(), 1139., 356.);
        capture(
            &mut cx,
            window.into(),
            path,
            "settings-full-access-dialog.png",
        );
        press(&mut cx, window.into(), "escape");
    }
    let page: Entity<SettingsPage> = page_ref.borrow().as_ref().unwrap().clone();
    cx.update_window(window.into(), |_, window, cx| {
        page.update(cx, |page, cx| page.focus_search(window, cx))
    })
    .unwrap();
    for key in ["down", "down", "enter"] {
        press(&mut cx, window.into(), key);
    }
    cx.update(|cx| {
        assert_eq!(page.read(cx).route().section, "Notifications");
        page.update(cx, |page, cx| page.set_unsaved(true, cx));
    });
    for key in ["down", "enter", "escape"] {
        press(&mut cx, window.into(), key);
    }
    cx.update(|cx| {
        assert_eq!(page.read(cx).route().section, "Notifications");
        assert!(page.read(cx).has_unsaved_changes());
    });
    for key in ["enter", "tab", "enter"] {
        press(&mut cx, window.into(), key);
    }
    cx.update(|cx| {
        assert_eq!(page.read(cx).route().section, "Import");
        assert!(!page.read(cx).has_unsaved_changes());
    });
    #[cfg(target_os = "macos")]
    if let Some(path) = std::env::var_os("CGX_GENERAL_EVIDENCE") {
        std::fs::create_dir_all(&path).unwrap();
        cx.capture_screenshot(window.into())
            .unwrap()
            .save(std::path::Path::new(&path).join("settings-navigation.png"))
            .unwrap();
    }
}

#[test]
fn settings_search_waits_for_a_highlight_then_opens_the_matching_panel() {
    let mut cx = context();
    let page_ref = Rc::new(RefCell::new(None));
    let saved = page_ref.clone();
    let window = cx
        .open_window(size(px(1280.), px(832.)), |_, cx| {
            let (shell, page) = settings_fixture(Some("Git"), cx);
            *saved.borrow_mut() = Some(page.clone());
            cx.new(|cx| {
                cx.observe(&page, |_, _, cx| cx.notify()).detach();
                Screen { shell, page }
            })
        })
        .unwrap();
    present(&mut cx, window.into(), std::time::Duration::ZERO);
    let page: Entity<SettingsPage> = page_ref.borrow().as_ref().unwrap().clone();
    for key in ["s", "p", "e", "e", "d", "enter"] {
        press(&mut cx, window.into(), key);
    }
    cx.update(|cx| assert_eq!(page.read(cx).route().section, "Git"));
    for key in ["down", "enter"] {
        press(&mut cx, window.into(), key);
    }
    cx.update(|cx| assert_eq!(page.read(cx).route().section, "General"));
    present(
        &mut cx,
        window.into(),
        std::time::Duration::from_millis(100),
    );
    #[cfg(target_os = "macos")]
    if let Some(path) = std::env::var_os("CGX_GENERAL_EVIDENCE") {
        std::fs::create_dir_all(&path).unwrap();
        cx.capture_screenshot(window.into())
            .unwrap()
            .save(std::path::Path::new(&path).join("settings-search-speed.png"))
            .unwrap();
        let path = std::path::Path::new(&path);
        capture(
            &mut cx,
            window.into(),
            path,
            "settings-search-speed-settled.png",
        );
        click(&mut cx, window.into(), 1096., 468.);
        capture(&mut cx, window.into(), path, "settings-speed-menu.png");
        press(&mut cx, window.into(), "escape");
        click(&mut cx, window.into(), 1131., 589.);
        cx.update(|cx| assert_eq!(page.read(cx).route().section, "Open source licenses"));
        capture(
            &mut cx,
            window.into(),
            path,
            "settings-licenses-loading.png",
        );
    }
}
