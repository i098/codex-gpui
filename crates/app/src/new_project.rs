//! `File > New Project...`: the native create-project form over the host's project store, and
//! remote projects over a connected host's app-server (`remoteWorkspaceDirectoryEntries`).

use crate::Root;
use anyhow::Context as _;
use bridge::types::ThreadListResponse;
use gpui::{
    AppContext as _, Context, Entity, Focusable as _, PathPromptOptions, SharedString, Window,
};
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
                    connection.host_id.clone().into(),
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
                Outcome::Browse { host_id, path } => {
                    browse_remote(dialog.clone(), host_id.clone(), path.clone(), window, cx)
                }
                Outcome::Create {
                    name,
                    sources,
                    appearance,
                    host_id: Some(host_id),
                } => this.create_remote_project(
                    dialog.clone(),
                    host_id.to_string(),
                    name.clone(),
                    sources.first().cloned().unwrap_or_default(),
                    appearance.clone(),
                    window,
                    cx,
                ),
                Outcome::Create {
                    name,
                    sources,
                    appearance,
                    host_id: None,
                } => this.create_project(
                    dialog.clone(),
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

    /// `projects.createRemote`: saves the remote folder as a project, then opens it in a window
    /// on that host.
    #[allow(clippy::too_many_arguments)]
    fn create_remote_project(
        &mut self,
        dialog: Entity<Dialog>,
        host_id: String,
        name: String,
        path: String,
        appearance: Option<Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.host.clone();
        cx.spawn_in(window, async move |this, cx| {
            let saved = {
                let host_id = host_id.clone();
                let path = path.clone();
                cx.background_spawn(async move {
                    // `label: name.trim() || basename(remotePath)`.
                    let label = if name.is_empty() {
                        path.rsplit('/')
                            .find(|part| !part.is_empty())
                            .unwrap_or(&path)
                    } else {
                        name.as_str()
                    };
                    host.add_remote_project(&host_id, label, &path, appearance)
                })
                .await
            };
            this.update_in(cx, |this, window, cx| match saved {
                Ok(()) => {
                    if this.new_project.as_ref() == Some(&dialog) {
                        this.close_new_project(window, cx);
                    }
                    this.finish_link_host(host_id, Some(path), window, cx);
                }
                Err(error) => {
                    eprintln!("codex-gpui: cannot create remote project: {error:#}");
                    dialog.update(cx, |dialog, cx| dialog.finish(cx));
                    this.toast(Toast::danger("Failed to create project"), cx);
                }
            })
            .ok();
        })
        .detach();
    }
}

/// Lists the subfolders of `path` on the connected `host_id`, or of the folder that holds the
/// host's Codex home when `path` is `None` (`getDefaultWorkspaceDirectoryPath`).
fn browse_remote(
    dialog: Entity<Dialog>,
    host_id: SharedString,
    path: Option<String>,
    window: &mut Window,
    cx: &mut Context<Root>,
) {
    let bridge = crate::connections::bridge(&host_id, cx);
    cx.spawn_in(window, async move |_, cx| {
        let listing = async {
            let bridge = bridge.context("Remote host is not connected")?;
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
    folders.sort_by(|a, b| (a.starts_with('.'), a.as_str()).cmp(&(b.starts_with('.'), b.as_str())));
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
