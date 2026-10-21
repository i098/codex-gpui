use crate::{composer::shadow, text};
use gpui::{
    App, Context, Corners, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder as _, px, rgba,
};
use theme::ActiveTheme as _;

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
    controls: [FocusHandle; 2],
    return_focus: Option<FocusHandle>,
    completed: bool,
    opt_out: Option<(SharedString, bool, FocusHandle)>,
    error: Option<SharedString>,
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
            controls,
            return_focus,
            completed: false,
            opt_out: None,
            error: None,
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

    /// Adds the `Don't ask me again` checkbox; keyboard focus starts on it, as in the original.
    pub fn with_opt_out(
        mut self,
        label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.opt_out = Some((label.into(), false, focus));
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
                let count = 2 + usize::from(self.opt_out.is_some());
                let current = self
                    .controls
                    .iter()
                    .position(|focus| focus.is_focused(window))
                    .unwrap_or(2);
                let next = if key.modifiers.shift {
                    (current + count - 1) % count
                } else {
                    (current + 1) % count
                };
                if next < 2 {
                    self.controls[next].focus(window, cx);
                } else if let Some((_, _, focus)) = &self.opt_out {
                    focus.focus(window, cx);
                }
                cx.notify();
            }
            "enter" | "space"
                if !key.modifiers.platform && !key.modifiers.control && !key.modifiers.alt =>
            {
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
        let width = 420_f32.min(f32::from(window.viewport_size().width) - 32.);
        // The dialog's pills: `rounded-[12.5px]` squircles with a 1 px border, 32 px tall.
        let pill = |fill: gpui::Hsla, border: gpui::Hsla| {
            shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(12.5)),
                corner: shapes::Corner::Squircle,
                fill: Some(fill),
                border: Some((px(1.), border)),
                ..Default::default()
            })
        };
        let checkbox = self.opt_out.as_ref().map(|(label, checked, focus)| {
            let ringed = focus.is_focused(window) && window.last_input_was_keyboard();
            let blue: gpui::Hsla = rgba(0x3a83f7ff).into();
            let boxed = shapes::Decoration {
                radius: Corners::all(px(5.)),
                fill: Some(gpui::Hsla {
                    a: 0.96,
                    ..t.menu_background
                }),
                border: Some((px(1.), if ringed { blue } else { t.border_strong })),
                shadows: if ringed {
                    vec![shadow(blue, 0., 0., 2., false)]
                } else {
                    Vec::new()
                },
                ..Default::default()
            };
            div().pt(px(12.)).child(
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
                    .h(px(21.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .relative()
                            .size(px(14.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(shapes::decoration(boxed))
                            .when(*checked, |d| {
                                d.child(crate::icon::icon("menu-check", 12., t.text))
                            }),
                    )
                    .child(text(label.clone(), 14., 21., 430., t.text))
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some((_, checked, focus)) = &mut this.opt_out {
                            *checked = !*checked;
                            focus.focus(window, cx);
                            cx.notify();
                        }
                    })),
            )
        });
        let buttons = [self.cancel_label.clone(), self.confirm_label.clone()]
            .into_iter()
            .enumerate()
            .map(|(index, label)| {
                let destructive = index == 1 && self.destructive;
                let (fill, border, foreground) = if destructive {
                    (t.text_error, t.border, t.text_inverse)
                } else if index == 1 {
                    (t.button_primary_solid, t.border, t.text_inverse)
                } else {
                    (t.text.opacity(0.05), gpui::transparent_black(), t.text)
                };
                div()
                    .id(("confirmation-button", index))
                    .role(gpui::Role::Button)
                    .aria_label(label.clone())
                    .track_focus(&self.controls[index])
                    .h(px(32.))
                    .px(px(17.))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(pill(fill, border))
                    .when(
                        self.controls[index].is_focused(window) && window.last_input_was_keyboard(),
                        |d| {
                            d.child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(px(12.5)),
                                corner: shapes::Corner::Squircle,
                                border: Some((px(2.), t.focus_ring)),
                                ..Default::default()
                            }))
                        },
                    )
                    .child(text(label, 14., 18., 430., foreground))
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
            .child(
                div()
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
                            radius: Corners::all(px(25.)),
                            corner: shapes::Corner::Squircle,
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
                    .child(
                        div()
                            .p(px(20.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(text(self.title.clone(), 20., 28., 600., t.text))
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
                            .children(checkbox)
                            .child(
                                div().pt(px(12.)).child(
                                    div().flex().justify_end().gap(px(12.)).children(buttons),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .id("confirmation-close")
                            .role(gpui::Role::Button)
                            .aria_label("Close dialog")
                            .absolute()
                            .top(px(16.))
                            .right(px(16.))
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(crate::icon::icon("close", 16., t.text.opacity(0.8)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish(Outcome::Cancel, window, cx)
                            })),
                    ),
            )
    }
}
