use super::*;
use gpui::{DispatchPhase, HitboxBehavior, ScrollHandle, WeakEntity, canvas};

#[derive(Default)]
pub(super) struct State {
    pub height: Option<Pixels>,
    pub scroll: ScrollHandle,
    resize: Option<(Pixels, Pixels)>,
    caret: Option<(Point<Pixels>, Size<Pixels>)>,
}

impl State {
    pub fn reveal(&mut self, caret: Point<Pixels>, line_height: Pixels, window: &mut Window) {
        let viewport = self.scroll.bounds().size;
        if viewport.width <= px(0.)
            || viewport.height <= px(0.)
            || self.caret == Some((caret, viewport))
        {
            return;
        }
        self.caret = Some((caret, viewport));
        let previous = self.scroll.offset();
        let mut offset = previous;
        if caret.x < -offset.x {
            offset.x = -caret.x;
        } else if caret.x + px(1.) > -offset.x + viewport.width {
            offset.x = viewport.width - caret.x - px(1.);
        }
        if caret.y < -offset.y {
            offset.y = -caret.y;
        } else if caret.y + line_height > -offset.y + viewport.height {
            offset.y = viewport.height - caret.y - line_height;
        }
        offset.x = offset.x.min(px(0.));
        offset.y = offset.y.min(px(0.));
        if offset != previous {
            self.scroll.set_offset(offset);
            window.request_animation_frame();
        }
    }

    pub fn blur(&mut self, window: &mut Window) {
        self.caret = None;
        let mut offset = self.scroll.offset();
        if offset.x != px(0.) {
            offset.x = px(0.);
            self.scroll.set_offset(offset);
            window.request_animation_frame();
        }
    }
}

pub(super) fn grip(view: WeakEntity<Composer>, height: Pixels) -> impl IntoElement {
    div()
        .absolute()
        .right(px(1.))
        .bottom(px(1.))
        .w(px(16.))
        .h(px(16.))
        .child(
            icon("textarea-resize", 16., gpui::rgb(0x111111).into())
                .absolute()
                .top(px(-1.5))
                .left(px(-0.5)),
        )
        .child(
            icon("textarea-resize", 16., gpui::rgb(0xaaaaaa).into())
                .absolute()
                .top(px(-0.5))
                .left(px(-0.5)),
        )
        .child(
            canvas(
                |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                move |_, hitbox, window, _| {
                    window.set_cursor_style(CursorStyle::ResizeUpDown, &hitbox);
                    let down_view = view.clone();
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble
                            || event.button != MouseButton::Left
                            || !hitbox.is_hovered(window)
                        {
                            return;
                        }
                        window.capture_pointer(hitbox.id);
                        let _ = down_view.update(cx, |composer, cx| {
                            if let Some(field) = composer.settings_input.as_mut() {
                                field.resize = Some((event.position.y, height));
                                window.focus(&composer.focus_handle, cx);
                                cx.notify();
                            }
                        });
                        cx.stop_propagation();
                    });
                    let move_view = view.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble {
                            return;
                        }
                        let handled = move_view
                            .update(cx, |composer, cx| {
                                let Some(field) = composer.settings_input.as_mut() else {
                                    return false;
                                };
                                let Some((start, height)) = field.resize else {
                                    return false;
                                };
                                let height = (height + event.position.y - start).max(px(96.));
                                if field.height != Some(height) {
                                    field.height = Some(height);
                                    cx.notify();
                                }
                                true
                            })
                            .unwrap_or(false);
                        if handled {
                            cx.stop_propagation();
                        }
                    });
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                        if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                            return;
                        }
                        let handled = view
                            .update(cx, |composer, cx| {
                                let handled = composer
                                    .settings_input
                                    .as_mut()
                                    .is_some_and(|field| field.resize.take().is_some());
                                if handled {
                                    cx.notify();
                                }
                                handled
                            })
                            .unwrap_or(false);
                        if handled {
                            cx.stop_propagation();
                        }
                    });
                },
            )
            .absolute()
            .size_full(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, Modifiers, PlatformInput, TestAppContext};

    #[gpui::test]
    fn resizing_clamps_height_and_keeps_the_caret_visible(cx: &mut TestAppContext) {
        cx.set_global(Theme::dark());
        let window = cx.open_window(size(px(300.), px(300.)), |_, cx| {
            Composer::settings_field("", "", true, cx).with_field_rows(4)
        });
        cx.run_until_parked();
        let initial = window
            .update(cx, |composer, _, _| {
                composer
                    .settings_input
                    .as_ref()
                    .unwrap()
                    .scroll
                    .bounds()
                    .size
                    .height
            })
            .unwrap();
        assert_eq!(initial, px(78.));
        for (from, to, expected) in [(92., 152., initial + px(60.)), (152., -100., px(78.))] {
            for event in [
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: point(px(294.), px(from)),
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
                PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position: point(px(294.), px(from)),
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: point(px(294.), px(to)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::default(),
                }),
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position: point(px(294.), px(to)),
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }),
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: point(px(294.), px(250.)),
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
            ] {
                cx.update_window(window.into(), |_, window, cx| {
                    window.dispatch_event(event, cx);
                })
                .unwrap();
                cx.run_until_parked();
            }
            let actual = window
                .update(cx, |composer, _, _| {
                    let field = composer.settings_input.as_ref().unwrap();
                    field.scroll.bounds().size.height
                })
                .unwrap();
            assert_eq!(actual, expected);
        }
        window
            .update(cx, |composer, window, cx| {
                window.focus(&composer.focus_handle, cx);
                composer.set_text(&"line\n".repeat(20), cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |composer, _, _| {
                let field = composer.settings_input.as_ref().unwrap();
                let caret = composer.point_for_index(composer.cursor()).unwrap().y;
                let top = caret + field.scroll.offset().y;
                assert!(top >= px(-1.));
                assert!(
                    top + composer.editor_line_height()
                        <= field.scroll.bounds().size.height + px(1.)
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn long_single_line_values_keep_the_caret_inside_the_field(cx: &mut TestAppContext) {
        cx.set_global(Theme::dark());
        let value = "/long/worktree/path".repeat(30);
        let window = cx.open_window(size(px(300.), px(100.)), |_, cx| {
            Composer::settings_field("", &value, false, cx)
        });
        cx.run_until_parked();
        window
            .update(cx, |composer, window, cx| {
                window.focus(&composer.focus_handle, cx);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |composer, _, _| {
                let field = composer.settings_input.as_ref().unwrap();
                let caret = composer.point_for_index(composer.cursor()).unwrap().x;
                let left = caret + field.scroll.offset().x;
                assert!(field.scroll.offset().x < px(0.));
                assert!(left >= px(0.));
                assert!(left + px(1.) <= field.scroll.bounds().size.width + px(1.));
                assert_eq!(composer.field_text(), value);
            })
            .unwrap();
        window
            .update(cx, |_, window, cx| {
                window.blur(cx);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |composer, _, _| {
                assert_eq!(
                    composer.settings_input.as_ref().unwrap().scroll.offset().x,
                    px(0.)
                );
                assert_eq!(composer.field_text(), value);
            })
            .unwrap();
    }
}
