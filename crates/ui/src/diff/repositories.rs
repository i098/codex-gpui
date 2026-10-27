use super::*;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ReviewRepository {
    pub root: String,
    pub cwd: String,
    pub name: String,
}

impl DiffPanel {
    pub fn working_directory(&self) -> &str {
        if self.source == DiffSource::LastTurn {
            return &self.cwd;
        }
        self.selected_repository
            .as_deref()
            .and_then(|selected| {
                self.repositories
                    .iter()
                    .find(|repository| repository.root == selected)
            })
            .map_or(&self.cwd, |repository| repository.cwd.as_str())
    }

    pub fn repositories(&self) -> &[ReviewRepository] {
        &self.repositories
    }

    pub fn set_repositories(
        &mut self,
        repositories: Vec<ReviewRepository>,
        cx: &mut Context<Self>,
    ) {
        self.repositories = repositories;
        if self.selected_repository.as_ref().is_some_and(|selected| {
            !self
                .repositories
                .iter()
                .any(|repository| &repository.root == selected)
        }) {
            self.selected_repository = None;
            self.repository = None;
            self.select_source(DiffSource::LastTurn, cx);
        }
        self.refresh_picker();
        cx.notify();
    }

    pub(super) fn select_repository(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(repository) = self.repositories.get(index) {
            self.selected_repository = Some(repository.root.clone());
            self.repository = None;
            self.base_branch = None;
            self.commit = None;
            let source = if self.source == DiffSource::LastTurn {
                DiffSource::Uncommitted
            } else {
                self.source
            };
            self.select_source(source, cx);
        }
        cx.notify();
    }

    pub(super) fn repository_label(&self) -> Option<&str> {
        if self.source == DiffSource::Cloud || self.repositories.len() < 2 {
            return None;
        }
        let selected = self.selected_repository.as_deref().or_else(|| {
            self.repository
                .as_ref()
                .map(|repository| repository.root.as_str())
        });
        let label = if self.source == DiffSource::LastTurn {
            "All repos"
        } else {
            self.repositories
                .iter()
                .find(|repository| Some(repository.root.as_str()) == selected)
                .unwrap_or(&self.repositories[0])
                .name
                .as_str()
        };
        Some(label)
    }

    pub(super) fn repository_control(
        &self,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let label = self.repository_label()?;
        Some(
            self.selector_button(
                label.to_owned(),
                "Select repository",
                menu::Kind::Repositories,
                t,
                cx,
            )
            .into_any_element(),
        )
    }

    pub(super) fn comparison_control(
        &self,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        match self.source {
            DiffSource::Branch => {
                let current = self
                    .repository
                    .as_ref()
                    .and_then(|repository| repository.head_branch.as_deref())
                    .unwrap_or("HEAD");
                let target = self.base_branch.as_deref().unwrap_or("main");
                Some(
                    self.selector_button(
                        format!("{current} → {target}"),
                        "Branch comparison",
                        menu::Kind::Branches,
                        t,
                        cx,
                    )
                    .into_any_element(),
                )
            }
            DiffSource::Commits => {
                let label = self
                    .repository
                    .as_ref()
                    .and_then(|repository| {
                        repository
                            .commits
                            .iter()
                            .find(|commit| Some(&commit.id) == self.commit.as_ref())
                    })
                    .map(|commit| commit.label.as_str())
                    .unwrap_or("Select commit");
                Some(
                    self.selector_button(
                        label.to_owned(),
                        "Selected commit",
                        menu::Kind::Commits,
                        t,
                        cx,
                    )
                    .into_any_element(),
                )
            }
            _ => None,
        }
    }

    fn selector_button(
        &self,
        label: String,
        description: &'static str,
        kind: menu::Kind,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(SharedString::from(format!("diff-{description}")))
            .role(gpui::Role::Button)
            .aria_label(description)
            .aria_disabled(self.pending)
            .aria_expanded(self.menu.as_ref().is_some_and(|menu| menu.kind == kind))
            .when(!self.pending, |d| d.tab_index(0).cursor_pointer())
            .h(px(28.))
            .max_w(px(230.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(4.))
            .child(text(label, 13., 20., 430., t.text).min_w_0().truncate())
            .child(icon("diff-chevron-down", 12., t.text))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if this.pending {
                    return;
                }
                this.open_menu(0, kind, event.position(), window, cx);
                if this.repository.is_none() && kind != menu::Kind::Repositories {
                    cx.emit(DiffRequest::Refresh);
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if !this.pending && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    let position = gpui::point(
                        window.viewport_size().width - panel_width(window, cx) + px(8.),
                        px(84.),
                    );
                    this.open_menu(0, kind, position, window, cx);
                    if this.repository.is_none() && kind != menu::Kind::Repositories {
                        cx.emit(DiffRequest::Refresh);
                    }
                    cx.stop_propagation();
                }
            }))
    }
}
