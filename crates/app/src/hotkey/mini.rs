use super::{Root, Runtime, Target, windows};
use anyhow::{Context as _, Result};
use gpui::{
    AnyElement, App, BorrowAppContext as _, Bounds, Context, Focusable as _,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Pixels, Point,
    StatefulInteractiveElement as _, Styled as _, Window, WindowHandle, div, point, px, size,
};
use theme::ActiveTheme as _;
use ui::screens::mini::{composer_bounds, pet_bounds};

#[derive(Default)]
pub(super) struct State {
    expanded: bool,
    #[cfg(target_os = "macos")]
    interactive: bool,
    drag: Option<Point<Pixels>>,
    dragged: bool,
    #[cfg(target_os = "macos")]
    _monitor: Option<super::desktop::Monitor>,
    #[cfg(target_os = "macos")]
    _pointer_task: Option<gpui::Task<()>>,
}

pub(super) fn initial_bounds(owner: WindowHandle<Root>, cx: &mut App) -> Result<Bounds<Pixels>> {
    let owner_bounds = owner.update(cx, |_, window, _| window.bounds())?;
    let displays = cx.displays();
    let display = displays
        .iter()
        .find(|display| display.bounds().intersects(&owner_bounds))
        .or_else(|| displays.first())
        .context("Mini requires a display")?
        .bounds();
    let extent = size(
        px(356.).min(display.size.width),
        px(320.).min(display.size.height),
    );
    Ok(Bounds::new(
        point(
            display.right() - extent.width,
            display.bottom() - extent.height,
        ),
        extent,
    ))
}

pub(super) fn toggle(from_hotkey: bool, cx: &mut App) {
    if let Err(error) = toggle_inner(from_hotkey, cx) {
        let owner = cx.global::<Runtime>().primary;
        if let Some(owner) = owner {
            let _ = owner.update(cx, |root, _, cx| {
                root.toast(
                    ui::toast::Toast::danger("Could not open Mini").description(error.to_string()),
                    cx,
                )
            });
        }
        eprintln!("codex-gpui: Mini: {error:#}");
    }
}

pub(super) fn restore(cx: &mut App) {
    if !crate::qa::ENABLED
        && cfg!(target_os = "macos")
        && cx
            .try_global::<crate::host_settings::Settings>()
            .is_some_and(|settings| {
                settings.read_all().get("electron-avatar-overlay-open")
                    == Some(&serde_json::Value::Bool(true))
            })
    {
        toggle(false, cx);
    }
}

fn toggle_inner(from_hotkey: bool, cx: &mut App) -> Result<()> {
    let owner = cx
        .global::<Runtime>()
        .primary
        .context("Mini requires a primary window")?;
    let existing = cx.global::<Runtime>().mini;
    let (handle, visible) = match existing {
        Some(handle) if handle.update(cx, |_, _, _| ()).is_ok() => {
            (handle, windows::is_visible(handle.into(), cx)?)
        }
        _ => (super::open(owner, Target::Mini, cx)?, false),
    };
    if visible && !from_hotkey {
        handle.update(cx, |_, window, cx| dismiss(window, cx))??;
        return Ok(());
    }
    let expanded = handle.update(cx, |root, window, cx| {
        let state = state_mut(root).context("Mini window lost its state")?;
        let expanded = if visible && window.is_window_active() {
            !state.expanded
        } else {
            from_hotkey
        };
        state.expanded = expanded;
        windows::set_keyboard_interactive(window, expanded)?;
        if expanded {
            let composer = root.home.read(cx).composer.clone();
            composer.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            window.blur(cx);
        }
        cx.notify();
        Ok::<_, anyhow::Error>(expanded)
    })??;
    windows::show(
        handle.into(),
        if expanded {
            windows::Presentation::Foreground
        } else {
            windows::Presentation::Background
        },
        cx,
    )?;
    save_open(true, cx);
    Ok(())
}

fn save_open(open: bool, cx: &mut App) {
    if cx.has_global::<crate::host_settings::Settings>() {
        let result = cx.update_global::<crate::host_settings::Settings, _>(|settings, _| {
            settings.write("electron-avatar-overlay-open", open.into())
        });
        if let Err(error) = result {
            eprintln!("codex-gpui: cannot save Mini visibility: {error:#}");
        }
    }
}

pub(super) fn dismiss(window: &mut Window, cx: &mut App) -> Result<()> {
    windows::set_keyboard_interactive(window, false)?;
    windows::close(window, cx);
    save_open(false, cx);
    Ok(())
}

fn state_mut(root: &mut Root) -> Option<&mut State> {
    root.popout.as_mut()?.mini.as_mut()
}

