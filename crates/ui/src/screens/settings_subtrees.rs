use super::{Layer, deco, probe};
use crate::{MONO, focus, icon::icon, rounded_clip, text};
use gpui::{
    Action, App, AppContext as _, Context, Entity, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use shapes::Corner::Squircle;
use theme::ActiveTheme as _;

#[derive(Clone, PartialEq, Action)]
#[action(namespace = settings, no_json)]
pub struct OpenSubtree {
    pub path: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = settings, no_json)]
pub struct OpenSection {
    pub label: String,
}

pub const LICENSES_PATH: &str = "/settings/open-source-licenses";
pub const GENERAL_PATH: &str = "/settings/general";

pub fn licenses(
    load: impl Future<Output = Result<Option<String>, String>> + Send + 'static,
    cx: &mut App,
) -> Entity<Licenses> {
    cx.new(|cx: &mut Context<Licenses>| {
        let read = cx.background_spawn(load);
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                this.text = Some(match result {
                    Ok(text) => text.map(SharedString::from),
                    Err(error) => {
                        eprintln!("third-party-notices: {error}");
                        None
                    }
                });
                cx.notify();
            });
        })
        .detach();
        Licenses {
            text: None,
            layer: Layer::default(),
        }
    })
}

pub struct Licenses {
    text: Option<Option<SharedString>>,
    layer: Layer,
}

impl Render for Licenses {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let back_focus = focus::handle("licenses-back", 0, window, cx);
        let back = focus::ring(
            div()
                .id("licenses-back")
                .role(gpui::Role::Button)
                .aria_label("Back")
                .h(px(28.))
                .px(px(9.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon("chevron-left", 16., t.text))
                .child(text("Back", 13., 18., 430., t.text)),
            &back_focus,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(|_, window, cx| {
            window.dispatch_action(
                Box::new(OpenSubtree {
                    path: GENERAL_PATH.into(),
                }),
                cx,
            );
        })
        .on_key_down(|event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                window.dispatch_action(
                    Box::new(OpenSubtree {
                        path: GENERAL_PATH.into(),
                    }),
                    cx,
                );
                cx.stop_propagation();
            }
        });
        let body = match &self.text {
            None => div()
                .p(px(16.))
                .child(text("Loading…", 14., 20., 430., t.text_secondary)),
            Some(Some(contents)) if !contents.is_empty() => {
                div().rounded(px(4.)).p(px(12.)).font_family(MONO).child(
                    text(contents.clone(), 12., 19.5, 400., t.text)
                        .font_family(MONO)
                        .whitespace_normal(),
                )
            }
            Some(_) => div().p(px(16.)).child(text(
                "No third-party notices were found.",
                14.,
                20.,
                430.,
                t.text_secondary,
            )),
        };
        let body = if self.text.is_some() {
            div()
                .relative()
                .p(px(1.))
                .child(deco(
                    &self.layer,
                    20.,
                    Squircle,
                    t.background_panel,
                    Some(t.border),
                ))
                .child(rounded_clip(px(19.), Squircle, body))
        } else {
            body
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().h(px(46.)).px(px(20.)).flex().items_center().child(back))
            .child(div()
                .id("licenses-page")
                .flex_1()
                .overflow_y_scroll()
                .layer()
                .child(probe(&self.layer))
                .child(div().p(px(20.)).flex().flex_col().items_center().child(
                    div().w_full().max_w(px(768.))
                        .child(div().pb(px(32.)).flex().flex_col().gap(px(6.))
                            .child(text("Open source licenses", 24., 28.8, 400., t.text))
                            .child(text("Third-party notices for dependencies included in this app", 14., 20., 400., t.text_secondary)))
                        .child(body))))
    }
}
