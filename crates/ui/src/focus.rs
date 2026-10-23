//! Keyboard focus rings from `refs/interactions/f1b.json`. The overlay does not affect layout.
//! Mouse movement preserves a keyboard ring. A pointer press clears it.

use gpui::{
    AnyElement, App, Corners, DispatchPhase, Div, ElementId, Entity, FocusHandle,
    InteractiveElement, IntoElement, KeyDownEvent, MouseDownEvent, ParentElement, Pixels, Stateful,
    Styled, Window, canvas, px, rgb,
};
use std::cell::Cell;

const RING_WIDTH: Pixels = px(2.);

pub struct ControlFocus {
    pub handle: FocusHandle,
    keyboard: Cell<bool>,
}

impl ControlFocus {
    /// Keyboard-visible focus persists through pointer motion, but not a pointer press.
    pub fn is_focus_visible(&self, window: &Window) -> bool {
        self.handle.is_focused(window) && (self.keyboard.get() || window.last_input_was_keyboard())
    }
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
    paint_ring(control, focus, radius, outset, false, cx)
}

/// Paints an outer focus ring as a zero-blur spread shadow.
pub fn shadow_ring(
    control: Stateful<Div>,
    focus: &Entity<ControlFocus>,
    radius: Pixels,
    spread: Pixels,
    cx: &App,
) -> Stateful<Div> {
    paint_ring(control, focus, radius, spread, true, cx)
}

/// Paints a 2px CSS outline at a signed offset from the border box.
/// An offset of -2px keeps the outline inside the control.
/// Active outlines use a bounded local recording for their corner coverage.
pub fn inset_outline(
    control: Stateful<Div>,
    focus: &Entity<ControlFocus>,
    radius: Pixels,
    offset: Pixels,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let visible = focus.read(cx).is_focus_visible(window);
    let control = ring(control, focus, radius, RING_WIDTH + offset, cx);
    if visible {
        crate::composited_bounds(control.top_layer()).into_any_element()
    } else {
        control.into_any_element()
    }
}

fn paint_ring(
    control: Stateful<Div>,
    focus: &Entity<ControlFocus>,
    radius: Pixels,
    outset: Pixels,
    shadow: bool,
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
                if focus.is_focus_visible(window) {
                    shapes::paint(
                        &shapes::Decoration {
                            radius: Corners::all(if shadow { radius } else { radius + outset }),
                            corner: if radius == theme::Theme::RADIUS_FULL {
                                shapes::Corner::Round
                            } else {
                                shapes::Corner::Squircle
                            },
                            border: (!shadow).then(|| (RING_WIDTH, rgb(0x3a83f7).into())),
                            shadows: if shadow {
                                vec![shapes::Shadow {
                                    color: rgb(0x3a83f7).into(),
                                    offset: gpui::point(px(0.), px(0.)),
                                    blur: px(0.),
                                    spread: RING_WIDTH,
                                    inset: false,
                                }]
                            } else {
                                Vec::new()
                            },
                            ..Default::default()
                        },
                        None,
                        if shadow {
                            bounds
                        } else {
                            bounds.dilate(outset)
                        },
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use gpui::{AppContext, HeadlessAppContext, IntoElement, Render, div, size};
    use std::sync::Arc;

    struct Ring {
        focus: Entity<ControlFocus>,
    }

    impl Render for Ring {
        fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .relative()
                .bg(rgb(0x282828))
                .child(shadow_ring(
                    div()
                        .id("shadow-ring-test")
                        .absolute()
                        .left(px(457.25))
                        .top(px(681.))
                        .w(px(132.27344))
                        .h(px(28.)),
                    &self.focus,
                    theme::Theme::RADIUS_FULL,
                    px(2.),
                    cx,
                ))
        }
    }

    struct InsetOutline {
        focus: Entity<ControlFocus>,
    }

    impl Render for InsetOutline {
        fn render(
            &mut self,
            window: &mut Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div()
                .size_full()
                .relative()
                .bg(rgb(0x1d1d1d))
                .child(inset_outline(
                    div()
                        .id("inset-outline-test")
                        .absolute()
                        .left(px(60.))
                        .top(px(96.))
                        .w(px(272.))
                        .h(px(30.)),
                    &self.focus,
                    px(12.5),
                    px(-2.),
                    window,
                    cx,
                ))
        }
    }

    #[test]
    fn shadow_ring_matches_original_edge_and_corner() -> anyhow::Result<()> {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let mut cx = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(()),
            gpui_platform::current_headless_renderer,
        );
        let window = cx.open_window(size(px(1280.), px(832.)), |window, cx| {
            let focus = cx.new(|cx| ControlFocus {
                handle: cx.focus_handle(),
                keyboard: Cell::new(true),
            });
            let handle = focus.read(cx).handle.clone();
            window.focus(&handle, cx);
            cx.new(|_| Ring { focus })
        })?;
        cx.run_until_parked();
        let image = cx.capture_screenshot(window.into())?;
        for (x, y, expected) in [
            (911, 1390, [56, 125, 234, 255]),
            (918, 1409, [57, 129, 245, 255]),
            (920, 1410, [58, 131, 247, 255]),
        ] {
            assert_eq!(image.get_pixel(x, y).0, expected, "ring ({x}, {y})");
        }
        Ok(())
    }

    #[test]
    fn inset_outline_matches_original_corners_and_stays_inside() -> anyhow::Result<()> {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        for keyboard in [true, false] {
            let mut cx = HeadlessAppContext::with_platform(
                textsys::text_system(),
                Arc::new(()),
                gpui_platform::current_headless_renderer,
            );
            let window = cx.open_window(size(px(1280.), px(832.)), move |window, cx| {
                let focus = cx.new(|cx| ControlFocus {
                    handle: cx.focus_handle(),
                    keyboard: Cell::new(keyboard),
                });
                let handle = focus.read(cx).handle.clone();
                window.focus(&handle, cx);
                cx.new(|_| InsetOutline { focus })
            })?;
            cx.run_until_parked();
            let image = cx.capture_screenshot(window.into())?;
            for (x, y, expected) in [
                (134, 192, [36, 55, 84, 255]),
                (643, 192, [53, 115, 212, 255]),
                (124, 235, [46, 90, 159, 255]),
                (300, 193, [58, 131, 247, 255]),
                (300, 191, [29, 29, 29, 255]),
                (119, 220, [29, 29, 29, 255]),
                (664, 220, [29, 29, 29, 255]),
                (300, 252, [29, 29, 29, 255]),
            ] {
                let expected = if keyboard {
                    expected
                } else {
                    [29, 29, 29, 255]
                };
                assert_eq!(
                    image.get_pixel(x, y).0,
                    expected,
                    "keyboard={keyboard}, outline ({x}, {y})"
                );
            }
        }
        Ok(())
    }
}
