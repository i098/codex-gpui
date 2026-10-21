use crate::{composer::shadow, text};
use gpui::{
    App, Context, Corners, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder as _, px, rgba,
};
use theme::{ActiveTheme as _, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Confirm,
    Cancel,
}

pub struct Confirmation {
    title: SharedString,
    description: SharedString,
    confirm_label: SharedString,
    cancel_label: SharedString,
    destructive: bool,
    settings_style: bool,
    controls: [FocusHandle; 2],
    return_focus: Option<FocusHandle>,
    completed: bool,
}

impl EventEmitter<Outcome> for Confirmation {}

impl Focusable for Confirmation {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.controls[0].clone()
    }
}

impl Confirmation {
    pub fn new(
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        confirm_label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let return_focus = window.focused(cx);
        let controls = std::array::from_fn(|_| cx.focus_handle());
        controls[0].focus(window, cx);
        Self {
            title: title.into(),
            description: description.into(),
            confirm_label: confirm_label.into(),
            cancel_label: "Cancel".into(),
            destructive: false,
            settings_style: false,
            controls,
            return_focus,
            completed: false,
        }
    }

    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    pub fn cancel_label(mut self, label: impl Into<SharedString>) -> Self {
        self.cancel_label = label.into();
        self
    }

    pub fn settings_style(mut self) -> Self {
        self.settings_style = true;
        self
    }

    fn finish(&mut self, outcome: Outcome, window: &mut Window, cx: &mut Context<Self>) {
        if self.completed {
            return;
        }
        self.completed = true;
        if let Some(focus) = &self.return_focus {
            focus.focus(window, cx);
        }
        cx.emit(outcome);
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        match key.key.as_str() {
            "escape" => self.finish(Outcome::Cancel, window, cx),
            "tab" if !key.modifiers.platform && !key.modifiers.control && !key.modifiers.alt => {
                let index = usize::from(self.controls[0].is_focused(window));
                self.controls[index].focus(window, cx);
                cx.notify();
            }
            "enter" | "space"
                if !key.modifiers.platform && !key.modifiers.control && !key.modifiers.alt =>
            {
                let outcome = if self.controls[1].is_focused(window) {
                    Outcome::Confirm
                } else {
                    Outcome::Cancel
                };
                self.finish(outcome, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for Confirmation {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let width = (if self.settings_style {
            520_f32
        } else {
            420_f32
        })
        .min(
            f32::from(window.viewport_size().width) * if self.settings_style { 0.92 } else { 1. }
                - if self.settings_style { 0. } else { 32. },
        );
        let buttons = [self.cancel_label.clone(), self.confirm_label.clone()]
            .into_iter()
            .enumerate()
            .map(|(index, label)| {
                let destructive = index == 1 && self.destructive;
                let fill = if self.settings_style {
                    if destructive {
                        t.text_error.opacity(0.1)
                    } else if index == 0 {
                        gpui::transparent_black()
                    } else {
                        t.button_primary_solid
                    }
                } else if destructive {
                    t.text_error
                } else if index == 1 {
                    t.button_primary_solid
                } else {
                    t.button_secondary
                };
                let foreground = if self.settings_style {
                    if destructive {
                        t.text_error
                    } else if index == 0 {
                        t.text_tertiary
                    } else {
                        t.text_inverse
                    }
                } else if index == 1 {
                    t.text_inverse
                } else {
                    t.text
                };
                let radius = if self.settings_style {
                    px(12.5)
                } else {
                    Theme::RADIUS_FULL
                };
                let corner = if self.settings_style {
                    shapes::Corner::Squircle
                } else {
                    shapes::Corner::Round
                };
                div()
                    .id(("confirmation-button", index))
                    .role(gpui::Role::Button)
                    .aria_label(label.clone())
                    .track_focus(&self.controls[index])
                    .h(px(if self.settings_style { 32. } else { 36. }))
                    .px(px(if self.settings_style { 16. } else { 14. }))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(self.settings_style, |button| {
                        button
                            .px(px(17.))
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(radius),
                                corner,
                                fill: Some(fill),
                                ..Default::default()
                            }))
                    })
                    .when(!self.settings_style, |button| {
                        button.rounded_full().bg(fill)
                    })
                    .when(
                        self.controls[index].is_focused(window) && window.last_input_was_keyboard(),
                        |d| {
                            d.child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(radius),
                                corner,
                                border: Some((px(2.), t.focus_ring)),
                                ..Default::default()
                            }))
                        },
                    )
                    .child(text(
                        label,
                        14.,
                        if self.settings_style { 18. } else { 20. },
                        if self.settings_style { 430. } else { 500. },
                        foreground,
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.finish(
                            if index == 0 {
                                Outcome::Cancel
                            } else {
                                Outcome::Confirm
                            },
                            window,
                            cx,
                        );
                    }))
            });
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000022))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.finish(Outcome::Cancel, window, cx)),
            )
            .child({
                let dialog = div()
                    .id("confirmation-dialog")
                    .role(gpui::Role::Dialog)
                    .aria_label(self.title.clone())
                    .w(px(width))
                    .relative()
                    .capture_key_down(cx.listener(Self::key))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(shapes::backdrop_blur(
                        px(24.),
                        shapes::Fill::Color,
                        false,
                        shapes::Decoration {
                            radius: Corners::all(px(if self.settings_style { 25. } else { 24. })),
                            corner: if self.settings_style {
                                shapes::Corner::Squircle
                            } else {
                                shapes::Corner::Round
                            },
                            fill: Some(gpui::Hsla {
                                a: 0.9,
                                ..t.menu_background
                            }),
                            shadows: vec![
                                shadow(t.border, 0., 0., 0.5, false),
                                shadow(rgba(0x0000001a).into(), 4., 8., -2., false),
                            ],
                            ..Default::default()
                        },
                    ))
                    .child({
                        let content = div()
                            .p(px(20.))
                            .flex()
                            .flex_col()
                            .gap(px(if self.settings_style { 12. } else { 20. }))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(12.))
                                    .child(text(
                                        self.title.clone(),
                                        20.,
                                        if self.settings_style { 28. } else { 26. },
                                        if self.settings_style { 600. } else { 500. },
                                        t.text,
                                    ))
                                    .child(
                                        text(
                                            self.description.clone(),
                                            14.,
                                            21.,
                                            430.,
                                            t.text_tertiary,
                                        )
                                        .whitespace_normal(),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap(px(if self.settings_style { 12. } else { 8. }))
                                    .children(buttons),
                            );
                        if self.settings_style {
                            crate::rounded_clip(px(25.), shapes::Corner::Squircle, content)
                                .into_any_element()
                        } else {
                            content.into_any_element()
                        }
                    });
                if self.settings_style {
                    // Chromium's layer starts at the shadow ink, 10px left and 6px above the box.
                    div()
                        .ml(px(-10.))
                        .mt(px(-6.))
                        .pl(px(10.))
                        .pt(px(6.))
                        .layer()
                        .child(dialog)
                        .into_any_element()
                } else {
                    dialog.into_any_element()
                }
            })
    }
}
