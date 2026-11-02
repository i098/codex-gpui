use super::*;
use crate::{focus, screens::settings::button};
use gpui::{AnyElement, ClipboardItem, Div};

impl Page {
    pub(super) fn render_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let query = self.query.read(cx).field_text().trim().to_lowercase();
        let mut body = div().id("plugins-body").flex().flex_col().gap(px(24.));
        if self.modal.is_none()
            && let Some(error) = &self.error
        {
            body = body.child(text(error.clone(), 14., 20., 400., t.text));
        }
        let header = if let Some(plugin) = &self.detail {
            body = body.child(self.detail_view(plugin, window, cx));
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .pb(px(24.))
                .child(text(plugin.title().to_owned(), 28., 36., 600., t.text))
                .child(self.action_button(
                    "back",
                    "Back",
                    Action::Navigate("/plugins".into()),
                    true,
                    window,
                    cx,
                ))
        } else {
            body = body.child(self.list_view(&query, window, cx));
            self.header(window, cx)
        };
        let mut page = div()
            .id("plugins-page")
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .pt(px(30.))
            .px(px(30.))
            .bg(t.surface)
            .child(header)
            .child(
                div()
                    .id("plugins-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb(px(30.))
                    .child(body),
            );
        if let Some(modal) = &self.modal {
            page = page.child(self.modal_view(modal, window, cx));
        }
        page.into_any_element()
    }

