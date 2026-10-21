use crate::{MONO, focus, icon::icon, text};
use gpui::{
    Action, App, AppContext as _, Bounds, ContentMask, Context, Corners, Div, Entity,
    InteractiveElement, IntoElement, KeyDownEvent, ListAlignment, ListState, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, canvas, div, list, point,
    prelude::FluentBuilder as _, px, size,
};
use std::sync::Arc;
use theme::{ActiveTheme as _, Theme};

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
        let read = cx.background_spawn(async move {
            load.await.map(|contents| {
                contents.filter(|text| !text.is_empty()).map(|text| {
                    text.split('\n')
                        .map(|line| SharedString::from(line.trim_end_matches('\r').to_owned()))
                        .collect::<Vec<_>>()
                        .into()
                })
            })
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                this.text = Some(match result {
                    Ok(lines) => lines,
                    Err(error) => {
                        eprintln!("third-party-notices: {error}");
                        None
                    }
                });
                let count = this
                    .text
                    .as_ref()
                    .and_then(Option::as_ref)
                    .map_or(1, |lines| lines.len());
                this.list.reset_with_uniform_height(count + 2, px(19.5));
                cx.notify();
            });
        })
        .detach();
        Licenses {
            text: None,
            list: ListState::new(3, ListAlignment::Top, px(200.)),
        }
    })
}

pub struct Licenses {
    text: Option<Option<Arc<[SharedString]>>>,
    list: ListState,
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
        let contents = self.text.clone();
        let count = contents
            .as_ref()
            .and_then(Option::as_ref)
            .map_or(1, |lines| lines.len());
        let rows = list(self.list.clone(), move |index, _, _| {
            if index == 0 {
                return column(
                    div()
                        .pt(px(20.))
                        .pb(px(32.))
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(text("Open source licenses", 24., 28.8, 400., t.text))
                        .child(text(
                            "Third-party notices for dependencies included in this app",
                            14.,
                            20.,
                            400.,
                            t.text_secondary,
                        )),
                )
                .into_any_element();
            }
            if index > count {
                return div().h(px(20.)).into_any_element();
            }
            let content = match &contents {
                None => div()
                    .p(px(16.))
                    .child(text("Loading…", 14., 20., 430., t.text_secondary)),
                Some(None) => notice_row(
                    "No third-party notices were found.".into(),
                    true,
                    true,
                    false,
                    &t,
                ),
                Some(Some(lines)) => notice_row(
                    lines[index - 1].clone(),
                    index == 1,
                    index == count,
                    true,
                    &t,
                ),
            };
            column(content).into_any_element()
        })
        .size_full();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .h(px(46.))
                    .px(px(20.))
                    .flex()
                    .items_center()
                    .child(back),
            )
            .child(div().id("licenses-page").min_h_0().flex_1().child(rows))
    }
}

fn column(content: Div) -> Div {
    div()
        .w_full()
        .px(px(20.))
        .flex()
        .justify_center()
        .child(div().w_full().max_w(px(768.)).child(content))
}

fn notice_row(line: SharedString, first: bool, last: bool, monospace: bool, theme: &Theme) -> Div {
    let fill = theme.background_panel;
    let border = theme.border;
    let content = if monospace {
        text(line, 12., 19.5, 400., theme.text)
            .font_family(MONO)
            .whitespace_normal()
    } else {
        text(line, 14., 20., 430., theme.text_secondary)
    };
    div()
        .relative()
        .w_full()
        .child(
            canvas(
                |bounds, _, _| bounds,
                move |bounds: Bounds<_>, _, window, cx| {
                    let above = if first { px(0.) } else { px(20.) };
                    let below = if last { px(0.) } else { px(20.) };
                    let painted = Bounds::new(
                        point(bounds.origin.x, bounds.origin.y - above),
                        size(bounds.size.width, bounds.size.height + above + below),
                    );
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        shapes::paint(
                            &shapes::Decoration {
                                radius: Corners {
                                    top_left: px(if first { 20. } else { 0. }),
                                    top_right: px(if first { 20. } else { 0. }),
                                    bottom_left: px(if last { 20. } else { 0. }),
                                    bottom_right: px(if last { 20. } else { 0. }),
                                },
                                corner: shapes::Corner::Squircle,
                                fill: Some(fill),
                                border: Some((px(1.), border)),
                                ..Default::default()
                            },
                            None,
                            painted,
                            window,
                            cx,
                        );
                    });
                },
            )
            .absolute()
            .inset_0()
            .size_full(),
        )
        .child(
            div()
                .relative()
                .px(px(if monospace { 13. } else { 17. }))
                .when(first, |d| d.pt(px(if monospace { 13. } else { 17. })))
                .when(last, |d| d.pb(px(if monospace { 13. } else { 17. })))
                .min_h(px(if monospace { 19.5 } else { 20. }))
                .child(content),
        )
}
