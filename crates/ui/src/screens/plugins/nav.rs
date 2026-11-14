//! The Customize sidebar of the Plugins and Skills pages: the title, search, the two
//! directories and the `Installed` plugins from `plugin/list`.

use super::*;
use crate::focus;
use gpui::{AnyElement, Div, Stateful, StyledImage as _};
use theme::{Appearance, Theme};

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
        leading: AnyElement,
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
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .rounded(Theme::RADIUS_LG)
                .when(selected, |d| d.bg(t.list_hover))
                .when(!selected, |d| d.hover(|d| d.bg(t.list_hover)))
                .child(leading)
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
            icon(
                glyph,
                16.,
                if selected {
                    cx.theme().icon_primary
                } else {
                    cx.theme().text
                },
            )
            .into_any_element(),
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

    /// The original's `Installed` sidebar group: every installed, visible plugin; a row opens it.
    fn installed(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let installed: Vec<_> = self
            .page
            .read(cx)
            .data
            .plugins
            .value()
            .into_iter()
            .flatten()
            .flat_map(|market| &market.plugins)
            .filter(|plugin| plugin.installed() && plugin.visible())
            .cloned()
            .collect();
        installed
            .into_iter()
            .map(|plugin| {
                let page = self.page.clone();
                let key_page = page.clone();
                let selected = plugin.clone();
                let key_selected = plugin.clone();
                let t = cx.theme();
                let fallback_color = t.text;
                let glyph = move || icon("plugins", 16., fallback_color).into_any_element();
                let leading = logo(
                    &plugin.summary,
                    self.page
                        .read(cx)
                        .data
                        .apps
                        .value()
                        .map_or(&[], Vec::as_slice),
                    t.appearance == Appearance::Dark,
                )
                .map(|path| {
                    let source: gpui::ImageSource = if Path::new(path).is_absolute() {
                        PathBuf::from(path).into()
                    } else {
                        path.to_owned().into()
                    };
                    gpui::img(source)
                        .size(px(16.))
                        .flex_none()
                        .object_fit(gpui::ObjectFit::Contain)
                        .with_fallback(glyph)
                        .into_any_element()
                })
                .unwrap_or_else(glyph);
                self.row(
                    format!("customize-installed-{}", plugin.id()).into(),
                    plugin.title().to_owned().into(),
                    leading,
                    false,
                    window,
                    cx,
                )
                .on_click(move |_, _, cx| {
                    page.update(cx, |page, cx| page.act(Action::Read(selected.clone()), cx))
                })
                .on_key_down(move |event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        key_page.update(cx, |page, cx| {
                            page.act(Action::Read(key_selected.clone()), cx)
                        });
                        cx.stop_propagation();
                    }
                })
            })
            .collect()
    }
}

impl Render for Nav {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let search = focus::handle("customize-search", 0, window, cx);
        let page = self.page.clone();
        let installed = self.installed(window, cx);
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
                    .when(!installed.is_empty(), |nav| {
                        nav.child(
                            div()
                                .mt(px(16.))
                                .px(px(8.))
                                .h(px(30.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .child(text("Installed", 13., 18., 430., t.text_tertiary)),
                        )
                        .children(installed)
                    }),
            )
    }
}

fn pick<'a>(v: &'a Value, light: [&str; 2], dark_keys: [&str; 2], dark: bool) -> Option<&'a str> {
    let first = |keys: [&str; 2]| {
        keys.into_iter()
            .find_map(|key| v[key].as_str().filter(|s| !s.is_empty()))
    };
    let (light, dark_logo) = (first(light), first(dark_keys));
    if dark {
        dark_logo.or(light)
    } else {
        light.or(dark_logo)
    }
}

fn logo<'a>(plugin: &'a Value, apps: &'a [Value], dark: bool) -> Option<&'a str> {
    let interface = &plugin["interface"];
    if let Some(path) = pick(
        interface,
        ["logo", "logoUrl"],
        ["logoDark", "logoUrlDark"],
        dark,
    ) {
        return Some(path);
    }
    // shortcut: match unique app names until plugin/list exposes canonical app IDs.
    let name = interface["displayName"]
        .as_str()
        .or_else(|| plugin["name"].as_str())?;
    let mut matches = apps.iter().filter(|app| {
        app["name"]
            .as_str()
            .is_some_and(|app_name| app_name.eq_ignore_ascii_case(name))
    });
    let app = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    pick(
        app,
        ["iconUrl", "iconUrl"],
        ["iconUrlDark", "iconUrlDark"],
        dark,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selects_themed_plugin_logos_before_app_icons() {
        let plugin = json!({"name": "github", "interface": {
            "displayName": "GitHub", "logo": "/plugins/github/light.png",
            "logoDark": "/plugins/github/dark.png"
        }});
        let apps = [json!({"id": "github-app", "name": "GitHub",
            "iconUrl": "https://example.com/app.png"})];
        assert_eq!(
            logo(&plugin, &apps, false),
            Some("/plugins/github/light.png")
        );
        assert_eq!(logo(&plugin, &apps, true), Some("/plugins/github/dark.png"));
        let dark_only = json!({"interface": {"logoUrlDark": "https://example.com/dark.png"}});
        assert_eq!(
            logo(&dark_only, &[], false),
            Some("https://example.com/dark.png")
        );
    }

    #[test]
    fn uses_only_a_matching_app_and_rejects_ambiguous_names() {
        let plugin = json!({"name": "linear", "interface": {"displayName": "Linear", "logo": ""}});
        let apps = [
            json!({"id": "other", "name": "Other", "iconUrl": "https://example.com/other.png"}),
            json!({"id": "linear-app", "name": "Linear", "iconUrl": "https://example.com/light.png",
                "iconUrlDark": "https://example.com/dark.png"}),
        ];
        assert_eq!(
            logo(&plugin, &apps, true),
            Some("https://example.com/dark.png")
        );
        assert_eq!(
            logo(&plugin, &apps, false),
            Some("https://example.com/light.png")
        );
        assert_eq!(logo(&plugin, &apps[..1], false), None);
        assert_eq!(
            logo(&plugin, &[apps[1].clone(), apps[1].clone()], false),
            None
        );
        assert_eq!(logo(&json!({}), &apps, false), None);
    }
}
