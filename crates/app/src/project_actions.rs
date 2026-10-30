use super::*;
use gpui::Div;
use ui::screens::sidebar::projects::{ProjectAction, ProjectCommand};

/// The archive dialog's id for a project's chats, so `archive_section` can tell it from a section.
pub(crate) const PROJECT_PREFIX: &str = "project:";

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, action: &ProjectAction, window, cx| {
        this.project_action(action, window, cx);
    }))
}

impl Root {
    fn project_action(
        &mut self,
        action: &ProjectAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = action.project.clone();
        match action.command {
            ProjectCommand::Pin(pinned) => {
                self.store_project(cx, move |host| host.set_project_pinned(&id, pinned));
            }
            ProjectCommand::Edit => self.open_edit_project(id, window, cx),
            ProjectCommand::MarkRead => {
                let members = self.project_threads(&id, cx);
                self.sidebar.update(cx, |sidebar, cx| {
                    sidebar.unread.retain(|thread| !members.contains(thread));
                    cx.notify();
                });
            }
            ProjectCommand::Archive => {
                if self.dialog.is_some() {
                    return;
                }
                let members = self.project_threads(&id, cx);
                if members.is_empty() {
                    return;
                }
                let running = self
                    .threads
                    .iter()
                    .any(|t| members.contains(&t.id) && t.status == ThreadStatus::Active);
                let Some(label) = self.project_label(&id, cx) else {
                    return;
                };
                let dialog = cx.new(|cx| {
                    Dialog::archive_section(
                        format!("{PROJECT_PREFIX}{id}"),
                        label,
                        members.len(),
                        running,
                        cx,
                    )
                });
                self.show_dialog(dialog, window, cx);
            }
            ProjectCommand::Remove => {
                if self.dialog.is_some() {
                    return;
                }
                let Some(label) = self.project_label(&id, cx) else {
                    return;
                };
                let dialog = cx.new(|cx| Dialog::remove_project(id, &label, cx));
                self.show_dialog(dialog, window, cx);
            }
        }
    }

    pub(crate) fn remove_project(&mut self, id: String, cx: &mut Context<Self>) {
        let deselect = self.state.project_root.is_some()
            && self.sidebar.read(cx).projects().iter().any(|p| {
                p.id.as_ref() == id && p.root.as_deref() == self.state.project_root.as_deref()
            });
        if deselect {
            self.state.project_root = None;
        }
        self.store_project(cx, move |host| host.remove_project(&id));
    }

    /// `Edit project` `Save`: the host rewrites the project, then the sidebar reloads.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_project(
        &mut self,
        dialog: Entity<ui::screens::new_project::Dialog>,
        id: String,
        name: String,
        sources: Vec<String>,
        appearance: Option<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.host.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let roots: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
                    host.update_project(&id, &name, &roots, appearance)?;
                    host.projects()
                })
                .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(projects) => {
                    if this.new_project.as_ref() == Some(&dialog) {
                        this.close_new_project(window, cx);
                    }
                    this.set_projects(projects, cx);
                }
                Err(error) => {
                    eprintln!("codex-gpui: cannot save project: {error:#}");
                    dialog.update(cx, |dialog, cx| dialog.finish(cx));
                    this.toast(Toast::danger("Failed to save project"), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Runs `change` on the host's project store off the UI thread, then reloads the rows.
    fn store_project(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&dyn host::Host) -> Result<()> + Send + 'static,
    ) {
        let host = self.host.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let result = cx
                .background_spawn(async move {
                    change(host.as_ref())?;
                    host.projects()
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(projects) => this.set_projects(projects, cx),
                Err(error) => {
                    eprintln!("codex-gpui: cannot update project: {error:#}");
                    this.toast(Toast::danger("Failed to update project"), cx);
                }
            })
        });
        log_err(task, cx);
    }

    fn project_label(&self, id: &str, cx: &App) -> Option<String> {
        let sidebar = self.sidebar.read(cx);
        let project = sidebar.projects().iter().find(|p| p.id.as_ref() == id)?;
        Some(project.label.to_string())
    }

    /// The loaded chats whose cwd is one of the project's roots or inside one.
    // shortcut: matches by cwd only, not `thread-project-assignments`; read that atom if moved chats must count.
    pub(crate) fn project_threads(&self, id: &str, cx: &App) -> Vec<String> {
        let sidebar = self.sidebar.read(cx);
        let Some(project) = sidebar.projects().iter().find(|p| p.id.as_ref() == id) else {
            return Vec::new();
        };
        self.threads
            .iter()
            .filter(|t| {
                project.roots.iter().any(|root| {
                    t.cwd
                        .strip_prefix(root.as_ref())
                        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
                })
            })
            .map(|t| t.id.clone())
            .collect()
    }
}
