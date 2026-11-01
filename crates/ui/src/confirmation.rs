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
    /// The left button of [`Confirmation::alternate`]; Escape still cancels.
    Alternate,
}

pub struct Confirmation {
    title: SharedString,
    description: SharedString,
    confirm_label: SharedString,
    cancel_label: SharedString,
    destructive: bool,
    /// The left button is a second, destructive choice instead of Cancel.
    alternate: bool,
    /// One primary button (`confirm_label`) and no Cancel, like `Unable to send message`.
    notice: bool,
    settings_style: bool,
    controls: [FocusHandle; 2],
    return_focus: Option<FocusHandle>,
    completed: bool,
    opt_out: Option<(SharedString, bool, FocusHandle)>,
    documentation: Option<(SharedString, SharedString, FocusHandle)>,
    error: Option<SharedString>,
}

impl EventEmitter<Outcome> for Confirmation {}

impl Focusable for Confirmation {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.controls[self.notice as usize].clone()
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
            alternate: false,
            notice: false,
            settings_style: false,
            controls,
            return_focus,
            completed: false,
            opt_out: None,
            documentation: None,
            error: None,
        }
    }

    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// Shows only the `confirm_label` button and focuses it.
    pub fn notice(mut self, window: &mut Window, cx: &mut Context<Self>) -> Self {
        self.notice = true;
        self.controls[1].focus(window, cx);
        self
    }

    pub fn cancel_label(mut self, label: impl Into<SharedString>) -> Self {
        self.cancel_label = label.into();
        self
    }

    /// Replaces Cancel with a destructive `label` button that emits [`Outcome::Alternate`],
    /// like `Clear queue` beside `Send message`. Enter confirms: the confirm button has focus.
    pub fn alternate(
        mut self,
        label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        self.cancel_label = label.into();
        self.alternate = true;
        self.controls[1].focus(window, cx);
        self
    }

    pub fn with_opt_out(mut self, label: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        self.opt_out = Some((label.into(), false, cx.focus_handle()));
        self
    }

    pub fn with_documentation(
        mut self,
        label: impl Into<SharedString>,
        url: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        self.documentation = Some((label.into(), url.into(), cx.focus_handle()));
        self
    }

    pub fn opt_out_checked(&self) -> bool {
        self.opt_out
            .as_ref()
            .is_some_and(|(_, checked, _)| *checked)
    }

    pub fn failed(
        &mut self,
        error: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.error = Some(error.into());
        self.completed = false;
        self.controls[1].focus(window, cx);
        cx.notify();
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
                let controls = || {
                    self.controls[self.notice as usize..]
                        .iter()
                        .chain(self.opt_out.iter().map(|(_, _, focus)| focus))
                        .chain(self.documentation.iter().map(|(_, _, focus)| focus))
                };
                let count = controls().count();
                let current = controls()
                    .position(|focus| focus.is_focused(window))
                    .unwrap_or(count - 1);
                let next = if key.modifiers.shift {
                    (current + count - 1) % count
                } else {
                    (current + 1) % count
                };
                if let Some(focus) = controls().nth(next) {
                    focus.focus(window, cx);
                }
                cx.notify();
            }
            "enter" | "space"
                if !key.modifiers.platform && !key.modifiers.control && !key.modifiers.alt =>
            {
                if let Some((_, url, focus)) = &self.documentation
                    && focus.is_focused(window)
                {
                    if !event.is_held {
                        cx.open_url(url);
                    }
                    cx.stop_propagation();
                    return;
                }
                if let Some((_, checked, focus)) = &mut self.opt_out
                    && focus.is_focused(window)
                {
                    *checked = !*checked;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                let outcome = if self.controls[1].is_focused(window) {
                    Outcome::Confirm
                } else if self.alternate {
                    Outcome::Alternate
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
        let checkbox = self.opt_out.as_ref().map(|(label, checked, focus)| {
            div()
                .id("confirmation-opt-out")
                .role(gpui::Role::CheckBox)
                .aria_label(label.clone())
                .aria_toggled(if *checked {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .track_focus(focus)
                .h(px(18.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .size(px(14.))
                        .rounded(px(5.))
                        .border_1()
                        .border_color(t.border)
                        .when(*checked, |d| {
                            d.bg(t.text)
                                .child(text("✓", 12., 12., 600., t.menu_background))
                        }),
                )
                .child(text(label.clone(), 13., 19., 430., t.text))
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some((_, checked, focus)) = &mut this.opt_out {
                        *checked = !*checked;
                        focus.focus(window, cx);
                        cx.notify();
                    }
                }))
        });
        let documentation = self.documentation.as_ref().map(|(label, _, focus)| {
            div()
                .id("confirmation-documentation")
                .role(gpui::Role::Link)
                .aria_label(label.clone())
                .track_focus(focus)
                .relative()
                .rounded(px(4.))
                .cursor_pointer()
                .when(
                    focus.is_focused(window) && window.last_input_was_keyboard(),
                    |link| {
                        link.child(shapes::decoration(shapes::Decoration {
                            radius: Corners::all(px(4.)),
                            corner: shapes::Corner::Round,
                            border: Some((px(2.), t.focus_ring)),
                            ..Default::default()
                        }))
                    },
                )
                .child(text(label.clone(), 14., 21., 430., t.text).underline())
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some((_, url, focus)) = &this.documentation {
                        focus.focus(window, cx);
                        cx.open_url(url);
                    }
                }))
        });
        let buttons = [self.cancel_label.clone(), self.confirm_label.clone()]
            .into_iter()
            .enumerate()
            .skip(self.notice as usize)
            .map(|(index, label)| {
                let destructive = if index == 0 {
                    self.alternate
                } else {
                    self.destructive
                };
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
                } else if index == 1 || destructive {
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
                            if index == 1 {
                                Outcome::Confirm
                            } else if this.alternate {
                                Outcome::Alternate
                            } else {
                                Outcome::Cancel
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
                            .children(
                                self.error
                                    .clone()
                                    .map(|error| text(error, 13., 18., 430., t.text_error)),
                            )
                            .children(documentation)
                            .children(checkbox)
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
