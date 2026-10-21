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
        if self.modal.is_none()
            && self.marketplace_form.is_none()
            && let Some(focus) = self.return_focus.take()
        {
            focus.focus(window, cx);
        }
        if self.share_on_load
            && let Some(plugin) = self.detail.clone()
        {
            self.share_on_load = false;
            self.act(Action::Share(plugin), cx);
        }
        let query = self.query.read(cx).field_text().trim().to_lowercase();
        let title = self
            .detail
            .as_ref()
            .map(|plugin| plugin.title())
            .unwrap_or(self.tab.label())
            .to_owned();
        let mut toolbar = div()
            .w_full()
            .max_w(px(960.))
            .mx_auto()
            .px(px(30.))
            .h(px(60.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .child(text(title, 24., 28.8, 500., t.text));
        toolbar = toolbar.child(if self.detail.is_some() {
            self.action_button(
                "back",
                "Back",
                Action::Navigate("/plugins".into()),
                true,
                window,
                cx,
            )
        } else {
            self.header_actions(window, cx)
        });
        let mut page = div()
            .id("plugins-page")
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if this.add_open && event.keystroke.key == "escape" {
                    this.add_open = false;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex_none()
                    .pt(px(16.))
                    .border_b_1()
                    .border_color(gpui::transparent_black())
                    .child(toolbar),
            );
        if self.detail.is_none() {
            page = page.child(
                div().h(px(24.)).flex_none().child(
                    div()
                        .w_full()
                        .max_w(px(960.))
                        .mx_auto()
                        .px(px(30.))
                        .relative()
                        .top(px(-6.))
                        .child(text(
                            if self.tab == Tab::Skills {
                                "Extend Codex with task-specific skills"
                            } else {
                                "Connect plugins to let Codex work across your tools"
                            },
                            14.,
                            24.,
                            430.,
                            t.text.opacity(0.65),
                        )),
                ),
            );
        }
        let mut content = div()
            .w_full()
            .max_w(px(960.))
            .mx_auto()
            .px(px(30.))
            .pb(px(if self.tab == Tab::Skills { 36. } else { 16. }));
        if self.detail.is_none() && self.tab == Tab::Plugins {
            content = content.child(self.directory_tabs(window, cx));
        } else if self.detail.is_none() {
            content = content.pt(px(8.));
        }
        if self.modal.is_none()
            && let Some(error) = &self.error
        {
            content = content.child(text(error.clone(), 14., 20., 400., t.text));
        }
        let body = if let Some(plugin) = &self.detail {
            self.detail_view(plugin, window, cx)
        } else {
            self.list_view(&query, window, cx)
        };
        content = content.child(
            div()
                .when(self.detail.is_none() && self.tab == Tab::Plugins, |body| {
                    body.pt(px(8.)).mx(px(-8.))
                })
                .child(body),
        );
        page = page.child(
            div()
                .id("plugins-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(content),
        );
        if self.add_open {
            page = page.child(self.add_menu(window, cx));
        }
        if let Some(form) = &self.marketplace_form {
            page = page.child(form.clone());
        } else if let Some(modal) = &self.modal {
            page = page.child(self.modal_view(modal, window, cx));
        }
        page.into_any_element()
    }

    fn header_actions(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let mut search = div()
            .w(px(240.))
            .h(px(32.))
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(13.))
            .child(shapes::decoration(shapes::Decoration {
                radius: gpui::Corners::all(px(9999.)),
                corner: shapes::Corner::Round,
                border: Some((px(1.), t.border_strong)),
                ..Default::default()
            }))
            .child(icon("search", 16., t.text.opacity(0.65)))
            .child(div().flex_1().min_w_0().child(self.query.clone()));
        if !self.query.read(cx).field_text().is_empty() {
            let handle = focus::handle("clear-plugin-search", 0, window, cx);
            search = search.child(
                focus::ring(
                    div()
                        .id("clear-plugin-search")
                        .role(gpui::Role::Button)
                        .aria_label("Clear search")
                        .cursor_pointer()
                        .size(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon("plugins-clear", 16., t.text.opacity(0.65))),
                    &handle,
                    px(8.),
                    px(2.),
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.query.update(cx, |query, cx| query.set_text("", cx))
                }))
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        this.query.update(cx, |query, cx| query.set_text("", cx));
                        cx.stop_propagation();
                    }
                })),
            );
        }
        let mut actions = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(self.icon_action(
                "refresh",
                "Refresh",
                "plugins-refresh",
                Action::Refresh,
                window,
                cx,
            ))
            .child(self.icon_action(
                "manage",
                "Manage",
                "settings-general",
                Action::Manage,
                window,
                cx,
            ));
        let is_skills = self.tab == Tab::Skills;
        let enabled = !is_skills
            || self.data.skills.value().is_some_and(|skills| {
                skills
                    .iter()
                    .any(|skill| skill["name"] == "skill-creator" && skill["enabled"] == true)
            });
        let label = "Add";
        let handle = focus::handle("plugins-add", 0, window, cx);
        let button = div()
            .id("plugins-add")
            .role(gpui::Role::Button)
            .aria_label(label)
            .relative()
            .h(px(32.))
            .px(px(9.))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(4.))
            .cursor_pointer()
            .when(!enabled || self.pending, |button| button.opacity(0.5))
            .child(shapes::decoration(shapes::Decoration {
                radius: gpui::Corners::all(px(9999.)),
                corner: shapes::Corner::Round,
                fill: Some(t.button_primary_solid),
                border: Some((px(1.), t.border)),
                ..Default::default()
            }))
            .child(text(
                label,
                13.,
                18.,
                430.,
                if t.appearance == theme::Appearance::Dark {
                    gpui::rgb(0x2d2d2d).into()
                } else {
                    t.text_inverse
                },
            ))
            .when(!is_skills, |button| {
                button.child(icon(
                    "chevron-down",
                    16.,
                    if t.appearance == theme::Appearance::Dark {
                        gpui::rgb(0x2d2d2d).into()
                    } else {
                        t.text_inverse
                    },
                ))
            });
        actions = actions.child(
            focus::ring(button, &handle, px(9999.), px(2.), cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !enabled || this.pending {
                        return;
                    }
                    if is_skills {
                        this.act(Action::NewSkill, cx);
                    } else {
                        this.add_open = !this.add_open;
                        cx.notify();
                    }
                }))
                .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                    if enabled
                        && !this.pending
                        && matches!(event.keystroke.key.as_str(), "enter" | "space")
                    {
                        if is_skills {
                            this.act(Action::NewSkill, cx);
                        } else {
                            this.add_open = !this.add_open;
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }
                })),
        );
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(search)
            .child(actions)
            .into_any_element()
    }

    fn icon_action(
        &self,
        id: &'static str,
        label: &'static str,
        name: &'static str,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let handle = focus::handle(id, 0, window, cx);
        let keyboard_action = action.clone();
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .size(px(32.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|button| button.bg(t.ghost_hover))
                .child(icon(name, 16., t.text_tertiary)),
            &handle,
            px(9999.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| this.act(action.clone(), cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.act(keyboard_action.clone(), cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }

    fn directory_tabs(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        div()
            .h(px(60.))
            .flex()
            .items_center()
            .gap(px(4.))
            .children(
                [Directory::Public, Directory::Personal]
                    .into_iter()
                    .map(|directory| {
                        let handle = focus::handle(directory.label(), 0, window, cx);
                        let selected = self.directory == directory;
                        focus::ring(
                            div()
                                .id(directory.label())
                                .role(gpui::Role::Tab)
                                .relative()
                                .h(px(36.))
                                .px(px(17.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .when(selected, |tab| {
                                    tab.child(shapes::decoration(shapes::Decoration {
                                        radius: gpui::Corners::all(px(9999.)),
                                        corner: shapes::Corner::Round,
                                        fill: Some(t.ghost_hover),
                                        ..Default::default()
                                    }))
                                })
                                .child(text(
                                    directory.label(),
                                    13.,
                                    18.5714,
                                    500.,
                                    if selected { t.text } else { t.text_secondary },
                                )),
                            &handle,
                            px(9999.),
                            px(2.),
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.act(Action::SelectDirectory(directory), cx)
                        }))
                        .on_key_down(cx.listener(
                            move |this, event: &KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.act(Action::SelectDirectory(directory), cx);
                                    cx.stop_propagation();
                                }
                            },
                        ))
                    }),
            )
            .into_any_element()
    }

    fn add_menu(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let mut menu = div()
            .absolute()
            .top(px(68.))
            .right(px(30.))
            .w(px(220.))
            .p(px(6.))
            .rounded(px(16.))
            .bg(t.menu_background)
            .border_1()
            .border_color(t.border)
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
        if cx
            .global::<crate::screens::settings::integrations::State>()
            .plugin_creator
            .is_some()
        {
            menu = menu.child(self.action_button(
                "create-plugin",
                "Create plugin",
                Action::CreatePlugin,
                true,
                window,
                cx,
            ));
        }
        let handle = focus::handle("add-marketplace", 0, window, cx);
        menu = menu.child(
            focus::ring(
                div()
                    .id("add-marketplace")
                    .role(gpui::Role::Button)
                    .aria_label("Add marketplace")
                    .h(px(36.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(10.))
                    .cursor_pointer()
                    .hover(|row| row.bg(t.menu_item_hover))
                    .child(text("Add marketplace", 13., 18.5714, 430., t.text)),
                &handle,
                px(10.),
                px(2.),
                cx,
            )
            .on_click(cx.listener(|this, _, window, cx| this.open_marketplace(window, cx)))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.open_marketplace(window, cx);
                    cx.stop_propagation();
                }
            })),
        );
        div()
            .absolute()
            .inset_0()
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.add_open = false;
                    cx.notify();
                }),
            )
            .child(menu)
            .into_any_element()
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

    pub(super) fn status(&self, state: &str, description: Option<&str>, cx: &App) -> AnyElement {
        let t = cx.theme();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(8.))
            .child(text(state.to_owned(), 14., 21., 430., t.text_secondary))
            .children(
                description.map(|value| text(value.to_owned(), 14., 20., 400., t.text_secondary)),
            )
            .into_any_element()
    }

    pub(super) fn failure(
        &self,
        title: &'static str,
        error: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let handle = focus::handle("retry-directory", 0, window, cx);
        let retry = focus::ring(
            div()
                .id("retry-directory")
                .role(gpui::Role::Button)
                .aria_label("Retry")
                .h(px(24.))
                .px(px(9.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(t.text_alpha_5)
                .child(text("Retry", 13., 18., 430., t.text)),
            &handle,
            px(9999.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(|this, _, _, cx| this.act(Action::Refresh, cx)))
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.act(Action::Refresh, cx);
                cx.stop_propagation();
            }
        }));
        div()
            .w_full()
            .px(px(12.))
            .py(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(576.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(8.))
                            .child(text(title, 16., 24., 500., t.text))
                            .child(text(error.to_owned(), 13., 18.5714, 430., t.text_secondary)),
                    )
                    .child(retry),
            )
            .into_any_element()
    }

    fn list_view(&self, query: &str, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.tab {
            Tab::Plugins => {
                if !self.available {
                    return self.status("Plugins are not available for this host", None, cx);
                }
                let state = cx.global::<crate::screens::settings::integrations::State>();
                if !state.gates_loaded {
                    return self.status("Loading plugins…", None, cx);
                }
                let hidden = state.hidden_plugins.clone();
                let markets = match &self.data.plugins {
                    Resource::Loading => return self.status("Loading plugins…", None, cx),
                    Resource::Failed(error) => {
                        return self.failure("Failed to load plugins", error, window, cx);
                    }
                    Resource::Ready(value) => value,
                };
                let plugins: Vec<_> = markets
                    .iter()
                    .flat_map(|m| &m.plugins)
                    .filter(|p| {
                        let personal =
                            self.data
                                .personal_marketplace
                                .as_deref()
                                .is_some_and(|path| {
                                    p.marketplace_path
                                        .as_deref()
                                        .is_some_and(|market| Path::new(market) == path)
                                });
                        (personal == (self.directory == Directory::Personal))
                            && self
                                .marketplace_path
                                .as_ref()
                                .is_none_or(|path| p.marketplace_path.as_ref() == Some(path))
                            && self.category.as_ref().is_none_or(|category| {
                                p.summary["category"]
                                    .as_str()
                                    .or_else(|| p.summary["interface"]["category"].as_str())
                                    .is_some_and(|value| value.eq_ignore_ascii_case(category))
                            })
                            && crate::screens::settings::integrations::visible_plugin(
                                &p.summary, &hidden,
                            )
                            && p.matches(query)
                    })
                    .collect();
                if plugins.is_empty() {
                    return self.status("No plugins found", None, cx);
                }
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(px(8.))
                    .children(
                        plugins
                            .into_iter()
                            .map(|plugin| self.plugin_card(plugin, window, cx)),
                    )
                    .into_any_element()
            }
            Tab::Skills => self.skills_view(query, window, cx),
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
            Modal::Skill(skill) => (skill_title(skill),
                div().flex().flex_col().gap(px(12.))
                    .child(text(skill_description(skill).to_owned(), 14., 20., 400., t.text_secondary).whitespace_normal())
                    .child(div().h(px(360.)).children(self.skill_file.clone()))
                    .when(skill["scope"].is_string(), |content| content.child(self.action_button(
                        "toggle-preview-skill", if skill["enabled"] == false { "Enable" } else { "Disable" },
                        Action::EnableSkill {name:string(skill,"name").into(),path:string(skill,"path").into(),enabled:skill["enabled"] == false},
                        true, window, cx))).into_any_element(),
                Some(("Open", Action::OpenSkill(string(skill, "path").to_owned()), skill["path"].is_string()))),
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