    /// The directory heading: title, search pill, Refresh, Manage and Add, then the subtitle.
    fn header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let (title, subtitle, add) = match self.tab {
            Tab::Plugins => (
                "Plugins",
                "Connect plugins to let Codex work across your tools",
                Action::Manage,
            ),
            Tab::Skills => (
                "Skills",
                "Extend Codex with task-specific skills",
                Action::NewSkill,
            ),
        };
        let add_enabled = self.tab == Tab::Plugins
            || self
                .data
                .skills
                .value()
                .into_iter()
                .flatten()
                .any(|skill| skill["name"] == "skill-creator" && skill["enabled"] == true);
        let search = div()
            .id("plugins-search")
            .w(px(240.))
            .h(px(32.))
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(12.))
            .pr(px(12.))
            .rounded_full()
            .border_1()
            .border_color(t.border_strong)
            .child(icon("search", 16., t.text_secondary))
            .child(div().flex_1().min_w_0().child(self.query.clone()));
        div()
            .flex()
            .flex_col()
            // Plugins' pills start 18 px under the subtitle, Skills' `Installed` header 14 px.
            .pb(px(if self.tab == Tab::Plugins { 18. } else { 14. }))
            .child(
                div()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.))
                    .child(text(title, 24., 28.8, 500., t.text))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(search)
                            .child(self.icon_button(
                                "refresh",
                                "Refresh",
                                "settings-hooks-reload",
                                Action::Refresh,
                                window,
                                cx,
                            ))
                            .child(self.icon_button(
                                "manage",
                                "Manage",
                                "settings-general",
                                Action::Manage,
                                window,
                                cx,
                            ))
                            .child(self.add_button(add, add_enabled, window, cx)),
                    ),
            )
            .child(
                div()
                    .mt(px(9.))
                    .child(text(subtitle, 14., 24., 430., t.text_secondary)),
            )
    }

    /// A 32 px round ghost button with a 16 px icon.
    fn icon_button(
        &self,
        id: &'static str,
        label: &'static str,
        glyph: &'static str,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let handle = focus::handle(id, 0, window, cx);
        let key_action = action.clone();
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .size(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|d| d.bg(t.ghost_hover))
                .child(icon(glyph, 16., t.text_tertiary)),
            &handle,
            px(16.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| this.act(action.clone(), cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.act(key_action.clone(), cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }

    /// The white `Add` pill: a new skill on Skills, plugin management on Plugins.
    fn add_button(
        &self,
        action: Action,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let handle = focus::handle("add", 0, window, cx);
        let key_action = action.clone();
        let enabled = enabled && !self.pending;
        let ink = gpui::rgb(0x2d2d2d).into();
        focus::ring(
            div()
                .id("add")
                .role(gpui::Role::Button)
                .aria_label("Add")
                .h(px(32.))
                .flex()
                .items_center()
                .gap(px(4.))
                .pl(px(10.))
                .pr(px(if self.tab == Tab::Plugins { 8. } else { 10. }))
                .rounded_full()
                .bg(gpui::white())
                .border_1()
                .border_color(t.border)
                .child(text("Add", 13., 18., 430., ink))
                .when(self.tab == Tab::Plugins, |d| {
                    d.child(icon("chevron-down-small", 16., ink))
                })
                .when(!enabled, |d| d.opacity(0.5)),
            &handle,
            px(16.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if enabled {
                this.act(action.clone(), cx);
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if enabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.act(key_action.clone(), cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }

    /// The 36 px segment pills under the heading; the selected one is `bg-text/8`.
    fn pills(&self, labels: &[&'static str], window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        div()
            .flex()
            .gap(px(4.))
            .children(labels.iter().map(|&label| {
                let selected = self.section == label;
                let id = format!("section-{label}");
                let handle = focus::handle(id.clone(), 0, window, cx);
                focus::ring(
                    div()
                        .id(id)
                        .role(gpui::Role::Tab)
                        .aria_label(label)
                        .h(px(36.))
                        .px(px(16.))
                        .flex()
                        .items_center()
                        .rounded_full()
                        .when(selected, |d| d.bg(t.list_hover))
                        .child(text(
                            label,
                            13.,
                            18.57,
                            500.,
                            if selected { t.text } else { t.text_secondary },
                        )),
                    &handle,
                    px(18.),
                    px(2.),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.section = label;
                    cx.notify();
                }))
                .on_key_down(cx.listener(
                    move |this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.section = label;
                            cx.notify();
                            cx.stop_propagation();
                        }
                    },
                ))
            }))
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
            .gap(px(7.))
            .pt(px(44.))
            .child(text(state.to_owned(), 16., 24., 500., t.text))
            .children(
                description.map(|value| text(value.to_owned(), 13., 18.57, 430., t.text_secondary)),
            )
            .into_any_element()
    }

    /// A load failure with the original's `Retry`.
    fn failure(
        &self,
        state: &str,
        error: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .child(self.status(state, Some(error), cx))
            .child(self.action_button("retry", "Retry", Action::Refresh, true, window, cx))
            .into_any_element()
    }

    fn list_view(&self, query: &str, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.tab {
            Tab::Plugins => div()
                .flex()
                .flex_col()
                .child(self.pills(&["Public", "Personal"], window, cx))
                .child(self.plugins_list(query, window, cx))
                .into_any_element(),
            Tab::Skills => self.skills_list(query, window, cx),
        }
    }

    fn plugins_list(&self, query: &str, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if !self.available {
            return self.status("Plugins are not available for this host", None, cx);
        }
        let markets = match &self.data.plugins {
            Resource::Loading => return self.status("Loading plugins…", None, cx),
            Resource::Failed(error) => {
                return self.failure("Failed to load plugins", error, window, cx);
            }
            Resource::Ready(value) => value,
        };
        let personal = self.section == "Personal";
        let sections: Vec<_> = markets
            .iter()
            .filter_map(|market| {
                let plugins: Vec<_> = market
                    .plugins
                    .iter()
                    .filter(|p| {
                        p.visible()
                            && p.matches(query)
                            && self.data.personal_path(p).is_some() == personal
                    })
                    .collect();
                (!plugins.is_empty()).then_some((market, plugins))
            })
            .collect();
        if sections.is_empty() {
            return self.status("No plugins found", None, cx);
        }
        let t = cx.theme().clone();
        div()
            .flex()
            .flex_col()
            .children(sections.into_iter().map(|(market, plugins)| {
                // A signed-in catalog holds ~10k remote plugins; render pages, not all cards.
                let shown = plugins
                    .len()
                    .min(self.plugin_rows.get(&market.name).copied().unwrap_or(6));
                let mut section = div()
                    .mt(px(24.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(text(market.title.clone(), 16., 24., 500., t.text))
                    .child(
                        div().grid().grid_cols(2).gap(px(16.)).children(
                            plugins[..shown]
                                .iter()
                                .map(|plugin| self.plugin_card(plugin, window, cx)),
                        ),
                    );
                if shown < plugins.len() {
                    let name = market.name.clone();
                    let id = format!("plugins-more-{name}");
                    let handle = focus::handle(id.clone(), 0, window, cx);
                    section = section.child(
                        focus::ring(
                            div()
                                .id(id)
                                .role(gpui::Role::Button)
                                .aria_label("See more")
                                .min_h(px(31.))
                                .px(px(8.))
                                .flex()
                                .items_center()
                                .rounded_full()
                                .hover(|d| d.bg(t.ghost_hover))
                                .child(text(
                                    format!("See more ({} more)", plugins.len() - shown),
                                    13.,
                                    21.125,
                                    430.,
                                    t.text_secondary,
                                )),
                            &handle,
                            px(16.),
                            px(2.),
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            *this.plugin_rows.entry(name.clone()).or_insert(6) += 60;
                            cx.notify();
                        })),
                    );
                }
                section
            }))
            .into_any_element()
    }

    /// `Installed` (every standalone skill), then the `Personal` and `System` groups.
    fn skills_list(&self, query: &str, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme().clone();
        let skills = match &self.data.skills {
            Resource::Loading => return self.status("Loading skills…", None, cx),
            Resource::Failed(error) => {
                return self.failure("Failed to load skills", error, window, cx);
            }
            Resource::Ready(value) => value,
        };
        let mut installed: Vec<_> = skills
            .iter()
            .filter(|skill| skill["pluginId"].is_null() && matches_value(skill, query))
            .collect();
        if installed.is_empty() {
            return self.status("No skills found", None, cx);
        }
        installed.sort_by_cached_key(|skill| skill_title(skill).to_lowercase());
        let scope = if self.section == "System" {
            "system"
        } else {
            "user"
        };
        let group: Vec<_> = installed
            .iter()
            .copied()
            .filter(|skill| skill["scope"] == scope)
            .collect();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(37.))
                    .pl(px(8.))
                    .pb(px(8.))
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(t.border_subtle)
                    .child(text("Installed", 16., 24., 500., t.text)),
            )
            .child(
                self.skill_grid("installed", &installed, window, cx)
                    .mt(px(16.)),
            )
            .child(self.pills(&["Personal", "System"], window, cx).mt(px(32.)))
            .child(
                self.skill_grid(self.section, &group, window, cx)
                    .mt(px(40.)),
            )
            .into_any_element()
    }

    /// Two columns of skill rows: the first six, then `See A, B, and N more` until expanded.
    fn skill_grid(
        &self,
        list: &'static str,
        skills: &[&Value],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let shown = if self.expanded.contains(list) {
            skills.len()
        } else {
            skills.len().min(6)
        };
        let grid = div().grid().grid_cols(2).gap(px(8.)).children(
            skills[..shown]
                .iter()
                .map(|skill| self.skill_row(list, skill, window, cx)),
        );
        let rest = &skills[shown..];
        let mut column = div().flex().flex_col().child(grid);
        if !rest.is_empty() {
            let label = match rest {
                [one] => format!("See {}", skill_title(one)),
                [one, two] => format!("See {} and {}", skill_title(one), skill_title(two)),
                [one, two, more @ ..] => format!(
                    "See {}, {}, and {} more",
                    skill_title(one),
                    skill_title(two),
                    more.len()
                ),
                [] => unreachable!(),
            };
            let id = format!("see-more-{list}");
            let handle = focus::handle(id.clone(), 0, window, cx);
            column = column.child(
                focus::ring(
                    div()
                        .id(id)
                        .role(gpui::Role::Button)
                        .aria_label(label.clone())
                        // The Installed column's gap is 16 px; the tab section's is 32 px.
                        .mt(px(if list == "installed" { 16. } else { 32. }))
                        .min_h(px(31.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded_full()
                        .hover(|d| d.bg(t.ghost_hover))
                        .child(text(label, 13., 21.125, 400., t.text_tertiary)),
                    &handle,
                    px(16.),
                    px(2.),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.expanded.insert(list);
                    cx.notify();
                })),
            );
        }
        column
    }

    /// A skill row: icon tile, name and one-line description, and the enabled check, which
    /// toggles the skill. The row opens the skill.
    fn skill_row(
        &self,
        list: &'static str,
        skill: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let path = string(skill, "path");
        let enabled = skill["enabled"].as_bool().unwrap_or(true);
        let id = format!("skill-{list}-{path}");
        let handle = focus::handle(id.clone(), 0, window, cx);
        let toggle = Action::EnableSkill {
            name: string(skill, "name").into(),
            path: path.into(),
            enabled: !enabled,
        };
        let check_id = format!("skill-toggle-{list}-{path}");
        let check_handle = focus::handle(check_id.clone(), 0, window, cx);
        let check = focus::ring(
            div()
                .id(check_id)
                .role(gpui::Role::CheckBox)
                .aria_label("Skill enabled")
                .aria_toggled(if enabled {
                    gpui::Toggled::True
                } else {
                    gpui::Toggled::False
                })
                .size(px(24.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .child(
                    icon("diff-option-check", 18., t.text_tertiary).opacity(if enabled {
                        0.6
                    } else {
                        0.15
                    }),
                ),
            &check_handle,
            px(6.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.act(toggle.clone(), cx);
            cx.stop_propagation();
        }));
        let modal = Modal::Skill(skill.clone());
        let key_modal = modal.clone();
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(skill_title(skill))
                // `p-2` measured from the border box: the 1 px border is inside the 8 px inset.
                .p(px(7.))
                .border_1()
                .border_color(gpui::hsla(0., 0., 1., 0.033))
                .flex()
                .items_center()
                .gap(px(12.))
                .rounded(px(20.))
                .hover(|d| d.bg(t.text_alpha_5))
                .child(
                    div()
                        .size(px(40.))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(12.5))
                        .bg(gpui::rgb(0x141414))
                        .child(gpui::img("icons/skill.svg").size(px(24.))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(text(skill_title(skill), 14., 21., 500., t.text).truncate())
                        .child(
                            text(
                                skill_description(skill).to_owned(),
                                13.,
                                21.125,
                                430.,
                                t.text_secondary,
                            )
                            .truncate(),
                        ),
                )
                .child(check),
            &handle,
            px(20.),
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
        view = view.child(self.detail_actions(plugin, detail, &t, window, cx));
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

    fn detail_actions(
        &self,
        plugin: &Arc<Plugin>,
        detail: &Value,
        t: &theme::Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
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
                button("Copy link", t.text_alpha_5, None, t)
                    .id("copy-plugin-link")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()))
                    }),
            );
        }
        actions
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
                    content = content.child(self.action_button(
                        "update-shared-version",
                        "Update shared version",
                        Action::UpdateSharedVersion(plugin.clone()),
                        true,
                        window,
                        cx,
                    ));
                    content = content.child(text("Discoverability", 14., 20., 500., t.text));
                    for discoverability in ["PRIVATE", "UNLISTED", "LISTED"] {
                        let selected = self.share_discoverability == discoverability;
                        content = content.child(
                            button(
                                discoverability,
                                if selected {
                                    t.surface_secondary
                                } else {
                                    t.text_alpha_5
                                },
                                None,
                                &t,
                            )
                            .id(format!("share-discoverability-{discoverability}"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.share_discoverability = discoverability.to_owned();
                                cx.notify();
                            })),
                        );
                    }
                    content =
                        content.child(text("People and teams with access", 14., 20., 500., t.text));
                    for (index, target) in self.share_targets.iter().enumerate() {
                        let label = target["name"]
                            .as_str()
                            .unwrap_or("Unknown target")
                            .to_owned();
                        content = content.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(text(label, 13., 18., 400., t.text_secondary))
                                .child(
                                    button("Remove", t.text_alpha_5, None, &t)
                                        .id(format!("remove-share-target-{index}"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if index < this.share_targets.len() {
                                                this.share_targets.remove(index);
                                                cx.notify();
                                            }
                                        })),
                                ),
                        );
                    }
                    if self.pending {
                        content = content.child(text("Loading access settings…", 13., 18., 400., t.text_secondary));
                    }
                    if let Some(error) = &self.error {
                        content = content.child(text(error.clone(), 13., 18., 400., t.text_secondary));
                    }
                    content = content.child(self.action_button(
                        "delete-share",
                        "Unshare",
                        Action::DeleteShare(plugin.clone()),
                        true,
                        window,
                        cx,
                    ));
                    let update = self.error.is_none().then(|| {
                        ("Save access", Action::UpdateShareTargets(plugin.clone()), true)
                    });
                    (format!("Share {}", plugin.title()), content.into_any_element(), update)
                } else {
                    content = content.child(self.action_button("create-share-link", "Copy link",
                        Action::CreateShare(plugin.clone()), true, window, cx));
                    (format!("Share {}", plugin.title()), content.into_any_element(), None)
                }
            }
            Modal::Skill(skill) => {
                let open_action = if is_remote_skill(skill) {
                    None
                } else {
                    Some(("Open", Action::OpenSkill(string(skill, "path").to_owned()), true))
                };
                (
                    string(skill, "name").to_owned(),
                    div().flex().flex_col().gap(px(12.))
                        .child(text(string(skill, "description").to_owned(), 14., 20., 400., t.text_secondary).whitespace_normal())
                        .child(div().h(px(360.)).children(self.skill_file.clone())).into_any_element(),
                    open_action,
                )
            }
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

