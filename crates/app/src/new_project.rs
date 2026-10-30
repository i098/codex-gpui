//! `File > New Project…`: the native create-project form over the host's project store.

use crate::Root;
use crate::host;
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
            .map(|connection| {
                (
                    connection.host_id.clone(),
                    connection.display_name.clone().into(),
                )
            })
            .collect();
        let dialog = cx.new(|cx| Dialog::new(remotes, cx));
        cx.subscribe_in(
            &dialog,
            window,
            |this, dialog, outcome, window, cx| match outcome {
                Outcome::Dismiss => this.close_new_project(window, cx),
                Outcome::AddRemote => {
                    this.close_new_project(window, cx);
                    this.open_connections(Tab::Ssh, window, cx);
                }
                Outcome::PickFolders => this.pick_project_sources(dialog.clone(), window, cx),
                Outcome::Create {
                    host_id,
                    project_id,
                    name,
                    sources,
                    appearance,
                } => this.create_project(
                    dialog.clone(),
                    host_id.clone(),
                    project_id.clone(),
                    name.clone(),
                    sources.clone(),
                    appearance.clone(),
                    window,
                    cx,
                ),
            },
        )
        .detach();
        window.focus(&dialog.focus_handle(cx), cx);
        self.new_project = Some(dialog);
        cx.notify();
    }

    fn close_new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        host_id: String,
        project_id: Option<String>,
        name: String,
        sources: Vec<String>,
        appearance: Option<serde_json::Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.host.clone();
        let bridge = if host_id == "local" {
            Some(self.bridge.clone())
        } else {
            crate::connections::bridge(&host_id, cx)
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let bridge = bridge.as_ref().ok_or_else(|| anyhow::anyhow!("Project host is disconnected"))?;
                let root = if let Some(project_id) = project_id {
                    update_project(bridge, &project_id, &name, &sources, appearance).await?;
                    sources.first().cloned().unwrap_or_default()
                } else if host_id == "local" {
                    cx.background_spawn(async move {
                        let roots: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
                        host.create_project(&name, &roots, appearance, &[])
                    }).await?
                } else {
                    anyhow::ensure!(!sources.is_empty(), "Add a remote project folder");
                    let _: serde_json::Value = bridge.request("project/create", json!({
                        "name": name,
                        "roots": sources.iter().map(|path| json!({"path": path})).collect::<Vec<_>>(),
                        "metadata": host::project_metadata(json!({}), appearance)?,
                        "idempotencyKey": crate::uuid_v4(),
                    })).await?;
                    sources[0].clone()
                };
                Ok::<_, anyhow::Error>(root)
            }.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(root) => {
                    if this.new_project.as_ref() == Some(&dialog) {
                        this.close_new_project(window, cx);
                    }
                    this.load_projects(cx);
                    if this.host_id == host_id && !root.is_empty() {
                        this.select_project(root, window, cx);
                    }
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

    pub(crate) fn project_action(
        &mut self,
        action: &ui::screens::sidebar::ProjectAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bridge = self.bridge.clone();
        let id = action.id.clone();
        let command = action.command.clone();
        let before = self
            .sidebar
            .read(cx)
            .projects()
            .iter()
            .position(|project| project.id.as_ref() == id)
            .and_then(|index| index.checked_sub(1))
            .map(|index| self.sidebar.read(cx).projects()[index].id.to_string());
        let answer = (command == "delete").then(|| {
            window.prompt(
                gpui::PromptLevel::Warning,
                "Remove project?",
                Some("This removes the project from the sidebar. Files on disk are not deleted."),
                &["Cancel", "Remove"],
                cx,
            )
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                if let Some(answer) = answer {
                    if answer.await != Ok(1) {
                        return Ok(None);
                    }
                }
                match command.as_str() {
                    "edit" => {
                        let value: serde_json::Value = bridge
                            .request("project/read", json!({"projectId": id}))
                            .await?;
                        Ok(Some(value["project"].clone()))
                    }
                    "move-up" => {
                        if let Some(before) = before {
                            let _: serde_json::Value = bridge
                                .request(
                                    "project/move",
                                    json!({"projectId": id, "beforeProjectId": before}),
                                )
                                .await?;
                        }
                        Ok(None)
                    }
                    "delete" => {
                        let _: serde_json::Value = bridge
                            .request("project/delete", json!({"projectId": id}))
                            .await?;
                        Ok(None)
                    }
                    _ => anyhow::bail!("Unknown project action"),
                }
            }
            .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(Some(project)) => {
                    this.open_new_project(window, cx);
                    if let Some(dialog) = &this.new_project {
                        let host_id = this.host_id.clone();
                        dialog.update(cx, |dialog, cx| dialog.edit(&project, host_id, cx));
                    }
                }
                Ok(None) => this.load_projects(cx),
                Err(error) => this.toast(Toast::danger(error.to_string()), cx),
            })
            .ok();
        })
        .detach();
    }
}

async fn update_project(
    bridge: &bridge::Bridge,
    id: &str,
    name: &str,
    roots: &[String],
    appearance: Option<serde_json::Value>,
) -> anyhow::Result<()> {
    anyhow::ensure!(!name.trim().is_empty(), "A project needs a name");
    let mut current: serde_json::Value = bridge
        .request("project/read", json!({"projectId": id}))
        .await?;
    let metadata = host::project_metadata(
        current["project"]
            .get_mut("metadata")
            .map(serde_json::Value::take)
            .unwrap_or_else(|| json!({})),
        appearance,
    )?;
    let _: serde_json::Value = bridge
        .request(
            "project/update",
            json!({
                "projectId": id, "name": name.trim(),
                "roots": roots.iter().map(|path| json!({"path": path})).collect::<Vec<_>>(),
                "metadata": metadata,
            }),
        )
        .await?;
    Ok(())
}
