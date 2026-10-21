mod artwork;
mod feedback;
mod startup;
pub use feedback::{Event as FeedbackEvent, Feedback};

use crate::{SANS, focus, text};
use gpui::{
    Action, Context, Div, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Window, actions, div, prelude::FluentBuilder as _, px,
};
use std::time::Instant;
use theme::{ActiveTheme as _, Appearance};

actions!(
    recovery,
    [Retry, SendFeedback, ReviewConfiguration, ClosePanel]
);

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Loading,
    RouteError(String),
    TabError,
    ConnectionError(String),
    Crashed { configuration: bool },
}

#[derive(Clone, Copy)]
enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
}

pub struct Recovery {
    state: State,
    started: Instant,
    hovered: Option<&'static str>,
    artwork: artwork::Artwork,
}

impl Recovery {
    pub fn new(state: State, cx: &Context<Self>) -> Self {
        Self {
            state,
            started: cx.background_executor().now(),
            hovered: None,
            artwork: Default::default(),
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn set_state(&mut self, state: State, cx: &mut Context<Self>) {
        self.state = state;
        self.started = cx.background_executor().now();
        self.hovered = None;
        cx.notify();
    }

    fn button(
        &self,
        label: &'static str,
        kind: ButtonKind,
        action: impl Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let theme = cx.theme();
        let (background, foreground, hover) = match kind {
            ButtonKind::Primary => (
                Some(theme.button_primary_solid),
                if theme.appearance == Appearance::Dark {
                    gpui::rgb(0x2d2d2d).into()
                } else {
                    theme.text_inverse
                },
                theme.text.opacity(0.8),
            ),
            ButtonKind::Secondary => (Some(theme.text_alpha_5), theme.text, theme.text_alpha_10),
            ButtonKind::Ghost => (None, theme.text, theme.ghost_hover),
        };
        let compact = self.state == State::TabError;
        let radius = if compact {
            theme::Theme::RADIUS_LG
        } else {
            px(9999.)
        };
        let border = matches!(kind, ButtonKind::Primary).then_some((px(1.), theme.border));
        let handle = focus::handle(label, 0, window, cx);
        let keyboard = action.boxed_clone();
        focus::ring(
            div()
                .id(label)
                .role(gpui::Role::Button)
                .aria_label(label)
                .h(px(if compact { 32. } else { 36. }))
                .px(px(if compact { 13. } else { 21. }))
                .relative()
                .child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(radius),
                    fill: if self.hovered == Some(label) {
                        Some(hover)
                    } else {
                        background
                    },
                    border,
                    ..Default::default()
                }))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.hovered = hovered.then_some(label);
                    cx.notify();
                }))
                .flex()
                .items_center()
                .justify_center()
                .child(text(
                    label,
                    if compact { 13. } else { 14. },
                    18.,
                    430.,
                    foreground,
                )),
            &handle,
            radius,
            px(2.),
            cx,
        )
        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                window.dispatch_action(keyboard.boxed_clone(), cx);
                cx.stop_propagation();
            }
        })
    }
}

impl Render for Recovery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let background: Hsla = match theme.appearance {
            Appearance::Light => gpui::rgb(0xf8f7f5).into(),
            Appearance::Dark => theme.surface_secondary,
        };
        let root = div()
            .size_full()
            .relative()
            .font_family(SANS)
            .on_key_down(focus::tab);
        if self.state == State::Loading {
            let elapsed = cx
                .background_executor()
                .now()
                .saturating_duration_since(self.started);
            let reduced = crate::motion::reduced(cx);
            if !reduced {
                window.request_animation_frame();
            }
            return root
                .bg(match theme.appearance {
                    Appearance::Dark => gpui::rgb(0x121212).into(),
                    Appearance::Light => gpui::white(),
                })
                .flex()
                .items_center()
                .justify_center()
                .child(startup::indicator(elapsed, reduced, &theme))
                .into_any_element();
        }
        if self.state == State::TabError {
            let retry = self.button("Try again", ButtonKind::Secondary, Retry, window, cx);
            let close = self.button("Close panel", ButtonKind::Ghost, ClosePanel, window, cx);
            return root
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .p(px(16.))
                .child(text(
                    "Tab content couldn't render",
                    13.,
                    18.571_428,
                    510.,
                    theme.text,
                ))
                .child(retry)
                .child(close)
                .into_any_element();
        }
        let (description, crashed, configuration) = match &self.state {
            State::RouteError(_) => (
                "Something went wrong. Try again to continue".to_owned(),
                false,
                false,
            ),
            State::ConnectionError(message) => (message.clone(), false, false),
            State::Crashed { configuration } => (
                if *configuration {
                    "ChatGPT couldn't load your configuration. Review it, then restart to try again."
                } else {
                    "Try again or update ChatGPT if this keeps happening"
                }
                .to_owned(),
                true,
                *configuration,
            ),
            State::Loading | State::TabError => unreachable!(),
        };
        let description: SharedString = description.into();
        let mut buttons = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .pt(px(16.))
            .child(self.button("Try again", ButtonKind::Primary, Retry, window, cx));
        if crashed {
            buttons = buttons.child(self.button(
                "Send feedback",
                ButtonKind::Secondary,
                SendFeedback,
                window,
                cx,
            ));
        }
        if configuration {
            buttons = buttons.child(self.button(
                "Review configuration",
                ButtonKind::Secondary,
                ReviewConfiguration,
                window,
                cx,
            ));
        }
        root.bg(background)
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size_full()
                    .max_w(px(896.))
                    .p(px(24.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div().flex_1().min_h_0().flex().items_end().child(
                            div()
                                .relative()
                                .left(px(-8.))
                                .size(px(160.))
                                .child(self.artwork.view(background)),
                        ),
                    )
                    .child(
                        div()
                            .id("recovery-alert")
                            .w(px(448.))
                            .max_w_full()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(12.))
                            .role(gpui::Role::Alert)
                            .text_center()
                            .child(text("ChatGPT hit a snag", 28., 39.2, 600., theme.text).w_full())
                            .child(
                                div()
                                    .max_w_full()
                                    .flex()
                                    .flex_col()
                                    .gap(px(12.))
                                    .text_left()
                                    .child(
                                        text(description, 14., 21., 430., theme.text_secondary)
                                            .whitespace_normal(),
                                    )
                                    .when(crashed, |body| {
                                        body.child(
                                            text(
                                                "Send feedback to help us make the app better.",
                                                14.,
                                                21.,
                                                430.,
                                                theme.text_secondary,
                                            )
                                            .whitespace_normal(),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(24.))
                            .child(buttons),
                    ),
            )
            .into_any_element()
    }
}
