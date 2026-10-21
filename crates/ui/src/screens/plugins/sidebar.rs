use super::{Action, Page, Tab, string};
use crate::{composer::Composer, focus, icon::icon, screens::computer_use::Load, text};
use gpui::{
    AnyElement, AppContext as _, Context, Corners, Entity, Focusable as _, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, div, img, prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use std::cmp::Reverse;
use theme::ActiveTheme as _;

pub struct Sidebar {
    page: Entity<Page>,
    search: Entity<Composer>,
    search_open: bool,
}

impl Sidebar {
    pub fn new(page: Entity<Page>, cx: &mut Context<Self>) -> Self {
        cx.observe(&page, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<crate::screens::settings::integrations::State>(|_, cx| cx.notify())
            .detach();
        let search = cx.new(|cx| Composer::inline_field("Search installed plugins", "", cx));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        Self {
            page,
            search,
            search_open: false,
        }
    }

    fn dispatch(&self, action: Action, cx: &mut Context<Self>) {
        self.page.update(cx, |page, cx| page.act(action, cx));
    }

    fn row(
        &self,
        id: SharedString,
        label: String,
        artwork: AnyElement,
        selected: bool,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let handle = focus::handle(id.clone(), 0, window, cx);
        let key_action = action.clone();
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label.clone())
                .relative()
                .h(px(30.))
                .flex_none()
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .when(selected, |row| {
                    row.child(shapes::decoration(shapes::Decoration {
                        radius: Corners::all(px(12.5)),
                        corner: shapes::Corner::Squircle,
                        fill: Some(t.ghost_hover),
                        ..Default::default()
                    }))
                })
                .when(!selected, |row| {
                    row.hover(|row| row.bg(t.ghost_hover).rounded(px(12.5)))
                })
                .child(div().size(px(16.)).flex_none().child(artwork))
                .child(
                    text(
                        label,
                        14.,
                        21.,
                        430.,
                        if selected { t.text } else { t.text_default_nav },
                    )
                    .truncate(),
                ),
            &handle,
            px(12.5),
            px(0.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| this.dispatch(action.clone(), cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.dispatch(key_action.clone(), cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let (tab, detail) = {
            let page = self.page.read(cx);
            (
                page.tab,
                page.detail.as_ref().map(|plugin| plugin.id().to_owned()),
            )
        };
        let state = cx.global::<crate::screens::settings::integrations::State>();
        let installed = state.plugins.clone();
        let hidden = state.hidden_plugins.clone();
        let mut header = div().h(px(52.)).flex_none().px(px(8.)).child(
            div()
                .h(px(52.))
                .ml(px(8.))
                .pr(px(4.))
                .flex()
                .items_center()
                .child(
                    text("Customize", 18., 24., 600., t.text_default_nav)
                        .font_family("OpenAI Sans")
                        .flex_1(),
                )
                .child(
                    div()
                        .id("installed-search-toggle")
                        .role(gpui::Role::Button)
                        .aria_label("Search installed plugins")
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .child(icon("search", 16., t.text.opacity(0.498)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.search_open = !this.search_open;
                            if this.search_open {
                                this.search.focus_handle(cx).focus(window, cx);
                            }
                            cx.notify();
                        })),
                ),
        );
        if self.search_open {
            header = header
                .h_auto()
                .child(div().px(px(8.)).py(px(5.)).child(self.search.clone()));
        }
        let mut navigation = div().flex().flex_col().gap(px(1.));
        for target in Tab::ALL {
            let name = if target == Tab::Skills {
                "sidebar-skills"
            } else {
                "sidebar-plugins"
            };
            let route = if target == Tab::Skills {
                "/skills"
            } else {
                "/plugins"
            };
            let active = detail.is_none() && tab == target;
            navigation = navigation.child(
                self.row(
                    format!("customize-{}", target.label()).into(),
                    target.label().into(),
                    icon(
                        name,
                        16.,
                        if active {
                            t.text.opacity(0.906)
                        } else {
                            t.text_default_nav
                        },
                    )
                    .into_any_element(),
                    active,
                    Action::Navigate(route.into()),
                    window,
                    cx,
                ),
            );
        }
        let query = self.search.read(cx).field_text().trim().to_lowercase();
        let mut installed_rows = div().flex().flex_col().gap(px(1.));
        match installed {
            Load::Loading => {
                installed_rows = installed_rows
                    .child(text("Loading plugins…", 13., 20., 430., t.text_secondary).p(px(8.)));
            }
            Load::Failed(error) => {
                installed_rows = installed_rows
                    .child(
                        text("Failed to load plugins", 13., 20., 430., t.text_secondary).p(px(8.)),
                    )
                    .child(
                        text(error, 12., 18., 430., t.text_secondary)
                            .whitespace_normal()
                            .px(px(8.)),
                    );
            }
            Load::Ready(value) => {
                let mut plugins: Vec<&Value> = super::array(&value, "marketplaces")
                    .iter()
                    .flat_map(|market| super::array(market, "plugins"))
                    .filter(|plugin| {
                        crate::screens::settings::integrations::visible_plugin(plugin, &hidden)
                    })
                    .filter(|plugin| {
                        query.is_empty()
                            || crate::screens::settings::integrations::plugin_name(plugin)
                                .to_lowercase()
                                .contains(&query)
                    })
                    .collect();
                plugins.sort_by_key(|plugin| {
                    Reverse(plugin["installedAt"].as_i64().unwrap_or(i64::MIN))
                });
                if plugins.is_empty() {
                    installed_rows = installed_rows.child(
                        text(
                            if query.is_empty() {
                                "No installed plugins"
                            } else {
                                "No matching plugins"
                            },
                            13.,
                            20.,
                            430.,
                            t.text_secondary,
                        )
                        .p(px(8.)),
                    );
                }
                for plugin in plugins {
                    let id = string(plugin, "id");
                    let known_icon = match string(plugin, "name") {
                        "google-calendar" => Some("app-google-calendar"),
                        "gmail" => Some("app-gmail"),
                        "github" => Some("app-github"),
                        _ => None,
                    };
                    let artwork = known_icon
                        .map(|name| icon(name, 16., t.text_default_nav).into_any_element())
                        .or_else(|| {
                            plugin["iconDataUrl"]
                                .as_str()
                                .and_then(|url| {
                                    crate::screens::computer_use::data_image(
                                        url,
                                        format!("customize-icon-{id}").into(),
                                        window,
                                        cx,
                                    )
                                })
                                .map(|image| {
                                    img(image).size(px(16.)).rounded(px(2.5)).into_any_element()
                                })
                        })
                        .unwrap_or_else(|| {
                            icon("sidebar-plugins", 16., t.text_default_nav).into_any_element()
                        });
                    installed_rows = installed_rows.child(self.row(
                        format!("customize-installed-{id}").into(),
                        crate::screens::settings::integrations::plugin_name(plugin).to_owned(),
                        artwork,
                        detail.as_deref() == Some(id),
                        Action::Navigate(format!("/plugins/{id}")),
                        window,
                        cx,
                    ));
                }
            }
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if this.search_open && event.keystroke.key == "escape" {
                    this.search_open = false;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(header)
            .child(
                div()
                    .id("customize-navigation")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(8.))
                    .pb(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(navigation)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .min_h(px(30.))
                                    .px(px(8.))
                                    .flex()
                                    .items_center()
                                    .child(text(
                                        "Installed",
                                        14.,
                                        21.,
                                        500.,
                                        t.text.opacity(0.498),
                                    )),
                            )
                            .child(installed_rows),
                    ),
            )
    }
}