pub(super) fn observe_pointer(handle: WindowHandle<Root>, cx: &mut App) -> Result<()> {
    handle.update(cx, |_, window, cx| {
        window.on_window_should_close(cx, |window, cx| {
            if let Err(error) = dismiss(window, cx) {
                eprintln!("codex-gpui: cannot dismiss Mini: {error:#}");
            }
            false
        });
    })?;
    #[cfg(target_os = "macos")]
    {
        let (monitor, events) = super::desktop::Monitor::pointer()?;
        let task = cx.spawn(async move |cx| {
            while events.recv().await.is_ok() {
                if handle
                    .update(cx, |root, window, _| {
                        let has_chat = root.thread.is_some()
                            && matches!(root.state.route, crate::Route::Thread(_));
                        let Some(state) = state_mut(root) else { return };
                        let inside = ui::screens::mini::hit_bounds(
                            window.bounds().size,
                            state.expanded,
                            has_chat,
                        )
                        .try_fold(
                            false,
                            |inside, region| -> Result<bool> {
                                Ok(inside || super::desktop::pointer_inside(window, region)?)
                            },
                        );
                        match inside {
                            Ok(inside) if state.interactive != inside => {
                                if windows::set_pointer_interactive_window(window, inside).is_ok() {
                                    state.interactive = inside;
                                }
                            }
                            Ok(_) => {}
                            Err(error) => eprintln!("codex-gpui: Mini pointer: {error:#}"),
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        handle.update(cx, |root, _, _| {
            if let Some(state) = state_mut(root) {
                state._monitor = Some(monitor);
                state._pointer_task = Some(task);
            }
        })?;
    }
    Ok(())
}

fn interact(root: &mut Root, window: &mut Window, cx: &mut Context<Root>) {
    if state_mut(root).is_some_and(|state| state.dragged) {
        return;
    }
    let Some(state) = state_mut(root) else { return };
    state.expanded = !state.expanded;
    let expanded = state.expanded;
    let result = windows::set_keyboard_interactive(window, expanded)
        .and_then(|()| windows::set_pointer_interactive_window(window, true));
    if let Err(error) = result {
        root.toast(
            ui::toast::Toast::danger("Could not focus Mini").description(error.to_string()),
            cx,
        );
        return;
    }
    if expanded {
        let composer = root.home.read(cx).composer.clone();
        composer.read(cx).focus_handle(cx).focus(window, cx);
        if !crate::qa::ENABLED {
            window.activate_window();
        }
    } else {
        window.blur(cx);
    }
    cx.notify();
}

pub(super) fn content(root: &Root, window: &Window, cx: &mut Context<Root>) -> AnyElement {
    let expanded = root
        .popout
        .as_ref()
        .and_then(|popout| popout.mini.as_ref())
        .is_some_and(|state| state.expanded);
    let settings = cx
        .try_global::<crate::host_settings::Settings>()
        .map(|settings| settings.read_all());
    let id = settings
        .and_then(|settings| settings.get("selected-avatar-id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("codex");
    let reduced_motion = ui::motion::reduced(cx);
    let viewport = window.viewport_size();
    let pet = pet_bounds(viewport);
    let composer_at = composer_bounds(viewport);
    let t = cx.theme().clone();
    let mut body = div().size_full().relative();
    if expanded {
        body = body.child(
            div()
                .absolute()
                .left(composer_at.origin.x)
                .top(composer_at.origin.y)
                .w(composer_at.size.width)
                .child(root.home.read(cx).composer.clone()),
        );
    } else {
        let image = ui::screens::mini::pet(id, window.scale_factor(), reduced_motion);
        let artwork = match image {
            Ok(image) => gpui::img(image).size_full().into_any_element(),
            Err(error) => ui::text(error.to_string(), 12., 16., 400., t.text).into_any_element(),
        };
        body = body.child(
            div()
                .id("mini-pet")
                .absolute()
                .left(pet.origin.x)
                .top(pet.origin.y)
                .w(pet.size.width)
                .h(pet.size.height)
                .role(gpui::Role::Button)
                .aria_label("Chat")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|root, event: &gpui::MouseDownEvent, _, _| {
                        if let Some(state) = state_mut(root) {
                            state.drag = Some(event.position);
                            state.dragged = false;
                        }
                    }),
                )
                .on_mouse_move(
                    cx.listener(|root, event: &gpui::MouseMoveEvent, window, _| {
                        if let Some(state) = state_mut(root)
                            && let Some(start) = state.drag
                            && ((event.position.x - start.x).abs() > px(4.)
                                || (event.position.y - start.y).abs() > px(4.))
                        {
                            state.drag = None;
                            state.dragged = true;
                            window.start_window_move();
                        }
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|root, _, _, _| {
                        if let Some(state) = state_mut(root) {
                            state.drag = None;
                        }
                    }),
                )
                .on_click(cx.listener(|root, _, window, cx| interact(root, window, cx)))
                .child(artwork),
        );
    }
    if let Some(thread) = root
        .thread
        .as_ref()
        .filter(|_| matches!(root.state.route, crate::Route::Thread(_)))
    {
        let title = thread.read(cx).header_title();
        let chat_at = ui::screens::mini::chat_bounds(viewport);
        body = body.child(
            div()
                .id("mini-current-chat")
                .absolute()
                .left(chat_at.origin.x)
                .top(chat_at.origin.y)
                .w(chat_at.size.width)
                .h(chat_at.size.height)
                .rounded(px(18.))
                .bg(t.control_background)
                .px(px(12.))
                .flex()
                .items_center()
                .role(gpui::Role::Button)
                .aria_label("Open in Main Window")
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(super::OpenInMainWindow), cx)
                })
                .child(ui::text(title, 14., 20., 500., t.text)),
        );
    }
    body.into_any_element()
}
