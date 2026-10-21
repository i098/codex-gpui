#![cfg(target_os = "macos")]

use anyhow::{Result, ensure};
use bridge::Bridge;
use std::{fs, path::Path};

const RECORDING: &str = include_str!("../../../fixtures/sessions/archived-chats.jsonl");

#[test]
fn native_archived_settings_restores_and_deletes_chats() -> Result<()> {
    use app::{Root, Route, State, ThemeSetting};
    use gpui::{
        AppContext as _, HeadlessAppContext, Keystroke, Modifiers, MouseButton, MouseDownEvent,
        MouseUpEvent, PlatformInput, point, px, size,
    };
    use std::{
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };
    use ui::screens::settings::archived_chats::OpenArchivedChats;

    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let file = tempfile::NamedTempFile::new()?;
    let mut script = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/readback.jsonl"),
    )?;
    script.push('\n');
    script.push_str(RECORDING);
    fs::write(file.path(), script)?;
    let (bridge, events) = Bridge::replay(file.path())?;
    let home = tempfile::tempdir()?;
    let host = Arc::new(app::host::LocalHost::new(home.path()));
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("../../../tools/route1/oracle.json"))?;
    let mut settings = app::host_settings::Settings::default();
    settings.write(
        "seen-model-upgrade-list",
        oracle["persisted-atoms"]["state"]["seen-model-upgrade-list"].clone(),
    )?;
    let mut cx = HeadlessAppContext::with_platform(
        textsys::text_system(),
        Arc::new(theme::Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        theme::load_fonts(cx);
        cx.set_global(theme::Theme::dark());
        cx.set_global(settings);
        ui::composer::Composer::set_caret_transparent(cx, true);
        app::init(cx);
    });
    cx.allow_parking();
    let window = cx.open_window(size(px(1280.), px(832.)), |window, cx| {
        cx.new(|cx| {
            let mut root = Root::new(
                State {
                    route: Route::Home,
                    theme: ThemeSetting::Dark,
                    ..State::default()
                },
                bridge,
                events,
                "/tmp/cgx-mac/proj".into(),
                window,
                cx,
            );
            root.set_host(host, cx);
            root
        })
    })?;
    let root = cx.read_window(&window, |root, _| root)?;
    let until = Instant::now() + Duration::from_secs(30);
    while !root.read_with(&cx, |root, _| root.ready()) {
        cx.run_until_parked();
        ensure!(Instant::now() < until, "application startup did not finish");
        thread::sleep(Duration::from_millis(5));
    }
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_action(Box::new(OpenArchivedChats), cx);
    })?;
    let until = Instant::now() + Duration::from_secs(30);
    let page = loop {
        cx.run_until_parked();
        let page = root.read_with(&cx, |root, cx| {
            root.archived_settings()
                .filter(|page| matches!(page.read(cx).threads, Some(Ok(_))))
                .cloned()
        });
        if let Some(page) = page {
            break page;
        }
        ensure!(Instant::now() < until, "archived list did not load");
        thread::sleep(Duration::from_millis(5));
    };
    let ids: Vec<_> = page.read_with(&cx, |page, _| {
        page.threads
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .iter()
            .map(|thread| thread.id.clone())
            .collect()
    });
    ensure!(ids.len() == 3);
    let shots = std::env::var("CGX_ARCHIVE_SHOTS").ok();
    if let Some(directory) = &shots {
        fs::create_dir_all(directory)?;
    }
    let click = |cx: &mut HeadlessAppContext, x: f32, y: f32| -> Result<()> {
        cx.update_window(window.into(), |_, window, cx| {
            let position = point(px(x), px(y));
            window.dispatch_event(
                PlatformInput::MouseMove(gpui::MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }),
                cx,
            );
        })?;
        cx.run_until_parked();
        Ok(())
    };
    let key = |cx: &mut HeadlessAppContext, key: &str| -> Result<()> {
        let stroke = Keystroke::parse(key)?;
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_keystroke(stroke, cx);
        })?;
        cx.run_until_parked();
        Ok(())
    };
    let type_text = |cx: &mut HeadlessAppContext, text: &str| -> Result<()> {
        cx.update_window(window.into(), |_, window, cx| {
            for character in text.chars() {
                window.dispatch_keystroke(
                    Keystroke {
                        key: character.to_string(),
                        ..Default::default()
                    },
                    cx,
                );
            }
        })?;
        cx.run_until_parked();
        Ok(())
    };
    let shot = |cx: &mut HeadlessAppContext, name: &str| -> Result<()> {
        if let Some(directory) = &shots {
            cx.update_window(window.into(), |_, window, cx| {
                window.dispatch_event(
                    PlatformInput::MouseMove(gpui::MouseMoveEvent {
                        position: point(px(768.), px(6.)),
                        pressed_button: None,
                        modifiers: Modifiers::default(),
                    }),
                    cx,
                );
            })?;
            cx.run_until_parked();
            cx.capture_screenshot(window.into())?
                .save(Path::new(directory).join(format!("archive-{name}.png")))?;
        }
        Ok(())
    };
    shot(&mut cx, "list")?;
    if shots.is_some() {
        click(&mut cx, 916., 202.)?;
        shot(&mut cx, "filter")?;
        key(&mut cx, "escape")?;
        click(&mut cx, 1084., 202.)?;
        shot(&mut cx, "projects")?;
        key(&mut cx, "escape")?;
    }
    click(&mut cx, 1128., 136.)?;
    ensure!(
        page.read_with(&cx, |page, _| page.confirmation().is_some()),
        "Delete all did not ask for confirmation"
    );
    shot(&mut cx, "delete-all")?;
    key(&mut cx, "escape")?;
    ensure!(
        page.read_with(&cx, |page, _| page.confirmation().is_none()
            && page.threads.as_ref().unwrap().as_ref().unwrap().len() == 3),
        "Cancel changed the archived list"
    );
    let restored = ids[0].clone();
    if shots.is_some() {
        click(&mut cx, 1054., 336.)?;
        shot(&mut cx, "delete-single")?;
        key(&mut cx, "escape")?;
        click(&mut cx, 600., 202.)?;
        type_text(&mut cx, "subtract")?;
        shot(&mut cx, "search")?;
        key(&mut cx, "cmd-a")?;
        key(&mut cx, "backspace")?;
        type_text(&mut cx, "zzzzzz")?;
        shot(&mut cx, "no-matches")?;
        key(&mut cx, "cmd-a")?;
        key(&mut cx, "backspace")?;
    }
    click(&mut cx, 1115., 336.)?;
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        cx.run_until_parked();
        let removed = page.read_with(&cx, |page, _| {
            page.threads
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .iter()
                .all(|thread| thread.id != restored)
        });
        if removed {
            break;
        }
        ensure!(
            Instant::now() < until,
            "restored chat stayed in the archive"
        );
        thread::sleep(Duration::from_millis(5));
    }
    ensure!(root.read_with(&cx, |root, cx| {
        root.recents().contains(&restored)
            && root
                .toasts(cx)
                .iter()
                .filter(|title| title.as_ref() == "Unarchived chat")
                .count()
                == 1
            && root
                .toasts(cx)
                .iter()
                .all(|title| title.as_ref() != "Unarchiving chat")
    }));
    shot(&mut cx, "restored")?;
    let deleted = ids[2].clone();
    click(&mut cx, 1054., 396.)?;
    ensure!(
        page.read_with(&cx, |page, _| page.confirmation().is_some()),
        "Row delete did not ask for confirmation"
    );
    click(&mut cx, 795., 456.)?;
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        cx.run_until_parked();
        if page.read_with(&cx, |page, _| {
            page.threads
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .iter()
                .all(|thread| thread.id != deleted)
        }) {
            break;
        }
        ensure!(Instant::now() < until, "deleted chat stayed in the archive");
        thread::sleep(Duration::from_millis(5));
    }
    ensure!(root.read_with(&cx, |root, cx| {
        !root.recents().contains(&deleted)
            && root
                .toasts(cx)
                .iter()
                .any(|title| title.as_ref() == "Deleted archived chat")
    }));
    shot(&mut cx, "deleted")?;
    Ok(())
}
