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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Appearance {
    Standard,
    Destructive,
    Warning,
}

pub struct Confirmation {
    title: SharedString,
    description: SharedString,
    confirm_label: SharedString,
    cancel_label: SharedString,
    appearance: Appearance,
    controls: [FocusHandle; 2],
    return_focus: Option<FocusHandle>,
    completed: bool,
    link: Option<(SharedString, SharedString, FocusHandle)>,
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
            appearance: Appearance::Standard,
            controls,
            return_focus,
            completed: false,
            link: None,
        }
    }

    pub fn destructive(mut self) -> Self {
        self.appearance = Appearance::Destructive;
        self
    }

    pub fn warning(mut self) -> Self {
        self.appearance = Appearance::Warning;
        self
    }

    pub fn cancel_label(mut self, label: impl Into<SharedString>) -> Self {
        self.cancel_label = label.into();
        self
    }

    pub fn link(
        mut self,
        label: impl Into<SharedString>,
        url: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        self.link = Some((label.into(), url.into(), cx.focus_handle()));
        self
    }

    fn warning_description(&self, width: f32, window: &Window, t: &Theme) -> gpui::Div {
        let (before, last) = self
            .description
            .rsplit_once("\n\n")
            .unwrap_or(("", &self.description));
        let body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .when(!before.is_empty(), |body| {
                body.child(
                    text(before.to_owned(), 14., 21., 430., t.text_tertiary).whitespace_normal(),
                )
            });
        let Some((label, url, focus)) = &self.link else {
            return body
                .child(text(last.to_owned(), 14., 21., 430., t.text_tertiary).whitespace_normal());
        };
        let start = last.len() + 1;
        let content: SharedString = format!("{last} {}", label.replace(' ', "\u{a0}")).into();
        let mut font = gpui::font(crate::SANS);
        font.weight = gpui::FontWeight(430.);
        let run = gpui::TextRun {
            len: content.len(),
            font,
            color: t.text_tertiary,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let lines = window
            .text_system()
            .shape_text(content.clone(), px(14.), &[run], Some(px(width)), None)
            .expect("the bundled UI font shapes warning text");
        let line = lines.first().expect("warning text contains a link");
        let link_start = line
            .position_for_index(start, px(21.))
            .expect("the link starts in the paragraph");
        let link_end = line
            .position_for_index(content.len(), px(21.))
            .expect("the link ends in the paragraph");
        let styled = gpui::StyledText::new(content.clone()).with_highlights([(
            start..content.len(),
            gpui::HighlightStyle {
                color: Some(t.text_accent),
                ..Default::default()
            },
        )]);
        let url = url.clone();
        let ring = t.focus_ring;
        body.child(
            div()
                .relative()
                .text_size(px(14.))
                .line_height(px(21.))
                .font_family(crate::SANS)
                .font_weight(gpui::FontWeight(430.))
                .text_color(t.text_tertiary)
                .child(styled)
                .child(
                    div()
                        .id("confirmation-inline-link")
                        .absolute()
                        .left(link_start.x)
                        .top(link_start.y)
                        .w((link_end.x - link_start.x).max(px(1.)))
                        .h(px(21.))
                        .role(gpui::Role::Link)
                        .aria_label(label.clone())
                        .track_focus(focus)
                        .focus_visible(move |style| style.border_2().border_color(ring))
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                ),
        )
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
                let mut controls = self
                    .controls
                    .iter()
                    .chain(self.link.iter().map(|(_, _, focus)| focus));
                let count = controls.clone().count();
                let current = controls
                    .clone()
                    .position(|focus| focus.is_focused(window))
                    .unwrap_or(0);
                let index = (current + if key.modifiers.shift { count - 1 } else { 1 }) % count;
                controls.nth(index).unwrap().focus(window, cx);
                cx.notify();
            }
            "enter" | "space"
                if !key.modifiers.platform && !key.modifiers.control && !key.modifiers.alt =>
            {
                if let Some((_, _, focus)) = &self.link
                    && focus.is_focused(window)
                {
                    if key.key == "space" {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
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
        let warning = self.appearance == Appearance::Warning;
        let width = if warning {
            520_f32.min(f32::from(window.viewport_size().width) * 0.92)
        } else {
            420_f32.min(f32::from(window.viewport_size().width) - 32.)
        };
        let radius = if warning { px(25.) } else { px(24.) };
        let title = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .children(warning.then(|| crate::icon::icon("settings-warning", 18., t.text)))
            .child(text(
                self.title.clone(),
                20.,
                if warning { 28. } else { 26. },
                if warning { 600. } else { 500. },
                t.text,
            ));
        let message = if warning {
            self.warning_description(width - 40., window, &t)
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    text(self.description.clone(), 14., 21., 430., t.text_tertiary)
                        .whitespace_normal(),
                )
                .children(self.link.clone().map(|(label, url, focus)| {
                    div()
                        .id("confirmation-learn-more")
                        .role(gpui::Role::Link)
                        .track_focus(&focus)
                        .text_color(t.text_accent)
                        .child(text(label, 14., 21., 430., t.text_accent))
                        .on_click(move |_, _, cx| cx.open_url(&url))
                }))
                .into_any_element()
        };
        let buttons = [self.cancel_label.clone(), self.confirm_label.clone()]
            .into_iter()
            .enumerate()
            .map(|(index, label)| {
                let destructive = index == 1 && self.appearance == Appearance::Destructive;
                let fill = if warning {
                    if index == 1 {
                        t.text_error.opacity(0.1)
                    } else {
                        t.text.opacity(0.05)
                    }
                } else if destructive {
                    t.text_error
                } else if index == 1 {
                    t.button_primary_solid
                } else {
                    t.button_secondary
                };
                let foreground = if warning && index == 1 {
                    t.text_error
                } else if index == 1 && !warning {
                    t.text_inverse
                } else {
                    t.text
                };
                div()
                    .id(("confirmation-button", index))
                    .role(gpui::Role::Button)
                    .aria_label(label.clone())
                    .track_focus(&self.controls[index])
                    .h(px(if warning { 32. } else { 36. }))
                    .px(px(if warning { 17. } else { 14. }))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(if warning {
                        px(12.5)
                    } else {
                        Theme::RADIUS_FULL
                    })
                    .bg(fill)
                    .gap(px(4.))
                    .when(
                        self.controls[index].is_focused(window) && window.last_input_was_keyboard(),
                        |d| {
                            d.child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(if warning {
                                    px(12.5)
                                } else {
                                    Theme::RADIUS_FULL
                                }),
                                border: Some((px(2.), t.focus_ring)),
                                ..Default::default()
                            }))
                        },
                    )
                    .children(
                        (warning && index == 1)
                            .then(|| crate::icon::icon("settings-warning", 16., foreground)),
                    )
                    .child(text(
                        label,
                        14.,
                        if warning { 18. } else { 20. },
                        if warning { 430. } else { 500. },
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
                            radius: Corners::all(radius),
                            corner: if warning {
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
                            .gap(px(if warning { 12. } else { 20. }))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(12.))
                                    .child(title)
                                    .child(message),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap(px(if warning { 12. } else { 8. }))
                                    .children(buttons),
                            ),
                    ),
            )
    }
}
