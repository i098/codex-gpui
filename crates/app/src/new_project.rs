//! `File > New Project…`: the native create-project form over the host's project store.

use crate::Root;
use bridge::types::ThreadListResponse;
use gpui::{AppContext as _, Context, Entity, Focusable as _, PathPromptOptions, Window};
use serde_json::json;
use std::path::PathBuf;
use ui::screens::{
    connections::{ConnectionsState, Tab},
    new_project::{Dialog, Outcome},
};
use ui::toast::Toast;

impl Root {
    pub fn new_project(&self) -> Option<&Entity<Dialog>> {
        self.new_project.as_ref()
    }

    pub(crate) fn open_new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_dialog() {
            return;
        }
        let remotes = cx
            .try_global::<ConnectionsState>()
            .and_then(|state| state.connections.as_ref())
            .into_iter()
            .flatten()
            .filter(|connection| connection.state == "connected")
            .map(|connection| connection.display_name.clone().into())
            .collect();
        let dialog = cx.new(|cx| Dialog::new(remotes, cx));
        self.show_project_dialog(dialog, None, window, cx);
    }

    /// `Edit project` from a project row's menu: the same form, prefilled, saving over `id`.
    pub(crate) fn open_edit_project(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.has_dialog() {
            return;
        }
        let Some(project) = self
            .sidebar
            .read(cx)
            .projects()
            .iter()
            .find(|p| p.id.as_ref() == id)
        else {
            return;
        };
        let sources = project.roots.iter().map(ToString::to_string).collect();
        let label = project.label.clone();
        let dialog = cx.new(|cx| Dialog::edit(&label, sources, cx));
        self.show_project_dialog(dialog, Some(id), window, cx);
    }

    fn show_project_dialog(
        &mut self,
        dialog: Entity<Dialog>,
        editing: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.subscribe_in(
            &dialog,
            window,
            move |this, dialog, outcome, window, cx| match outcome {
                Outcome::Dismiss => this.close_new_project(window, cx),
                Outcome::AddRemote => {
                    this.close_new_project(window, cx);
                    this.open_connections(Tab::Ssh, window, cx);
                }
                Outcome::PickFolders => this.pick_project_sources(dialog.clone(), window, cx),
                Outcome::Create {
                    name,
                    sources,
                    appearance,
                } => match &editing {
                    Some(id) => this.update_project(
                        dialog.clone(),
                        id.clone(),
                        name.clone(),
                        sources.clone(),
                        appearance.clone(),
                        window,
                        cx,
                    ),
                    None => this.create_project(
                        dialog.clone(),
                        name.clone(),
                        sources.clone(),
                        appearance.clone(),
                        window,
                        cx,
                    ),
                },
            },
        )
        .detach();
        window.focus(&dialog.focus_handle(cx), cx);
        self.new_project = Some(dialog);
        cx.notify();
    }

    pub(crate) fn close_new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_project = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn pick_project_sources(
        &mut self,
        dialog: Entity<Dialog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Choose source folders".into()),
        });
        cx.spawn_in(window, async move |_, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let paths = paths
                .into_iter()
                .filter_map(|path| path.to_str().map(str::to_owned));
            dialog.update(cx, |dialog, cx| dialog.add_sources(paths, cx));
        })
        .detach();
    }

    fn create_project(
        &mut self,
        dialog: Entity<Dialog>,
        name: String,
        sources: Vec<String>,
        appearance: Option<serde_json::Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                // The chats the new roots could take over, pinned before the store changes.
                let mut threads = Vec::new();
                if !sources.is_empty() {
                    for archived in [false, true] {
                        let mut cursor: Option<String> = None;
                        loop {
                            let page: ThreadListResponse = bridge
                                .request(
                                    "thread/list",
                                    json!({"cursor": cursor, "limit": 100, "cwd": sources,
                                           "archived": archived, "useStateDbOnly": true}),
                                )
                                .await?;
                            threads.extend(page.data.into_iter().map(|t| (t.id, t.cwd)));
                            cursor = page.next_cursor;
                            if cursor.is_none() {
                                break;
                            }
                        }
                    }
                }
                cx.background_spawn(async move {
                    let roots: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
                    let root = host.create_project(&name, &roots, appearance, &threads)?;
                    Ok::<_, anyhow::Error>((root, host.projects()?))
                })
                .await
            }
            .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok((root, projects)) => {
                    if this.new_project.as_ref() == Some(&dialog) {
                        this.close_new_project(window, cx);
                    }
                    this.set_projects(projects, cx);
                    this.select_project(root, window, cx);
                }
                Err(error) => {
                    eprintln!("codex-gpui: cannot create project: {error:#}");
                    dialog.update(cx, |dialog, cx| dialog.finish(cx));
                    this.toast(Toast::danger("Failed to create project"), cx);
                }
            })
            .ok();
        })
        .detach();
    }
}
