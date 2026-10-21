#![cfg(target_os = "macos")]

mod common;

use anyhow::{Result, ensure};
use common::Fx;
use gpui::{
    Entity, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, point, px,
};
use std::{fs, path::Path, sync::Arc};
use ui::screens::settings::archived_chats::{ArchivedChats, OpenArchivedChats};

#[test]
fn proxy() {
    common::install_panic_hook();
    common::proxy_child();
}

fn fixture() -> Result<Fx> {
    common::install_panic_hook();
    let mut fx = Fx::captured(
        "dark",
        (1280., 832.),
        common::script(Some("sessions/archived-chats.jsonl"), |_| {})?,
    )?;
    if std::env::var_os("CGX_ARCHIVE_PARTIAL_ERROR").is_some() {
        let host = Arc::new(match std::env::var("CGX_ARCHIVE_PROJECT_FIXTURE") {
            Ok(path) => app::host::ReplayHost::from_recording(&fs::read_to_string(path)?)?,
            Err(_) => app::host::ReplayHost::from_oracle(include_str!(
                "../../../tools/route1/oracle.json"
            ))?,
        });
        fx.root
            .update(&mut fx.cx, |root, cx| root.set_host(host, cx));
    }
    fx.cx
        .update(|cx| ui::composer::Composer::set_caret_transparent(cx, true));
    fx.action(Box::new(OpenArchivedChats))?;
    fx.wait("archived settings", |fx| {
        fx.read(|root, cx| {
            root.archived_settings()
                .is_some_and(|page| matches!(page.read(cx).threads, Some(Ok(_))))
        })
    })?;
    Ok(fx)
}

fn page(fx: &Fx) -> Entity<ArchivedChats> {
    fx.read(|root, _| root.archived_settings().expect("archived settings").clone())
}

fn ids(fx: &Fx) -> Vec<String> {
    fx.read(|_, cx| {
        page(fx)
            .read(cx)
            .threads
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .iter()
            .map(|thread| thread.id.clone())
            .collect()
    })
}

fn shot(fx: &mut Fx, name: &str) -> Result<()> {
    if let Ok(directory) = std::env::var("CGX_ARCHIVE_SHOTS") {
        fs::create_dir_all(&directory)?;
        fx.hover((768., 6.))?;
        fx.shot()?
            .save(Path::new(&directory).join(format!("archive-{name}.png")))?;
    }
    Ok(())
}

#[test]
fn native_archive_interactions() -> Result<()> {
    let mut fx = fixture()?;
    let initial = ids(&fx);
    ensure!(initial.len() == 3);
    shot(&mut fx, "list")?;
    if std::env::var_os("CGX_ARCHIVE_PROJECT_FIXTURE").is_some() {
        shot(&mut fx, "project-list")?;
        fx.click((1158., 267.))?;
        shot(&mut fx, "project-actions")?;
        fx.click((1060., 301.))?;
        ensure!(fx.read(|_, cx| page(&fx).read(cx).confirmation().is_some()));
        shot(&mut fx, "project-delete")?;
        fx.keys("escape")?;
    }
    if std::env::var_os("CGX_ARCHIVE_SHOTS").is_some() {
        fx.click((916., 203.))?;
        shot(&mut fx, "filter")?;
        fx.keys("escape")?;
        fx.click((1084., 203.))?;
        shot(&mut fx, "projects")?;
        fx.keys("escape")?;
    }
    fx.click((1128., 136.))?;
    ensure!(
        fx.read(|_, cx| page(&fx).read(cx).confirmation().is_some()),
        "Delete all did not ask for confirmation"
    );
    shot(&mut fx, "delete-all")?;
    fx.keys("escape")?;
    ensure!(ids(&fx) == initial, "Cancel changed the archived list");
    if std::env::var_os("CGX_ARCHIVE_SHOTS").is_some() {
        fx.click((1054., 324.))?;
        shot(&mut fx, "delete-single")?;
        fx.keys("escape")?;
        fx.click((600., 203.))?;
        fx.type_text("subtract")?;
        shot(&mut fx, "search")?;
        fx.keys("cmd-a backspace")?;
        fx.type_text("zzzzzz")?;
        shot(&mut fx, "no-matches")?;
        fx.keys("cmd-a backspace")?;
    }
    fx.click((1115., 324.))?;
    fx.wait("restored archived chat", |fx| {
        !ids(fx).contains(&initial[0])
    })?;
    ensure!(fx.read(|root, cx| {
        root.recents().contains(&initial[0])
            && root
                .toasts(cx)
                .iter()
                .any(|title| title.as_ref() == "Unarchived chat")
    }));
    shot(&mut fx, "restored")?;
    fx.click((1054., 385.))?;
    ensure!(
        fx.read(|_, cx| page(&fx).read(cx).confirmation().is_some()),
        "Row delete did not ask for confirmation"
    );
    fx.click((842., 452.))?;
    fx.wait("deleted archived chat", |fx| !ids(fx).contains(&initial[2]))?;
    ensure!(fx.read(|root, cx| {
        !root.recents().contains(&initial[2])
            && root
                .toasts(cx)
                .iter()
                .any(|title| title.as_ref() == "Deleted archived chat")
    }));
    shot(&mut fx, "deleted")?;
    fx.click((1128., 136.))?;
    fx.click((842., 452.))?;
    fx.wait("all archived chats deleted", |fx| ids(fx).is_empty())?;
    ensure!(
        fx.read(|root, _| root.recents()) == [initial[0].clone()],
        "Delete all removed an active chat or retained an archived chat"
    );
    shot(&mut fx, "empty-error")?;
    if let Ok(directory) = std::env::var("CGX_ARCHIVE_SHOTS") {
        let archived = page(&fx);
        let image = fx.cx.update_window(fx.window, |_, window, cx| {
            let position = point(px(1128.), px(214.));
            for event in [
                PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Default::default(),
                }),
                PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Default::default(),
                    click_count: 1,
                }),
            ] {
                window.dispatch_event(event, cx);
            }
            ensure!(archived.read(cx).threads.is_none());
            window.draw(cx).clear(cx);
            window.render_to_image()
        })??;
        image.save(Path::new(&directory).join("archive-loading.png"))?;
        fx.wait("retried empty archive", |fx| {
            fx.read(|_, cx| page(fx).read(cx).threads.is_some())
        })?;
    }
    Ok(())
}

#[test]
fn view_opens_the_restored_chat() -> Result<()> {
    let mut fx = fixture()?;
    let restored = ids(&fx)[0].clone();
    let prior_recents = fx.read(|root, _| root.recents());
    fx.click((1115., 324.))?;
    fx.wait("unarchive View action", |fx| {
        fx.read(|root, cx| {
            root.toasts(cx)
                .iter()
                .any(|title| title.as_ref() == "Unarchived chat")
        })
    })?;
    ensure!(
        fx.read(|root, _| root.recents()) == prior_recents,
        "Unarchive reordered existing recent chats"
    );
    fx.click((699., 69.))?;
    fx.wait_thread(&restored)?;
    shot(&mut fx, "view")?;
    Ok(())
}
