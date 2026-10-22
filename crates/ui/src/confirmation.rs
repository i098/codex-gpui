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
    hovered_controls: [bool; 2],
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
            hovered_controls: [false; 2],
            return_focus,
            completed: false,
        }
    }

    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    pub fn settings_dialog(mut self) -> Self {
        self.settings_style = true;
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

    fn button_colors(&self, index: usize, t: &Theme) -> (gpui::Hsla, gpui::Hsla, gpui::Hsla) {
        let (fill, foreground) = match (self.settings_style, index == 1, self.destructive) {
            (true, true, true) => (t.text_error.opacity(0.1), t.text_error),
            (false, true, true) => (t.text_error, t.text_inverse),
            (_, true, false) => (t.button_primary_solid, t.text_inverse),
            (true, false, _) => (rgba(0).into(), t.text_tertiary),
            (false, false, _) => (t.button_secondary, t.text),
        };
        let hover = if index == 1 && self.destructive {
            t.text_error.opacity(0.2)
        } else {
            t.ghost_hover
        };
        (fill, foreground, hover)
    }

    fn button(
        &self,
        (index, label): (usize, SharedString),
        window: &Window,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let t = cx.theme();
        let settings = self.settings_style;
        let (fill, foreground, hover) = self.button_colors(index, t);
        let (height, padding, line_height, weight, radius, corner) = if settings {
            (
                32.,
                17.,
                18.,
                430.,
                Theme::RADIUS_LG,
                shapes::Corner::Squircle,
            )
        } else {
            (
                36.,
                14.,
                20.,
                500.,
                Theme::RADIUS_FULL,
                shapes::Corner::Round,
            )
        };
        div()
            .id(("confirmation-button", index))
            .role(gpui::Role::Button)
            .aria_label(label.clone())
            .track_focus(&self.controls[index])
            .h(px(height))
            .px(px(padding))
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .when(!settings, |d| d.rounded(radius).bg(fill))
            .when(settings, |d| {
                d.child(shapes::decoration(shapes::Decoration {
                    radius: Corners::all(radius),
                    corner,
                    fill: Some(if self.hovered_controls[index] {
                        hover
                    } else {
                        fill
                    }),
                    ..Default::default()
                }))
                .on_hover(cx.listener(move |this, hovered, _, cx| {
                    this.hovered_controls[index] = *hovered;
                    cx.notify();
                }))
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
            .child(text(label, 14., line_height, weight, foreground))
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
    }
}

impl Render for Confirmation {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let settings = self.settings_style;
        let viewport_width = f32::from(window.viewport_size().width);
        let width = if settings {
            520_f32.min(viewport_width * 0.92)
        } else {
            420_f32.min(viewport_width - 32.)
        };
        let buttons = [self.cancel_label.clone(), self.confirm_label.clone()]
            .into_iter()
            .enumerate()
            .map(|entry| self.button(entry, window, cx));
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
                            radius: Corners::all(px(if settings { 25. } else { 24. })),
                            corner: if settings {
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
                    .child(
                        div()
                            .p(px(20.))
                            .flex()
                            .flex_col()
                            .gap(px(if settings { 12. } else { 20. }))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(12.))
                                    .child(text(
                                        self.title.clone(),
                                        20.,
                                        if settings { 28. } else { 26. },
                                        if settings { 600. } else { 500. },
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
                                    .gap(px(if settings { 12. } else { 8. }))
                                    .children(buttons),
                            ),
                    ),
            )
    }
}
