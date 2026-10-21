use super::*;
use gpui::{AnyElement, Corners, ImageSource, StyledImage, img};
use theme::Appearance;

#[derive(Clone, Copy)]
enum Group {
    Installed,
    Selected,
}

fn icon_value<'a>(skill: &'a Value, keys: [&str; 2]) -> Option<&'a str> {
    let fields = skill
        .get("interface")
        .filter(|value| value.is_object())
        .unwrap_or(skill);
    keys.into_iter()
        .find_map(|key| fields[key].as_str().filter(|value| !value.is_empty()))
}

impl Page {
    pub(super) fn skills_view(
        &self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let skills = match &self.data.skills {
            Resource::Loading => return self.status("Loading skills…", None, cx),
            Resource::Failed(error) => {
                return self.failure("Failed to load skills", error, window, cx);
            }
            Resource::Ready(skills) => skills,
        };
        let matches = |skill: &&Value| {
            query.is_empty()
                || skill_title(skill).to_lowercase().contains(query)
                || skill_description(skill).to_lowercase().contains(query)
                || string(skill, "name").to_lowercase().contains(query)
        };
        let filtered: Vec<_> = skills.iter().filter(matches).collect();
        if !query.is_empty() {
            return if filtered.is_empty() {
                text("No skills found", 14., 21., 430., t.text_secondary).into_any_element()
            } else {
                self.skill_section("Skills", &filtered, Group::Installed, true, window, cx)
            };
        }
        let mut content = div().flex().flex_col().gap(px(32.));
        content = content.child(self.skill_section(
            "Installed",
            &filtered,
            Group::Installed,
            false,
            window,
            cx,
        ));
        let mut tabs = div().flex().gap(px(4.)).mb(px(8.));
        for section in SkillSection::ALL {
            let active = self.skill_section == section;
            let handle =
                crate::focus::handle(format!("skill-section-{}", section.label()), 0, window, cx);
            tabs = tabs.child(
                crate::focus::ring(
                    div()
                        .id(section.label())
                        .role(gpui::Role::Tab)
                        .h(px(36.))
                        .px(px(17.))
                        .flex()
                        .items_center()
                        .relative()
                        .cursor_pointer()
                        .when(active, |d| {
                            d.child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(px(9999.)),
                                corner: shapes::Corner::Round,
                                fill: Some(t.ghost_hover),
                                ..Default::default()
                            }))
                        })
                        .child(text(
                            section.label(),
                            13.,
                            18.5714,
                            500.,
                            if active { t.text } else { t.text_secondary },
                        )),
                    &handle,
                    px(9999.),
                    px(2.),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.select_skill_section(section, cx)))
                .on_key_down(cx.listener(
                    move |this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.select_skill_section(section, cx);
                            cx.stop_propagation();
                        }
                    },
                )),
            );
        }
        content = content.child(tabs);
        if self.skill_section == SkillSection::Recommended
            && let Some(error) = &self.recommended_error
        {
            content = content.child(text(error.clone(), 14., 21., 430., t.text_secondary));
        }
        if self.skill_section == SkillSection::Recommended {
            return content
                .child(match &self.recommended {
                    Resource::Loading => self.status("Loading skills…", None, cx),
                    Resource::Failed(error) => {
                        self.status("Unable to load recommended skills", Some(error), cx)
                    }
                    Resource::Ready(skills) => {
                        let skills: Vec<_> = skills
                            .iter()
                            .filter(|skill| {
                                !filtered
                                    .iter()
                                    .any(|installed| installed["name"] == skill["name"])
                            })
                            .collect();
                        self.skill_grid(&skills, Group::Selected, false, false, window, cx)
                    }
                })
                .into_any_element();
        }
        let selected: Vec<_> = filtered
            .into_iter()
            .filter(|skill| match self.skill_section {
                SkillSection::Personal => skill["scope"] == "user" || skill["scope"] == "repo",
                SkillSection::System => skill["scope"] == "system" || skill["scope"] == "admin",
                SkillSection::Recommended => false,
            })
            .collect();
        content
            .child(self.skill_grid(&selected, Group::Selected, false, true, window, cx))
            .into_any_element()
    }

    fn skill_section(
        &self,
        title: &'static str,
        skills: &[&Value],
        group: Group,
        search: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .pl(px(8.))
                    .pr(px(2.))
                    .pb(px(8.))
                    .border_b_1()
                    .border_color(t.text.opacity(0.043))
                    .child(
                        div()
                            .h(px(28.))
                            .flex()
                            .items_center()
                            .child(text(title, 16., 24., 500., t.text)),
                    ),
            )
            .child(self.skill_grid(skills, group, search, true, window, cx))
            .into_any_element()
    }

    fn skill_grid(
        &self,
        skills: &[&Value],
        group: Group,
        all: bool,
        installed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        if skills.is_empty() {
            return text("No skills found", 14., 21., 430., t.text_secondary).into_any_element();
        }
        let expanded = all
            || match group {
                Group::Installed => self.installed_expanded,
                Group::Selected => self.section_expanded,
            };
        let limit = if expanded {
            skills.len()
        } else {
            skills.len().min(6)
        };
        let visible = &skills[..limit];
        let gap = if !all && skills.len() > 6 { 8. } else { 16. };
        let cards = div().grid().grid_cols(2).gap(px(gap)).children(
            visible
                .iter()
                .map(|skill| self.skill_card(skill, group, installed, window, cx)),
        );
        let mut group_view = div()
            .flex()
            .flex_col()
            .gap(px(match group {
                Group::Installed => 16.,
                Group::Selected => 32.,
            }))
            .child(cards);
        if !all && skills.len() > 6 {
            let label = if expanded {
                "Show less".to_owned()
            } else {
                let hidden = &skills[limit..];
                let names = hidden
                    .iter()
                    .take(2)
                    .map(|skill| skill_title(skill))
                    .collect::<Vec<_>>()
                    .join(", ");
                let remaining = hidden.len().saturating_sub(2);
                if remaining == 0 {
                    format!("See {names}")
                } else {
                    format!("See {names}, and {remaining} more")
                }
            };
            let id = match group {
                Group::Installed => "installed-skills-more",
                Group::Selected => "selected-skills-more",
            };
            let handle = crate::focus::handle(id, 0, window, cx);
            group_view = group_view.child(
                crate::focus::ring(
                    div()
                        .id(id)
                        .role(gpui::Role::Button)
                        .aria_label(label.clone())
                        .min_h(px(30.))
                        .px(px(8.))
                        .py(px(5.))
                        .cursor_pointer()
                        .child(text(label, 13., 21.125, 400., t.text.opacity(0.498))),
                    &handle,
                    px(8.),
                    px(2.),
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_skill_group(group, cx)))
                .on_key_down(cx.listener(
                    move |this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.toggle_skill_group(group, cx);
                            cx.stop_propagation();
                        }
                    },
                )),
            );
        }
        group_view.into_any_element()
    }

    fn skill_card(
        &self,
        skill: &Value,
        group: Group,
        installed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let path = string(skill, if installed { "path" } else { "id" }).to_owned();
        let id = format!(
            "skill-card-{}-{path}",
            match group {
                Group::Installed => "installed",
                Group::Selected => "selected",
            }
        );
        let action = if installed {
            if skill["enabled"] == false {
                text("Disabled", 12., 16., 430., t.text_secondary).into_any_element()
            } else {
                div()
                    .size(px(28.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .opacity(0.6)
                            .child(icon("menu-check", 18., t.text.opacity(0.498))),
                    )
                    .into_any_element()
            }
        } else {
            let skill_id = path.clone();
            let keyboard_id = path.clone();
            let handle = crate::focus::handle(format!("install-skill-{path}"), 0, window, cx);
            crate::focus::ring(
                crate::screens::settings::button("Install", t.text_alpha_5, None, &t)
                    .id(format!("install-skill-{path}"))
                    .role(gpui::Role::Button)
                    .aria_label("Install")
                    .when(self.pending, |button| button.opacity(0.4)),
                &handle,
                px(12.5),
                px(2.),
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.install_recommended(&skill_id, cx);
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.install_recommended(&keyboard_id, cx);
                    cx.stop_propagation();
                }
            }))
            .into_any_element()
        };
        let handle = crate::focus::handle(id.clone(), 0, window, cx);
        let keyboard_path = path.clone();
        crate::focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(skill_title(skill))
                .p(px(8.))
                .relative()
                .flex()
                .items_center()
                .gap(px(12.))
                .cursor_pointer()
                .child(
                    div()
                        .size(px(40.))
                        .flex_none()
                        .relative()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(shapes::decoration(shapes::Decoration {
                            radius: Corners::all(px(12.5)),
                            corner: shapes::Corner::Squircle,
                            fill: Some(t.surface_secondary),
                            ..Default::default()
                        }))
                        .child(self.skill_artwork(skill, cx)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(12.))
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
                                        t.text.opacity(0.65),
                                    )
                                    .whitespace_normal()
                                    .line_clamp(1)
                                    .text_ellipsis(),
                                ),
                        )
                        .child(action),
                ),
            &handle,
            px(20.),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, window, cx| this.open_skill(&path, window, cx)))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                this.open_skill(&keyboard_path, window, cx);
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }

    fn skill_artwork(&self, skill: &Value, cx: &App) -> AnyElement {
        let large = icon_value(skill, ["iconLargeUrl", "iconLarge"]);
        let source = large.or_else(|| icon_value(skill, ["iconSmallUrl", "iconSmall"]));
        let size = if large.is_some() { 40. } else { 24. };
        let fallback_name = match string(skill, "name") {
            "openai-docs" => "skill-openai-docs",
            "review-agent" => "skill-review-agent",
            "skill-installer" => "skill-installer",
            _ if cx.theme().appearance == Appearance::Dark => "skill-default-dark",
            _ => "skill-default-light",
        };
        let color = cx.theme().text.opacity(0.65);
        let fallback = move || icon(fallback_name, size, color).into_any_element();
        let resource = source.and_then(|source| {
            if source.starts_with("https://") || source.starts_with("http://") {
                Some(ImageSource::from(source.to_owned()))
            } else if source.contains("://") {
                None
            } else {
                let path = Path::new(source);
                Some(ImageSource::from(if path.is_absolute() {
                    path.to_owned()
                } else {
                    Path::new(string(skill, "path")).parent()?.join(path)
                }))
            }
        });
        match resource {
            Some(resource) => img(resource)
                .size(px(size))
                .with_loading(fallback)
                .with_fallback(fallback)
                .into_any_element(),
            None => fallback(),
        }
    }

    fn toggle_skill_group(&mut self, group: Group, cx: &mut Context<Self>) {
        match group {
            Group::Installed => self.installed_expanded = !self.installed_expanded,
            Group::Selected => self.section_expanded = !self.section_expanded,
        }
        cx.notify();
    }

    fn install_recommended(&mut self, skill_id: &str, cx: &mut Context<Self>) {
        if let Some(skill) = self
            .recommended
            .value()
            .and_then(|skills| skills.iter().find(|skill| skill["id"] == skill_id))
            .cloned()
        {
            self.act(Action::InstallRecommended(skill), cx);
        }
    }

    fn open_skill(&mut self, skill_key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let skill = self
            .data
            .skills
            .value()
            .into_iter()
            .flatten()
            .chain(self.recommended.value().into_iter().flatten())
            .find(|skill| skill["path"] == skill_key || skill["id"] == skill_key)
            .cloned();
        if let Some(skill) = skill {
            self.show_modal(Modal::Skill(skill), window, cx);
        }
    }
}
