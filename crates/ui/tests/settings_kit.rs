use gpui::{
    AnyWindowHandle, AppContext as _, Context, Focusable, HeadlessAppContext, IntoElement,
    KeyUpEvent, Keystroke, MouseButton, MouseDownEvent, PlatformInput, Render, Window, point, px,
    size,
};
use std::sync::Arc;
use ui::settings_kit::{Choice, MultiSelect};

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
    cx.run_until_parked();
}

#[test]
fn keyboard_skips_locked_choices_and_restores_trigger_focus() {
    let mut cx =
        HeadlessAppContext::with_platform(textsys::text_system(), Arc::new(theme::Assets), || {
            gpui_platform::current_headless_renderer()
        });
    cx.update(|cx| {
        cx.set_global(theme::Theme::dark());
        theme::load_fonts(cx);
    });
    let window = cx
        .open_window(size(px(500.), px(400.)), |window, cx| {
            let menu = cx.new(|cx| {
                MultiSelect::new(
                    "Available reasoning efforts",
                    vec![
                        Choice {
                            value: "medium".into(),
                            label: "Medium".into(),
                            checked: true,
                            disabled: true,
                        },
                        Choice {
                            value: "max".into(),
                            label: "Extra high".into(),
                            checked: true,
                            disabled: false,
                        },
                    ],
                    cx,
                )
            });
            menu.read(cx).focus_handle(cx).focus(window, cx);
            menu
        })
        .unwrap();
    cx.run_until_parked();
    for key in ["down", "space", "escape"] {
        press(&mut cx, window.into(), key);
    }
    cx.update(|cx| {
        window
            .update(cx, |menu, window, cx| {
                assert!(menu.choices()[0].checked);
                assert!(!menu.choices()[1].checked);
                assert!(menu.focus_handle(cx).is_focused(window));
                menu.set_checked("max", true, cx);
                assert!(menu.choices()[1].checked);
            })
            .unwrap()
    });
    press(&mut cx, window.into(), "down");
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position: point(px(450.), px(350.)),
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        window
            .update(cx, |menu, window, cx| {
                assert!(menu.focus_handle(cx).is_focused(window));
            })
            .unwrap()
    });
}

#[test]
fn destructive_confirmation_defaults_to_cancel_and_returns_focus() {
    use std::{cell::RefCell, rc::Rc};
    use ui::confirmation::{Confirmation, Outcome};

    let mut cx =
        HeadlessAppContext::with_platform(textsys::text_system(), Arc::new(theme::Assets), || {
            gpui_platform::current_headless_renderer()
        });
    cx.update(|cx| cx.set_global(theme::Theme::dark()));
    let outcomes = Rc::new(RefCell::new(Vec::new()));
    let received = outcomes.clone();
    let previous = cx.update(|cx| cx.focus_handle());
    let return_focus = previous.clone();
    let window = cx
        .open_window(size(px(500.), px(400.)), |window, cx| {
            previous.focus(window, cx);
            let dialog = cx.new(|cx| {
                Confirmation::new(
                    "Discard changes?",
                    "Your changes will be lost.",
                    "Discard changes",
                    window,
                    cx,
                )
                .destructive()
            });
            cx.subscribe(&dialog, move |_, outcome: &Outcome, _| {
                received.borrow_mut().push(*outcome)
            })
            .detach();
            dialog
        })
        .unwrap();
    cx.run_until_parked();
    press(&mut cx, window.into(), "enter");
    assert_eq!(*outcomes.borrow(), [Outcome::Cancel]);
    cx.update_window(window.into(), |_, window, _| {
        assert!(return_focus.is_focused(window));
    })
    .unwrap();
}

struct TestLink;

impl Render for TestLink {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ui::settings_kit::external_link_row(
            "documentation",
            "Documentation",
            None,
            "https://example.com/docs",
            window,
            cx,
        )
    }
}

#[test]
fn link_opens_once_when_enter_is_released() {
    let mut cx =
        HeadlessAppContext::with_platform(textsys::text_system(), Arc::new(theme::Assets), || {
            gpui_platform::current_headless_renderer()
        });
    cx.update(|cx| cx.set_global(theme::Theme::dark()));
    let window = cx
        .open_window(size(px(500.), px(400.)), |_, cx| cx.new(|_| TestLink))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.focus_next(cx);
        window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.take_opened_url().is_none());
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(
            PlatformInput::KeyUp(KeyUpEvent {
                keystroke: Keystroke::parse("enter").unwrap(),
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.take_opened_url().as_deref(),
        Some("https://example.com/docs")
    );
}
