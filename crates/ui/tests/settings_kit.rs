// Interaction checks use no GPU renderer; the example owns pixel captures.
use gpui::{
    AnyWindowHandle, AppContext as _, Context, Focusable, HeadlessAppContext, IntoElement,
    KeyUpEvent, Keystroke, MouseButton, MouseDownEvent, PlatformInput, Render, Window, point, px,
    size,
};
use std::{cell::RefCell, rc::Rc, sync::Arc};
use ui::settings_kit::{Choice, MultiSelect, Select, Selected};

fn context() -> HeadlessAppContext {
    #[cfg(target_os = "macos")]
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        std::process::exit(101);
    }));
    let mut cx =
        HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
    cx.update(|cx| {
        cx.set_global(theme::Theme::dark());
        theme::load_fonts(cx);
    });
    cx
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
    cx.run_until_parked();
}

#[test]
fn keyboard_skips_locked_choices_and_restores_trigger_focus() {
    let mut cx = context();
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

    let mut cx = context();
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
    let mut cx = context();
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

#[test]
fn searchable_select_uses_aliases_and_returns_focus_without_duplicate_selection() {
    let mut cx = context();
    cx.update(ui::composer::bind_keys);
    let selections = Rc::new(RefCell::new(Vec::new()));
    let received = selections.clone();
    let window = cx
        .open_window(size(px(640.), px(480.)), |window, cx| {
            let select = cx.new(|cx| {
                Select::new(
                    "fr",
                    vec![
                        Choice {
                            value: "en".into(),
                            label: "English".into(),
                            checked: false,
                            disabled: true,
                        },
                        Choice {
                            value: "fr".into(),
                            label: "Français".into(),
                            checked: false,
                            disabled: false,
                        },
                        Choice {
                            value: "de".into(),
                            label: "Deutsch".into(),
                            checked: false,
                            disabled: false,
                        },
                    ],
                    Some("Search…"),
                    cx,
                )
            });
            select.update(cx, |select, _| {
                select.search_aliases(|| [("de".into(), "German".into())].into())
            });
            cx.subscribe(&select, move |_, selected: &Selected, _| {
                received.borrow_mut().push(selected.0.to_string())
            })
            .detach();
            select.focus_handle(cx).focus(window, cx);
            select
        })
        .unwrap();
    cx.run_until_parked();
    press(&mut cx, window.into(), "down");
    for key in ["g", "e", "r", "m", "a", "n", "down", "enter"] {
        press(&mut cx, window.into(), key);
    }
    assert_eq!(*selections.borrow(), ["de"]);
    cx.update_window(window.into(), |view, window, cx| {
        let select = view.downcast::<Select>().unwrap();
        assert!(select.focus_handle(cx).is_focused(window));
    })
    .unwrap();
}

#[test]
fn non_searchable_select_with_details_supports_keyboard_selection() {
    let mut cx = context();
    cx.update(ui::composer::bind_keys);
    let selections = Rc::new(RefCell::new(Vec::new()));
    let received = selections.clone();
    let window = cx
        .open_window(size(px(500.), px(400.)), |window, cx| {
            let select = cx.new(|cx| {
                Select::new(
                    "a",
                    vec![
                        Choice {
                            value: "a".into(),
                            label: "First".into(),
                            checked: false,
                            disabled: false,
                        },
                        Choice {
                            value: "b".into(),
                            label: "Second".into(),
                            checked: false,
                            disabled: false,
                        },
                    ],
                    None,
                    cx,
                )
                .with_details(&[("a", "First detail"), ("b", "Second detail")])
            });
            cx.subscribe(&select, move |_, selected: &Selected, _| {
                received.borrow_mut().push(selected.0.to_string());
            })
            .detach();
            select.focus_handle(cx).focus(window, cx);
            select
        })
        .unwrap();
    cx.run_until_parked();
    for key in ["down", "down", "enter"] {
        press(&mut cx, window.into(), key);
    }
    assert_eq!(*selections.borrow(), ["b"]);
    cx.update_window(window.into(), |view, window, cx| {
        assert!(
            view.downcast::<Select>()
                .unwrap()
                .focus_handle(cx)
                .is_focused(window)
        );
    })
    .unwrap();
}

#[test]
fn typing_after_arrow_navigation_keeps_filtering_the_search_box() {
    let mut cx = context();
    cx.update(ui::composer::bind_keys);
    let selections = Rc::new(RefCell::new(Vec::new()));
    let received = selections.clone();
    let window = cx
        .open_window(size(px(500.), px(400.)), |window, cx| {
            let choices = [("en", "English"), ("fr", "French"), ("de", "German")]
                .into_iter()
                .map(|(value, label)| Choice {
                    value: value.into(),
                    label: label.into(),
                    checked: false,
                    disabled: false,
                })
                .collect();
            let select = cx.new(|cx| Select::new("en", choices, Some("Search"), cx));
            cx.subscribe(&select, move |_, selected: &Selected, _| {
                received.borrow_mut().push(selected.0.to_string());
            })
            .detach();
            select.focus_handle(cx).focus(window, cx);
            select
        })
        .unwrap();
    cx.run_until_parked();
    for key in ["down", "down", "g", "e", "r", "m", "a", "n", "enter"] {
        press(&mut cx, window.into(), key);
    }
    assert_eq!(*selections.borrow(), ["de"]);
}

#[test]
fn default_action_remains_available_while_options_are_filtered() {
    let mut cx = context();
    cx.update(ui::composer::bind_keys);
    let selections = Rc::new(RefCell::new(Vec::new()));
    let received = selections.clone();
    let window = cx
        .open_window(size(px(500.), px(400.)), |window, cx| {
            let choices = [("", "Auto detect"), ("fr", "French"), ("de", "German")]
                .into_iter()
                .map(|(value, label)| Choice {
                    value: value.into(),
                    label: label.into(),
                    checked: false,
                    disabled: false,
                })
                .collect();
            let select =
                cx.new(|cx| Select::new("de", choices, Some("Search"), cx).keep_default_visible());
            cx.subscribe(&select, move |_, selected: &Selected, _| {
                received.borrow_mut().push(selected.0.to_string());
            })
            .detach();
            select.focus_handle(cx).focus(window, cx);
            select
        })
        .unwrap();
    cx.run_until_parked();
    for key in ["down", "g", "e", "r", "m", "a", "n", "up", "enter"] {
        press(&mut cx, window.into(), key);
    }
    assert_eq!(*selections.borrow(), [""]);
}
