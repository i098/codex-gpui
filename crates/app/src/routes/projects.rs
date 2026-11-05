use crate::*;
use gpui::Div;
use ui::OpenProjects;
use ui::screens::projects::{self, Event, Load, Page};
use ui::screens::sidebar::{Project, ProjectAction};

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenProjects, window, cx| {
            this.navigate(Route::Projects, window, cx)
        }),
    )
}

/// The page over the host's project list (`project/list` on app-server hosts); every load
/// also refreshes the sidebar's list.
pub(super) fn page(root: &mut Root, window: &mut Window, cx: &mut Context<Root>) -> Entity<Shell> {
    let page = cx.new(|cx| Page::new(root.clock(), cx));
    cx.subscribe_in(&page, window, |this, page, event, window, cx| match event {
        Event::Retry => {
            page.update(cx, |page, cx| {
                page.load = Load::Loading;
                cx.notify();
            });
            this.load_projects(cx)
        }
        Event::Create => this.open_new_project(window, cx),
        Event::Open(root) => this.select_project(root.to_string(), window, cx),
        Event::Edit(id) => {
            let action = ProjectAction {
                id: id.to_string(),
                command: "edit".into(),
            };
            this.project_action(&action, window, cx)
        }
    })
    .detach();
    root.projects_page = Some(page.clone());
    root.load_projects(cx);
    projects::shell(page, cx)
}

impl Root {
    /// Shows a project list read on the open Projects page, with each row's `Updated` taken
    /// from the project and its loaded chats.
    pub(crate) fn show_projects(&self, read: Result<&[Project], String>, cx: &mut Context<Self>) {
        let Some(page) = &self.projects_page else {
            return;
        };
        let load = match read {
            Ok(projects) => {
                let sidebar = self.sidebar.read(cx);
                Load::Ready(
                    projects
                        .iter()
                        .map(|project| Project {
                            updated_at: sidebar.project_activity(project),
                            ..project.clone()
                        })
                        .collect(),
                )
            }
            Err(error) => Load::Failed(error),
        };
        page.update(cx, |page, cx| {
            page.load = load;
            cx.notify();
        });
    }
}
