use super::*;
use crate::{focus, screens::settings::button};
use gpui::{AnyElement, ClipboardItem};

impl Page {
    pub(super) fn render_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let heading = self
            .detail
            .as_ref()
            .map(|p| p.title())
            .unwrap_or(self.tab.label())
            .to_owned();
        let query = self.query.read(cx).field_text().trim().to_lowercase();
        let mut toolbar = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .pb(px(24.))
            .child(text(heading, 28., 36., 600., t.text));
        let mut actions = div().flex().items_center().gap(px(8.));
        if self.detail.is_some() {
            actions = actions.child(self.action_button(
                "back",
                "Back",
                Action::Navigate("/plugins".into()),
                true,
                window,
                cx,
            ));
        } else {
            actions = actions.child(self.action_button(
                "refresh",
                "Refresh",
                Action::Refresh,
                true,
                window,
                cx,
            ));
            if self.tab == Tab::Skills {
                let available = self
                    .data
                    .skills
                    .value()
                    .into_iter()
                    .flatten()
                    .any(|skill| skill["name"] == "skill-creator" && skill["enabled"] == true);
                actions = actions.child(self.action_button(
                    "new-skill",
                    "New skill",
                    Action::NewSkill,
                    available,
                    window,
                    cx,
                ));
            }
            actions = actions.child(self.action_button(
                "manage",
                "Manage",
                Action::Manage,
                true,
                window,
                cx,
            ));
        }
        toolbar = toolbar.child(actions);
        let mut body = div().id("plugins-body").flex().flex_col().gap(px(24.));
        if self.detail.is_none() {
            let mut tabs = div()
                .flex()
                .gap(px(24.))
                .border_b_1()
                .border_color(t.border);
            for tab in Tab::ALL {
                let id = format!("browse-{}", tab.label());
                let handle = focus::handle(id.clone(), 0, window, cx);
                let route = if tab == Tab::Skills {
                    "/skills"
                } else {
                    "/plugins"
                };
                tabs = tabs.child(
                    focus::ring(
                        div()
                            .id(id)
                            .role(gpui::Role::Tab)
                            .cursor_pointer()
                            .pb(px(12.))
                            .when(self.tab == tab, |d| d.border_b_2().border_color(t.text))
                            .child(text(
                                tab.label(),
                                14.,
                                20.,
                                500.,
                                if self.tab == tab {
                                    t.text
                                } else {
                                    t.text_secondary
                                },
                            )),
                        &handle,
                        px(0.),
                        px(2.),
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.act(Action::Navigate(route.into()), cx)
                    }))
                    .on_key_down(cx.listener(
                        move |this, event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                this.act(Action::Navigate(route.into()), cx);
                                cx.stop_propagation();
                            }
                        },
                    )),
                );
            }
            body = body
                .child(tabs)
                .child(div().max_w(px(420.)).child(self.query.clone()));
        }
        if self.modal.is_none()
            && let Some(error) = &self.error
        {
            body = body.child(text(error.clone(), 14., 20., 400., t.text));
        }
        body = body.child(if let Some(plugin) = &self.detail {
            self.detail_view(plugin, window, cx)
        } else {
            self.list_view(&query, window, cx)
        });
        let mut page = div()
            .id("plugins-page")
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .p(px(24.))
            .bg(t.surface)
            .child(toolbar)
            .child(
                div()
                    .id("plugins-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            );
        if let Some(modal) = &self.modal {
            page = page.child(self.modal_view(modal, window, cx));
        }
        page.into_any_element()
    }

    fn action_button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: Action,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let id = id.into();
        let label = label.into();
        let enabled = enabled && !self.pending;
        let handle = focus::handle(id.clone(), 0, window, cx);
        let key_action = action.clone();
        focus::ring(
            button(label.clone(), t.text_alpha_5, None, &t)
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .when(!enabled, |d| d.opacity(0.5)),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if enabled {
                this.act(action.clone(), cx);
            }
            cx.stop_propagation();
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if enabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.act(key_action.clone(), cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }

    fn modal_button(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        modal: Modal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let id = id.into();
        let handle = focus::handle(id.clone(), 0, window, cx);
        let key_modal = modal.clone();
        focus::ring(
            button(label, t.text_alpha_5, None, &t)
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            if !this.pending {
                this.show_modal(modal.clone(), window, cx);
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if !this.pending && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.show_modal(key_modal.clone(), window, cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }

    fn status(&self, state: &str, description: Option<&str>, cx: &App) -> AnyElement {
        let t = cx.theme();
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .py(px(64.))
            .child(text(state.to_owned(), 16., 24., 500., t.text_secondary))
            .children(
                description.map(|value| text(value.to_owned(), 14., 20., 400., t.text_secondary)),
            )
            .into_any_element()
    }

    fn list_view(&self, query: &str, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        match self.tab {
            Tab::Plugins => {
                if !self.available {
                    return self.status("Plugins are not available for this host", None, cx);
                }
                let markets = match &self.data.plugins {
                    Resource::Loading => return self.status("Loading plugins…", None, cx),
                    Resource::Failed(error) => {
                        return self.status("Failed to load plugins", Some(error), cx);
                    }
                    Resource::Ready(value) => value,
                };
                let plugins: Vec<_> = markets
                    .iter()
                    .flat_map(|m| &m.plugins)
                    .filter(|p| p.visible() && p.matches(query))
                    .collect();
                if plugins.is_empty() {
                    return self.status("No plugins found", None, cx);
                }
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(px(16.))
                    .children(
                        plugins
                            .into_iter()
                            .map(|plugin| self.plugin_card(plugin, window, cx)),
                    )
                    .into_any_element()
            }
            Tab::Skills => {
                let skills = match &self.data.skills {
                    Resource::Loading => return self.status("Loading skills…", None, cx),
                    Resource::Failed(error) => {
                        return self.status("Failed to load skills", Some(error), cx);
                    }
                    Resource::Ready(value) => value,
                };
                let filtered: Vec<_> = skills
                    .iter()
                    .filter(|skill| skill["pluginId"].is_null() && matches_value(skill, query))
                    .collect();
                if filtered.is_empty() {
                    return self.status("No skills found", None, cx);
                }
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(px(16.))
                    .children(filtered.into_iter().map(|skill| {
                        let path = string(skill, "path");
                        let enabled = skill["enabled"].as_bool().unwrap_or(true);
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .p(px(16.))
                            .rounded(px(12.))
                            .border_1()
                            .border_color(t.border)
                            .child(text(
                                skill["interface"]["displayName"]
                                    .as_str()
                                    .unwrap_or(string(skill, "name"))
                                    .to_owned(),
                                16.,
                                24.,
                                500.,
                                t.text,
                            ))
                            .child(text(
                                string(skill, "description").to_owned(),
                                14.,
                                20.,
                                400.,
                                t.text_secondary,
                            ))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .child(self.modal_button(
                                        format!("skill-{path}"),
                                        "View skill",
                                        Modal::Skill(skill.clone()),
                                        window,
                                        cx,
                                    ))
                                    .child(self.action_button(
                                        format!("skill-toggle-{path}"),
                                        if enabled { "Disable" } else { "Enable" },
                                        Action::EnableSkill {
                                            name: string(skill, "name").into(),
                                            path: path.into(),
                                            enabled: !enabled,
                                        },
                                        true,
                                        window,
                                        cx,
                                    ))
                                    .child(self.action_button(
                                        format!("skill-open-{path}"),
                                        "Open",
                                        Action::OpenSkill(path.into()),
                                        true,
                                        window,
                                        cx,
                                    )),
                            )
                    }))
                    .into_any_element()
            }
        }
    }

    fn plugin_card(
        &self,
        plugin: &Arc<Plugin>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let id = format!("plugin-{}", plugin.id());
        let handle = focus::handle(id.clone(), 0, window, cx);
        let action = if plugin.installed() {
            self.action_button(
                format!("open-{}", plugin.id()),
                "Installed",
                Action::Read(plugin.clone()),
                true,
                window,
                cx,
            )
        } else {
            self.modal_button(
                format!("install-{}", plugin.id()),
                "Install",
                Modal::Install(plugin.clone()),
                window,
                cx,
            )
        };
        let selected = plugin.clone();
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(plugin.title().to_owned())
                .min_h(px(56.))
                .flex()
                .items_center()
                .gap(px(12.))
                .p(px(8.))
                .rounded(px(20.))
                .hover(|d| d.bg(t.text_alpha_5))
                .child(
                    div()
                        .size(px(40.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(t.border)
                        .child(icon("plugins", 20., t.text_secondary)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(text(plugin.title().to_owned(), 14., 21., 500., t.text).truncate())
                        .child(
                            text(
                                plugin.description().to_owned(),
                                13.,
                                18.57,
                                430.,
                                t.text_secondary,
                            )
                            .truncate(),
                        ),
                )
                .child(action),
            &handle,
            px(20.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| this.act(Action::Read(selected.clone()), cx)))
        .into_any_element()
    }

    fn app_row(&self, app: &Value, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let id = string(app, "id");
        let app = self
            .data
            .apps
            .value()
            .into_iter()
            .flatten()
            .find(|entry| entry["id"] == app["id"])
            .unwrap_or(app);
        let connected = app["isAccessible"] == true;
        let enabled = app["isEnabled"].as_bool().unwrap_or(true);
        let mut actions = div().flex().gap(px(8.));
        if connected {
            actions = actions
                .child(self.action_button(
                    format!("app-toggle-{id}"),
                    if enabled { "Disable" } else { "Enable" },
                    Action::EnableApp {
                        id: id.into(),
                        enabled: !enabled,
                    },
                    true,
                    window,
                    cx,
                ))
                .child(self.modal_button(
                    format!("disconnect-{id}"),
                    "Disconnect",
                    Modal::Disconnect(id.into()),
                    window,
                    cx,
                ));
        } else {
            actions = actions.child(self.modal_button(
                format!("connect-{id}"),
                "Connect",
                Modal::Connect(app.clone()),
                window,
                cx,
            ));
        }
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .p(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(text(string(app, "name").to_owned(), 16., 24., 500., t.text))
                    .child(text(
                        string(app, "description").to_owned(),
                        14.,
                        20.,
                        400.,
                        t.text_secondary,
                    )),
            )
            .child(actions)
            .into_any_element()
    }

    fn detail_view(
        &self,
        plugin: &Arc<Plugin>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let detail = match &self.detail_data {
            Resource::Loading => return self.status("Loading plugin details…", None, cx),
            Resource::Failed(error) => {
                return self.status("Couldn't load plugin details", Some(error), cx);
            }
            Resource::Ready(detail) => detail,
        };
        let mut view = div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .max_w(px(800.))
            .child(text(
                detail["description"]
                    .as_str()
                    .unwrap_or(plugin.description())
                    .to_owned(),
                16.,
                24.,
                400.,
                t.text_secondary,
            ));
        let mut actions = div().flex().gap(px(8.));
        if plugin.installed() {
            actions = actions
                .child(self.action_button(
                    "try-plugin",
                    "Try now",
                    Action::TryPlugin(plugin.clone()),
                    self.data.can_use(plugin, Some(detail)),
                    window,
                    cx,
                ))
                .child(self.action_button(
                    "enable-plugin",
                    if plugin.enabled() {
                        "Disable"
                    } else {
                        "Enable"
                    },
                    Action::EnablePlugin(plugin.clone(), !plugin.enabled()),
                    plugin.disabled_reason().is_none(),
                    window,
                    cx,
                ));
            if plugin.can_uninstall() {
                actions = actions.child(self.modal_button(
                    "uninstall-plugin",
                    "Uninstall",
                    Modal::Uninstall(plugin.clone()),
                    window,
                    cx,
                ));
            }
        } else {
            actions = actions.child(self.modal_button(
                "install-plugin",
                "Install",
                Modal::Install(plugin.clone()),
                window,
                cx,
            ));
        }
        if self.data.personal_path(plugin).is_some() {
            actions = actions
                .child(self.action_button(
                    "edit-plugin",
                    "Edit",
                    Action::EditPlugin(plugin.clone()),
                    true,
                    window,
                    cx,
                ))
                .child(self.action_button(
                    "share-plugin",
                    "Share",
                    Action::Share(plugin.clone()),
                    true,
                    window,
                    cx,
                ));
        } else if let Some(url) = detail["shareUrl"].as_str() {
            let url = url.to_owned();
            actions = actions.child(
                button("Copy link", t.text_alpha_5, None, &t)
                    .id("copy-plugin-link")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()))
                    }),
            );
        }
        view = view.child(actions);
        if let Some(reason) = plugin.disabled_reason() {
            view = view.child(text(reason, 14., 20., 500., t.text_secondary));
        }
        for (field, label) in [
            ("apps", "Apps"),
            ("skills", "Skills"),
            ("mcpServers", "MCP servers"),
        ] {
            if array(detail, field).is_empty() {
                continue;
            }
            let mut rows = div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(text(label, 18., 26., 500., t.text));
            for value in array(detail, field) {
                rows = rows.child(if field == "apps" {
                    self.app_row(value, window, cx)
                } else {
                    let title = value.as_str().unwrap_or_else(|| string(value, "name"));
                    div()
                        .child(text(title.to_owned(), 14., 20., 400., t.text_secondary))
                        .into_any_element()
                });
            }
            view = view.child(rows);
        }
        let caps = array(&plugin.summary["interface"], "capabilities");
        if !caps.is_empty() {
            view = view
                .child(text("Capabilities", 18., 26., 500., t.text))
                .children(
                    caps.iter()
                        .filter_map(Value::as_str)
                        .map(|cap| text(cap.to_owned(), 14., 20., 400., t.text_secondary)),
                );
        }
        view.into_any_element()
    }

    fn modal_view(&self, modal: &Modal, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let (title, content, confirm) = self.modal_content(modal, window, cx);
        let mut buttons = div().flex().justify_end().gap(px(8.)).child(
            button("Cancel", t.text_alpha_5, None, &t)
                .id("plugins-modal-cancel")
                .on_click(cx.listener(|this, _, window, cx| this.close_modal(window, cx))),
        );
        if let Some((label, action, enabled)) = confirm {
            buttons = buttons.child(self.action_button(
                "plugins-modal-confirm",
                label,
                action,
                enabled,
                window,
                cx,
            ));
        }
        let panel = div()
            .id("plugins-modal")
            .role(gpui::Role::Dialog)
            .aria_label(title.clone())
            .track_focus(&self.modal_focus)
            .w(px(512.))
            .max_h(px(700.))
            .flex()
            .flex_col()
            .gap(px(20.))
            .p(px(24.))
            .rounded(px(24.))
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .child(text(title, 20., 28., 600., t.text))
            .child(content)
            .children(
                self.error
                    .as_ref()
                    .map(|e| text(e.clone(), 14., 20., 400., t.text)),
            )
            .child(buttons)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close_modal(window, cx);
                    cx.stop_propagation();
                }
            }));
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.5))
            .child(panel)
            .into_any_element()
    }

    fn modal_content(
        &self,
        modal: &Modal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (String, AnyElement, Option<(&'static str, Action, bool)>) {
        let t = cx.theme().clone();
        match modal {
            Modal::Install(plugin) => {
                let mut content = div().flex().flex_col().gap(px(12.))
                    .child(text(plugin.description().to_owned(), 14., 20., 400., t.text_secondary));
                for cap in array(&plugin.summary["interface"], "capabilities").iter().filter_map(Value::as_str) {
                    content = content.child(text(cap.to_owned(), 14., 20., 400., t.text_secondary));
                }
                (format!("Install {}", plugin.title()), content.into_any_element(), Some(("Install", Action::Install(plugin.clone()), plugin.disabled_reason().is_none())))
            }
            Modal::Connections { plugin, apps } => (
                format!("Connect your apps to {}", plugin.title()),
                div().flex().flex_col().gap(px(16.))
                    .child(text("Get more relevant help from the apps you already use. You can always do this later.", 14., 20., 400., t.text_secondary))
                    .children(apps.iter().map(|app| self.app_row(app, window, cx))).into_any_element(),
                Some(("Try now", Action::TryPlugin(plugin.clone()), self.data.can_use(plugin, self.detail_data.value()))),
            ),
            Modal::Uninstall(plugin) => (format!("Uninstall {}?", plugin.title()),
                text("You can install this plugin again later.", 14., 20., 400., t.text_secondary).into_any_element(),
                Some(("Uninstall", Action::Uninstall(plugin.clone()), plugin.can_uninstall()))),
            Modal::Share(plugin) => {
                let mut content = div().flex().flex_col().gap(px(12.));
                if let Some(url) = &self.share_url {
                    content = content.child(text(url.clone(), 14., 20., 400., t.text_secondary));
                    let url = url.clone();
                    content = content.child(button("Copy link", t.text_alpha_5, None, &t).id("copy-share-link")
                        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(url.clone()))));
                } else {
                    content = content.child(self.action_button("create-share-link", "Copy link",
                        Action::CreateShare(plugin.clone()), true, window, cx));
                }
                let update = self.share_url.as_ref().map(|_| ("Update shared version",
                    Action::UpdateSharedVersion(plugin.clone()), true));
                (format!("Share {}", plugin.title()), content.into_any_element(), update)
            }
            Modal::Skill(skill) => (string(skill, "name").to_owned(),
                div().flex().flex_col().gap(px(12.))
                    .child(text(string(skill, "description").to_owned(), 14., 20., 400., t.text_secondary).whitespace_normal())
                    .child(div().h(px(360.)).children(self.skill_file.clone())).into_any_element(),
                Some(("Open", Action::OpenSkill(string(skill, "path").to_owned()), true))),
            Modal::Connect(app) => (format!("Connect {}", string(app, "name")),
                text(string(app, "description").to_owned(), 14., 20., 400., t.text_secondary).into_any_element(),
                Some(("Connect", Action::ConnectApp(app.clone()), true))),
            Modal::Connecting(app) => (format!("Connect {}", string(app, "name")),
                text("Finish connecting in your browser", 14., 20., 400., t.text_secondary).into_any_element(),
                Some(("Check connection", Action::Refresh, true))),
            Modal::Disconnect(id) => ("Disconnect app?".into(),
                text("This app will no longer be available to your conversations.", 14., 20., 400., t.text_secondary).into_any_element(),
                Some(("Disconnect", Action::DisconnectApp(id.clone()), true))),
        }
    }
}

fn matches_value(value: &Value, query: &str) -> bool {
    query.is_empty()
        || ["name", "description"]
            .iter()
            .any(|field| string(value, field).to_lowercase().contains(query))
}
