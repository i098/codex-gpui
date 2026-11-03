use super::*;
use gpui::Div;
use ui::screens::sidebar::projects::{ProjectCommand, ProjectMenuAction};

/// The archive dialog's id for a project's chats, so `archive_section` can tell it from a section.
pub(crate) const PROJECT_PREFIX: &str = "project:";

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, action: &ProjectMenuAction, window, cx| {
        this.project_menu_action(action, window, cx);
    }))
}

impl Root {
    fn project_menu_action(
        &mut self,
        action: &ProjectMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = action.project.clone();
        match action.command {
            ProjectCommand::Pin(pinned) => {
                self.store_project(cx, move |host| host.set_project_pinned(&id, pinned));
            }
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
        }
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