/// The interface display name, else the original's title formatter (`GH`): `-`/`_` become
/// spaces, known acronyms and brands keep their casing, and `and`, `or`, `to`, `up`, `with`
/// stay lower case after the first word (`aws-billing-and-cost-management` is
/// `Aws Billing and Cost Management`).
fn skill_title(skill: &Value) -> String {
    const ACRONYMS: [&str; 16] = [
        "GH", "IA", "MCP", "API", "CI", "CLI", "LLM", "PDF", "PR", "RCS", "UI", "URL", "SQL", "TW",
        "GPU", "CPU",
    ];
    const BRANDS: [(&str, &str); 10] = [
        ("openai", "OpenAI"),
        ("openaideveloperdocs", "OpenAI Developer Docs"),
        ("openapi", "OpenAPI"),
        ("github", "GitHub"),
        ("imessage", "iMessage"),
        ("pagerduty", "PagerDuty"),
        ("datadog", "DataDog"),
        ("sharepoint", "SharePoint"),
        ("sqlite", "SQLite"),
        ("fastapi", "FastAPI"),
    ];
    if let Some(name) = skill["interface"]["displayName"].as_str() {
        return name.to_owned();
    }
    let word = |index: usize, word: &str| {
        let upper = word.to_uppercase();
        if ACRONYMS.contains(&upper.as_str()) {
            return upper;
        }
        if word.ends_with(['s', 'S'])
            && let Some(stem) = upper.strip_suffix('S')
            && ACRONYMS.contains(&stem)
        {
            return format!("{stem}s");
        }
        let lower = word.to_lowercase();
        if let Some((_, brand)) = BRANDS.iter().find(|(key, _)| *key == lower) {
            return (*brand).to_owned();
        }
        if index > 0 && ["and", "or", "to", "up", "with"].contains(&lower.as_str()) {
            return lower;
        }
        let mut chars = lower.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        })
    };
    string(skill, "name")
        .split(':')
        .map(|part| {
            part.replace(['-', '_'], " ")
                .split_whitespace()
                .enumerate()
                .map(|(index, part)| word(index, part))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(": ")
}

/// The interface short description when it is not empty, else the full description.
fn skill_description(skill: &Value) -> &str {
    skill["interface"]["shortDescription"]
        .as_str()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| string(skill, "description"))
}
fn is_remote_skill(skill: &Value) -> bool {
    skill
        .get("remotePluginSkill")
        .is_some_and(|remote| !remote.is_null())
        || !skill["remotePluginId"].is_null()
        || !skill["remoteMarketplaceName"].is_null()
}

#[cfg(test)]
#[test]
fn remote_skills_do_not_offer_local_open_action() {
    assert!(is_remote_skill(
        &serde_json::json!({"remotePluginSkill": {"remotePluginId": "remote"}})
    ));
    assert!(is_remote_skill(
        &serde_json::json!({"remotePluginId": "remote-plugin"})
    ));
    assert!(!is_remote_skill(
        &serde_json::json!({"path": "/tmp/SKILL.md"})
    ));
}

#[test]
fn skill_titles_fall_back_to_title_cased_names() {
    let title = |name: &str| skill_title(&serde_json::json!({ "name": name }));
    assert_eq!(title("aws-cdk"), "Aws Cdk");
    assert_eq!(
        title("aws-billing-and-cost-management"),
        "Aws Billing and Cost Management"
    );
    assert_eq!(title("and-then"), "And Then");
    assert_eq!(title("github_pr-reviews"), "GitHub PR Reviews");
    assert_eq!(title("gh-apis"), "GH APIs");
    assert_eq!(title("openai:docs"), "OpenAI: Docs");
    assert_eq!(
        skill_title(&serde_json::json!({"name": "x", "interface": {"displayName": "Custom"}})),
        "Custom"
    );
}
