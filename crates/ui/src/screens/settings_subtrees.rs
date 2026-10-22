use crate::{MONO, focus, icon::icon, text};
use gpui::{
    Action, App, AppContext as _, Bounds, ContentMask, Context, Corners, Div, Entity, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, ListAlignment, ListState, ParentElement, Pixels,
    Render, SharedString, StatefulInteractiveElement, Styled, TextRun, Window, canvas, div, font,
    list, point, prelude::FluentBuilder as _, px, size,
};
use std::{cell::RefCell, rc::Rc};
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
                })
            })
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                this.text = Some(match result {
                    Ok(lines) => lines.map(|lines| {
                        Rc::new(Notices {
                            lines,
                            layout: RefCell::new(None),
                        })
                    }),
                    Err(error) => {
                        eprintln!("third-party-notices: {error}");
                        None
                    }
                });
                let count = this
                    .text
                    .as_ref()
                    .and_then(Option::as_ref)
                    .map_or(1, |notices| notices.lines.len());
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
    text: Option<Option<Rc<Notices>>>,
    list: ListState,
}

struct Notices {
    lines: Vec<SharedString>,
    layout: RefCell<Option<(Pixels, Vec<usize>)>>,
}

impl Notices {
    fn card_bounds(&self, index: usize, row: Bounds<Pixels>, window: &Window) -> Bounds<Pixels> {
        let width = row.size.width - px(26.);
        let mut layout = self.layout.borrow_mut();
        if layout.as_ref().is_none_or(|(cached, _)| *cached != width) {
            let mut offsets = Vec::with_capacity(self.lines.len() + 1);
            offsets.push(0);
            let mut font = font(MONO);
            font.weight = FontWeight(430.);
            for line in &self.lines {
                let run = TextRun {
                    len: line.len(),
                    font: font.clone(),
                    color: gpui::white(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let wrapped = window
                    .text_system()
                    .shape_text(line.clone(), px(12.), &[run], Some(width), None)
                    .expect("license text must shape with the loaded monospace font");
                let count: usize = wrapped
                    .iter()
                    .map(|line| line.wrap_boundaries.len() + 1)
                    .sum();
                offsets.push(offsets.last().copied().unwrap() + count.max(1));
            }
            *layout = Some((width, offsets));
        }
        let (_, offsets) = layout.as_ref().unwrap();
        let above = px(19.5) * offsets[index] + px(if index == 0 { 0. } else { 13. });
        Bounds::new(
            point(row.origin.x, row.origin.y - above),
            size(
                row.size.width,
                px(19.5) * offsets.last().copied().unwrap() + px(26.),
            ),
        )
    }
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
                .gap(px(4.))
                .child(icon("arrow-left", 16., t.text_tertiary).runtime())
                .child(text("Back", 13., 18., 430., t.text_tertiary)),
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
            .map_or(1, |notices| notices.lines.len());
        let rows = list(self.list.clone(), move |index, _, _| {
            if index == 0 {
                return column(
                    div()
                        .pt(px(28.))
                        .pb(px(33.))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(text("Open source licenses", 28., 33.6, 500., t.text))
                        .child(text(
                            "Third-party notices for dependencies included in this app",
                            14.,
                            24.,
                            430.,
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
                Some(None) => notice_row("No third-party notices were found.".into(), None, &t),
                Some(Some(notices)) => notice_row(
                    notices.lines[index - 1].clone(),
                    Some((notices.clone(), index - 1)),
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
            .child(
                div()
                    .id("licenses-page")
                    .min_h_0()
                    .flex_1()
                    .layer()
                    .child(rows),
            )
    }
}

fn column(content: Div) -> Div {
    div()
        .w_full()
        .flex()
        .justify_center()
        .child(div().w_full().max_w(px(768.)).px(px(20.)).child(content))
}

fn notice_row(line: SharedString, notices: Option<(Rc<Notices>, usize)>, theme: &Theme) -> Div {
    let monospace = notices.is_some();
    let first = notices.as_ref().is_none_or(|(_, index)| *index == 0);
    let last = notices
        .as_ref()
        .is_none_or(|(notices, index)| *index + 1 == notices.lines.len());
    let fill = theme.background_panel;
    let border = theme.border;
    let content = if monospace {
        text(line, 12., 19.5, 430., theme.text)
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
                    let painted = notices.as_ref().map_or(bounds, |(notices, index)| {
                        notices.card_bounds(*index, bounds, window)
                    });
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        shapes::paint(
                            &shapes::Decoration {
                                radius: Corners::all(px(20.)),
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use gpui::HeadlessAppContext;
    use std::sync::Arc;

    #[test]
    fn card_geometry_tracks_wrapping_empty_lines_and_scrolled_rows() {
        let mut cx =
            HeadlessAppContext::with_asset_source(textsys::text_system(), Arc::new(theme::Assets));
        cx.update(|cx| {
            cx.set_global(Theme::dark());
            theme::load_fonts(cx);
        });
        let handle = cx
            .open_window(size(px(500.), px(400.)), |_, cx| {
                licenses(async { Ok(None) }, cx)
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, _| {
            let notices = Notices {
                lines: vec!["".into(), "alpha beta".into(), "Z".into()],
                layout: RefCell::new(None),
            };
            let row = Bounds::new(point(px(10.), px(20.)), size(px(76.), px(32.5)));
            let narrow = notices.card_bounds(0, row, window);
            assert_eq!(narrow.size.height, px(104.));
            let last_row = Bounds::new(
                point(px(10.), px(20. + 13. + 3. * 19.5)),
                size(px(76.), px(32.5)),
            );
            assert_eq!(notices.card_bounds(2, last_row, window), narrow);
            let wide = Bounds::new(row.origin, size(px(500.), row.size.height));
            assert_eq!(notices.card_bounds(0, wide, window).size.height, px(84.5));
            assert_eq!(notices.card_bounds(0, row, window), narrow);
        })
        .unwrap();
    }
}
