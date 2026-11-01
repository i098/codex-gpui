//! `File > New Project…`: the native create-project form over the host's project store, with
//! a remote folder picker over the device's app-server (`remoteWorkspaceDirectoryEntries`).

use crate::Root;
use crate::host;
use anyhow::Context as _;
use gpui::{AppContext as _, Context, Entity, Focusable as _, PathPromptOptions, Window};
use serde_json::{Value, json};
use std::path::PathBuf;
use ui::screens::{
    connections::{ConnectionsState, Tab},
    new_project::{Browser, Dialog, Outcome},
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
        let dialog = cx.new(|cx| Dialog::new(remotes, cx).on_host(self.host_id.clone()));
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
                Outcome::Browse { host_id, path } => {
                    this.browse_remote(dialog.clone(), host_id, path.clone(), window, cx)
                }
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

    #[allow(clippy::too_many_arguments)]
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
        let appearance_changed = dialog.read(cx).appearance_changed();
        let threads = self
            .threads
            .iter()
            .map(|thread| (thread.id.clone(), thread.cwd.clone()))
            .collect::<Vec<_>>();
        let bridge = self.project_bridge(&host_id, cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let bridge = bridge
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Project host is disconnected"))?;
                let root = if let Some(project_id) = project_id {
                    update_project(
                        bridge,
                        &project_id,
                        &name,
                        &sources,
                        appearance,
                        appearance_changed,
                    )
                    .await?;
                    sources.first().cloned().unwrap_or_default()
                } else if host_id == "local" {
                    cx.background_spawn(async move {
                        let roots: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
                        host.create_project(&name, &roots, appearance, &threads)
                    })
                    .await?
                } else {
                    host::create_server_project(bridge, &name, &sources, appearance).await?
                };
                Ok::<_, anyhow::Error>(root)
            }
            .await;
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
                    this.toast(
                        Toast::danger(format!("Could not save project: {error}")),
                        cx,
                    );
                }
            })
            .ok();
        })
        .detach();
    }

    fn project_bridge(
        &self,
        host_id: &str,
        cx: &Context<Self>,
    ) -> Option<std::sync::Arc<bridge::Bridge>> {
        if host_id == self.host_id {
            Some(self.bridge.clone())
        } else {
            crate::connections::bridge(host_id, cx)
        }
    }

    /// Lists the subfolders of `path` on `host_id`, or of the folder that holds the host's Codex
    /// home when `path` is `None` (`getDefaultWorkspaceDirectoryPath`).
    fn browse_remote(
        &mut self,
        dialog: Entity<Dialog>,
        host_id: &str,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bridge = self.project_bridge(host_id, cx);
        cx.spawn_in(window, async move |_, cx| {
            let listing = async {
                let bridge = bridge.context("Project host is disconnected")?;
                let path = match path {
                    Some(path) => path,
                    None => {
                        let home = bridge
                            .initialize("codex-gpui", env!("CARGO_PKG_VERSION"))
                            .await?
                            .codex_home;
                        parent(&home).unwrap_or(home)
                    }
                };
                let response: Value = bridge
                    .request("fs/readDirectory", json!({"path": path}))
                    .await?;
                Ok::<_, anyhow::Error>(listing(path, &response))
            }
            .await;
            dialog.update(cx, |dialog, cx| {
                dialog.set_listing(listing.map_err(|error| format!("{error:#}")), cx)
            });
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
                if let Some(answer) = answer
                    && answer.await != Ok(1)
                {
                    return Ok(None);
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
    appearance_changed: bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(!name.trim().is_empty(), "A project needs a name");
    let mut current: serde_json::Value = bridge
        .request("project/read", json!({"projectId": id}))
        .await?;
    let mut metadata = current["project"]
        .get_mut("metadata")
        .map(serde_json::Value::take)
        .filter(|value| !value.is_null())
        .unwrap_or_else(|| json!({}));
    if appearance_changed {
        metadata = host::project_metadata(metadata, appearance)?;
    }
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

// shortcut: POSIX remote paths only (SSH hosts); upgrade when Windows remotes connect.
fn parent(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/')? {
        0 => Some("/".to_owned()),
        index => Some(trimmed[..index].to_owned()),
    }
}

/// `fs/readDirectory` `entries` as the picker's folders: directories only, dot folders last,
/// each group by name (`remoteWorkspaceDirectoryEntries`).
fn listing(path: String, response: &Value) -> Browser {
    let mut folders: Vec<String> = response["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["isDirectory"] == true)
        .filter_map(|entry| entry["fileName"].as_str().map(str::to_owned))
        .collect();
    folders.sort_by(|a, b| (a.starts_with('.'), a).cmp(&(b.starts_with('.'), b)));
    Browser {
        parent: parent(&path),
        path,
        folders,
        error: None,
        loading: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_listing_keeps_folders_with_dot_folders_last() {
        let response = json!({"entries": [
            {"fileName": "src", "isDirectory": true, "isFile": false},
            {"fileName": "README.md", "isDirectory": false, "isFile": true},
            {"fileName": ".git", "isDirectory": true, "isFile": false},
            {"fileName": "app", "isDirectory": true, "isFile": false},
        ]});
        let browser = listing("/home/me/repo/".into(), &response);
        assert_eq!(browser.folders, ["app", "src", ".git"]);
        assert_eq!(browser.parent.as_deref(), Some("/home/me"));
        assert_eq!(parent("/home"), Some("/".into()));
        assert_eq!(parent("/"), None);
    }
}
