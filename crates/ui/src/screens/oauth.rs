use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
    Transformation, Window, div, radians,
};
use std::time::Instant;
use theme::ActiveTheme as _;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CallbackState {
    Finishing,
    Superseded,
}

pub struct CallbackPage {
    pub state: CallbackState,
    started: Instant,
}

impl CallbackPage {
    pub fn new(cx: &Context<Self>) -> Self {
        Self {
            state: CallbackState::Finishing,
            started: cx.background_executor().now(),
        }
    }
}

impl Render for CallbackPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let page = div()
            .id("oauth-callback")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.background_surface_under);
        match self.state {
            CallbackState::Finishing => {
                window.request_animation_frame();
                let elapsed = cx.background_executor().now() - self.started;
                let angle = elapsed.as_secs_f32().fract() * std::f32::consts::TAU;
                page.child(
                    crate::icon("sidebar-cmd-spinner", 16., theme.text)
                        .with_transformation(Transformation::rotate(radians(angle))),
                )
            }
            CallbackState::Superseded => page.child(crate::text(
                "This connection attempt was replaced",
                14.,
                21.,
                430.,
                theme.text,
            )),
        }
    }
}
