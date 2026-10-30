//! The Customize sidebar of the Plugins and Skills pages: the title, search, the two
//! directories and the original's `Installed` section (`InstalledPlugins`).

use super::*;
use crate::focus;
use gpui::{Div, Stateful};
use theme::Theme;

pub struct Nav {
    page: Entity<Page>,
}

impl Nav {
    pub fn new(page: Entity<Page>) -> Self {
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

    /// `sidebar.customize.installed`: installed plugins matching the search, each opening its
    /// detail; `No installed plugins` / `No matching plugins` when the list is empty.
    fn installed(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let page = self.page.read(cx);
        let query = page.query.read(cx).field_text().trim().to_lowercase();
        let selected = page.detail.as_ref().map(|plugin| plugin.id().to_owned());
        let mut section = div().flex().flex_col().gap(px(1.)).pt(px(16.)).child(
            div()
                .h(px(30.))
                .px(px(8.))
                .flex()
                .items_center()
                .child(text("Installed", 13., 18., 500., t.text_tertiary)),
        );
        let plugins = match &page.data.plugins {
            Resource::Loading => return section,
            Resource::Failed(_) => {
                let failed = "Failed to load plugins";
                return section.child(text(failed, 14., 20., 400., t.text_secondary).px(px(8.)));
            }
            Resource::Ready(markets) => installed_plugins(markets, &query)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>(),
        };
        if plugins.is_empty() {
            let empty = if query.is_empty() {
                "No installed plugins"
            } else {
                "No matching plugins"
            };
            return section.child(text(empty, 14., 20., 400., t.text_secondary).px(px(8.)));
        }
        for plugin in plugins {
            let active = selected.as_deref() == Some(plugin.id());
            let open = (self.page.clone(), plugin.clone());
            let key = open.clone();
            section = section.child(
                self.row(
                    format!("customize-installed-{}", plugin.id()).into(),
                    plugin.title().to_owned().into(),
                    "plugins",
                    active,
                    window,
                    cx,
                )
                .on_click(move |_, _, cx| {
                    let (page, plugin) = open.clone();
                    page.update(cx, |page, cx| page.act(Action::Read(plugin), cx))
                })
                .on_key_down(move |event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        let (page, plugin) = key.clone();
                        page.update(cx, |page, cx| page.act(Action::Read(plugin), cx));
                        cx.stop_propagation();
                    }
                }),
            );
        }
        section
    }
}

/// Installed, visible plugins matching `query`, once each: the same plugin can be listed by
/// several marketplaces, which the original collapses (`dedupeSearchResults`).
fn installed_plugins<'a>(markets: &'a [Marketplace], query: &str) -> Vec<&'a Arc<Plugin>> {
    let mut seen = std::collections::HashSet::new();
    markets
        .iter()
        .flat_map(|market| &market.plugins)
        .filter(|plugin| plugin.installed() && plugin.visible() && plugin.matches(query))
        .filter(|plugin| seen.insert(plugin.id()))
        .collect()
}

impl Render for Nav {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
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
                    .pb(px(8.))
                    .px(px(8.))
                    .gap(px(1.))
                    .child(self.directory(Tab::Plugins, "sidebar-cmd-plugins", window, cx))
                    .child(self.directory(Tab::Skills, "customize-skills", window, cx))
                    .child(self.installed(window, cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn installed_lists_each_matching_installed_plugin_once() {
        let market = |name: &str| Marketplace {
            name: name.into(),
            path: None,
            title: name.into(),
            plugins: [
                json!({"id":"github@a","name":"github","installed":true}),
                json!({"id":"slack@a","name":"slack","installed":false}),
                json!({"id":"codex-app-tools@openai-bundled","name":"tools","installed":true}),
            ]
            .into_iter()
            .map(|summary| {
                Arc::new(Plugin {
                    summary,
                    marketplace_name: name.into(),
                    marketplace_path: None,
                    remote_marketplace_name: None,
                })
            })
            .collect(),
        };
        let markets = [market("one"), market("two")];
        let ids = |query| -> Vec<_> {
            installed_plugins(&markets, query)
                .iter()
                .map(|p| p.id().to_owned())
                .collect()
        };
        assert_eq!(ids(""), ["github@a"]);
        assert_eq!(ids("git"), ["github@a"]);
        assert!(ids("slack").is_empty());
    }
}
