//! The Customize sidebar of the Plugins and Skills pages: title, the two directories and the
//! connected apps.

use super::*;
use crate::{focus, sidebar_section};
use gpui::{Div, Stateful};
use theme::Theme;

pub struct Nav {
    page: Entity<Page>,
}

impl Nav {
    pub fn new(page: Entity<Page>, cx: &mut Context<Self>) -> Self {
        cx.observe(&page, |_, _, cx| cx.notify()).detach();
        Self { page }
    }

    /// A 30 px nav row: 16 px icon and 14 px label, `bg-primary-ghost-hover` when selected.
    fn row(
        &self,
        id: SharedString,
        label: SharedString,
        glyph: &'static str,
        selected: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme().clone();
        let handle = focus::handle(id.clone(), 0, window, cx);
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label.clone())
                .h(px(30.))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .rounded(Theme::RADIUS_LG)
                .when(selected, |d| d.bg(t.list_hover))
                .when(!selected, |d| d.hover(|d| d.bg(t.list_hover)))
                .child(icon(
                    glyph,
                    16.,
                    if selected { t.icon_primary } else { t.text },
                ))
                .child(text(label, 14., 21., 430., t.text).truncate()),
            &handle,
            Theme::RADIUS_LG,
            px(2.),
            cx,
        )
    }

    fn directory(
        &self,
        tab: Tab,
        glyph: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let selected = self.page.read(cx).tab == tab;
        let route = match tab {
            Tab::Plugins => "/plugins",
            Tab::Skills => "/skills",
        };
        let page = self.page.clone();
        let key_page = page.clone();
        self.row(
            format!("customize-{}", tab.label()).into(),
            tab.label().into(),
            glyph,
            selected,
            window,
            cx,
        )
        .on_click(move |_, _, cx| {
            page.update(cx, |page, cx| page.act(Action::Navigate(route.into()), cx))
        })
        .on_key_down(move |event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                key_page.update(cx, |page, cx| page.act(Action::Navigate(route.into()), cx));
                cx.stop_propagation();
            }
        })
    }
}

impl Render for Nav {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let apps: Vec<(SharedString, SharedString)> = self
            .page
            .read(cx)
            .data
            .apps
            .value()
            .into_iter()
            .flatten()
            .filter(|app| app["isAccessible"] == true && app["isEnabled"] != false)
            .map(|app| {
                (
                    string(app, "id").to_owned().into(),
                    string(app, "name").to_owned().into(),
                )
            })
            .collect();
        let rows: Vec<_> = apps
            .into_iter()
            .map(|(id, name)| {
                self.row(
                    format!("customize-app-{id}").into(),
                    name,
                    "plugins",
                    false,
                    window,
                    cx,
                )
            })
            .collect();
        let search = focus::handle("customize-search", 0, window, cx);
        let page = self.page.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .h(px(52.))
                    .mx(px(8.))
                    .pl(px(8.))
                    .pr(px(4.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(text("Customize", 18., 24., 600., t.text).font_family("OpenAI Sans"))
                    .child(
                        focus::ring(
                            div()
                                .id("customize-search")
                                .role(gpui::Role::Button)
                                .aria_label("Search")
                                .size(px(28.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(9.375))
                                .hover(|d| d.bg(t.ghost_hover))
                                .child(icon("search", 16., t.text_tertiary)),
                            &search,
                            px(9.375),
                            px(2.),
                            cx,
                        )
                        .on_click(move |_, window, cx| {
                            let query = page.read(cx).query.clone();
                            query.focus_handle(cx).focus(window, cx);
                        }),
                    ),
            )
            .child(
                div()
                    .id("customize-nav")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .pb(px(8.))
                    .child(
                        div()
                            .px(px(8.))
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(self.directory(Tab::Plugins, "sidebar-cmd-plugins", window, cx))
                            .child(self.directory(Tab::Skills, "customize-skills", window, cx)),
                    )
                    .when(!rows.is_empty(), |d| {
                        d.child(sidebar_section(
                            "Installed",
                            div().flex().flex_col().gap(px(1.)).children(rows),
                            cx,
                        ))
                    }),
            )
    }
}
