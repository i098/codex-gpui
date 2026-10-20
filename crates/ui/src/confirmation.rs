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
        let width = 420_f32.min(f32::from(window.viewport_size().width) - 32.);
        let buttons = [self.cancel_label.clone(), self.confirm_label.clone()]
            .into_iter()
            .enumerate()
            .map(|(index, label)| {
                let destructive = index == 1 && self.destructive;
                let fill = if destructive {
                    t.text_error
                } else if index == 1 {
                    t.button_primary_solid
                } else {
                    t.button_secondary
                };
                let foreground = if index == 1 { t.text_inverse } else { t.text };
                div()
                    .id(("confirmation-button", index))
                    .role(gpui::Role::Button)
                    .aria_label(label.clone())
                    .track_focus(&self.controls[index])
                    .h(px(36.))
                    .px(px(14.))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(fill)
                    .when(
                        self.controls[index].is_focused(window) && window.last_input_was_keyboard(),
                        |d| {
                            d.child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(Theme::RADIUS_FULL),
                                border: Some((px(2.), t.focus_ring)),
                                ..Default::default()
                            }))
                        },
                    )
                    .child(text(label, 14., 20., 500., foreground))
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
                            radius: Corners::all(px(24.)),
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
                            .gap(px(20.))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(12.))
                                    .child(text(self.title.clone(), 20., 26., 500., t.text))
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
                            .child(div().flex().justify_end().gap(px(8.)).children(buttons)),
                    ),
            )
    }
}
