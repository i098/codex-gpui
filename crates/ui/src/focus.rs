//! Keyboard focus rings from `refs/interactions/f1b.json`. The overlay does not affect layout.
//! Mouse movement preserves a keyboard ring. A pointer press clears it.

use gpui::{
    App, Corners, DispatchPhase, Div, ElementId, Entity, FocusHandle, InteractiveElement,
    KeyDownEvent, MouseDownEvent, ParentElement, Pixels, Stateful, Styled, Window, canvas, px, rgb,
};
use std::cell::Cell;

pub struct ControlFocus {
    pub handle: FocusHandle,
    keyboard: Cell<bool>,
}

pub fn handle(
    id: impl Into<ElementId>,
    index: isize,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ControlFocus> {
    window.with_element_namespace("control-focus", |window| {
        window.use_keyed_state(id, cx, |_, cx| ControlFocus {
            handle: cx.focus_handle().tab_stop(true).tab_index(index),
            keyboard: Cell::new(false),
        })
    })
}

pub fn ring(
    control: Stateful<Div>,
    focus: &Entity<ControlFocus>,
    radius: Pixels,
    outset: Pixels,
    cx: &App,
) -> Stateful<Div> {
    let state = focus.clone();
    let handle = state.read(cx).handle.clone();
    control.track_focus(&handle).child(
        canvas(
            |_, _, _| {},
            move |bounds, (), window, cx| {
                let focus = state.read(cx);
                if handle.is_focused(window) && window.last_input_was_keyboard() {
                    focus.keyboard.set(true);
                }
                if handle.is_focused(window) && focus.keyboard.get() {
                    shapes::paint(
                        &shapes::Decoration {
                            radius: Corners::all(radius + outset),
                            corner: if radius == theme::Theme::RADIUS_FULL {
                                shapes::Corner::Round
                            } else {
                                shapes::Corner::Squircle
                            },
                            border: Some((px(2.), rgb(0x3a83f7).into())),
                            ..Default::default()
                        },
                        None,
                        bounds.dilate(outset),
                        window,
                        cx,
                    );
                }
                window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture && state.read(cx).keyboard.replace(false) {
                        window.refresh();
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    )
}

pub fn tab(event: &KeyDownEvent, window: &mut Window, cx: &mut App) {
    let key = &event.keystroke;
    if key.key == "tab" && !key.modifiers.control && !key.modifiers.alt && !key.modifiers.platform {
        if key.modifiers.shift {
            window.focus_prev(cx);
        } else {
            window.focus_next(cx);
        }
        cx.stop_propagation();
    }
}
