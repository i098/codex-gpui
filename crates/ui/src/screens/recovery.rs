use crate::{SANS, focus, icon, text};
use gpui::{
    Action, Context, Div, Hsla, Image, ImageFormat, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Transformation, Window, actions, div, img, px, radians, prelude::FluentBuilder as _,
};
use std::{sync::{Arc, LazyLock}, time::{Duration, Instant}};
use theme::{ActiveTheme as _, Appearance};

actions!(recovery, [Retry, SendFeedback, ReviewConfiguration, ClosePanel]);

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Loading,
    RouteError(String),
    TabError,
    ConnectionError(String),
    Crashed { configuration: bool, details: String },
}

pub struct Recovery {
    state: State,
    started: Instant,
}

impl Recovery {
    pub fn new(state: State, cx: &Context<Self>) -> Self {
        Self { state, started: cx.background_executor().now() }
    }

    pub fn state(&self) -> &State { &self.state }

    pub fn set_state(&mut self, state: State, cx: &mut Context<Self>) {
        self.state = state;
        self.started = cx.background_executor().now();
        cx.notify();
    }

    fn button(&self, label: &'static str, primary: bool, action: impl Action, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let (background, foreground, hover) = if primary {
            (theme.button_primary_solid, theme.text_inverse, theme.button_primary_solid_hover)
        } else {
            (theme.button_secondary, theme.text, theme.button_secondary_hover)
        };
        let handle = focus::handle(label, 0, window, cx);
        let keyboard = action.boxed_clone();
        focus::ring(div().id(label).role(gpui::Role::Button).aria_label(label)
            .h(px(40.)).px(px(16.)).rounded(px(12.5)).bg(background)
            .hover(move |style| style.bg(hover)).flex().items_center().justify_center()
            .child(text(label, 14., 20., 510., foreground)), &handle, px(12.5), px(2.), cx)
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
        let root = div().size_full().relative().font_family(SANS).on_key_down(focus::tab);
        if self.state == State::Loading {
            let elapsed = cx.background_executor().now().saturating_duration_since(self.started);
            window.request_animation_frame();
            return root.bg(theme.background_surface_under).flex().items_center().justify_center()
                .when(elapsed >= Duration::from_millis(400), |root| root.child(
                    icon("sidebar-cmd-spinner", 20., theme.text_secondary)
                        .with_transformation(Transformation::rotate(radians(elapsed.as_secs_f32() * std::f32::consts::TAU))),
                )).into_any_element();
        }
        if self.state == State::TabError {
            let retry = self.button("Try again", false, Retry, window, cx);
            let close = self.button("Close panel", false, ClosePanel, window, cx);
            return root.flex().flex_col().items_center().justify_center().gap(px(12.)).p(px(16.))
                .child(text("Tab content couldn't render", 14., 20., 510., theme.text))
                .child(retry).child(close).into_any_element();
        }
        let (description, details, crashed, configuration) = match &self.state {
            State::RouteError(_) => ("Something went wrong. Try again to continue".to_owned(), "", false, false),
            State::ConnectionError(message) => (message.clone(), "", false, false),
            State::Crashed { configuration, details } => (
                if *configuration { "ChatGPT couldn't load your configuration. Review it, then restart to try again." } else { "Try again or update ChatGPT if this keeps happening" }.to_owned(),
                details.as_str(), true, *configuration,
            ),
            State::Loading | State::TabError => unreachable!(),
        };
        let description: SharedString = description.into();
        let details: SharedString = details.to_owned().into();
        let mut buttons = div().flex().flex_wrap().items_center().justify_center().gap(px(12.)).pt(px(16.))
            .child(self.button("Try again", true, Retry, window, cx));
        if crashed {
            buttons = buttons.child(self.button("Send feedback", false, SendFeedback, window, cx));
        }
        if configuration {
            buttons = buttons.child(self.button("Review configuration", false, ReviewConfiguration, window, cx));
        }
        static ART: LazyLock<Arc<Image>> = LazyLock::new(|| Arc::new(Image::from_bytes(
            ImageFormat::Png, include_bytes!("../../assets/recovery-pet.png").to_vec(),
        )));
        root.bg(background).flex().items_center().justify_center()
            .child(div().size_full().max_w(px(896.)).p(px(24.)).flex().flex_col().items_center().gap(px(8.))
                .child(div().flex_1().min_h_0().flex().items_end().child(
                    div().relative().left(px(-8.)).size(px(160.)).child(img(ART.clone()).size_full()),
                ))
                .child(div().id("recovery-alert").max_w(px(448.)).flex_none().flex().flex_col().items_center().gap(px(12.)).role(gpui::Role::Alert)
                    .child(text("ChatGPT hit a snag", 20., 28., 600., theme.text))
                    .child(text(description, 16., 24., 430., theme.text_secondary))
                    .when(crashed, |body| body.child(text("Send feedback to help us make the app better.", 16., 24., 430., theme.text_secondary))))
                .child(div().flex_1().min_h_0().flex().flex_col().items_center().gap(px(24.)).child(buttons)
                    .when(!details.is_empty(), |body| body.child(
                        div().id("recovery-details").max_w(px(640.)).max_h(px(160.)).overflow_y_scroll().font_family(crate::MONO)
                            .child(text(details, 12., 18., 400., theme.text_tertiary)),
                    ))))
            .into_any_element()
    }
}
