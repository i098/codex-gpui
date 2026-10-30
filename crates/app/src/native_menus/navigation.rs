use crate::*;
use ui::screens::diff::panel::{FileTreeRequest, GitBlame};

pub(crate) fn available(root: &Root, cx: &App) -> bool {
    super::active_thread(root, cx).is_some()
        && match root.side {
            Some(SidePanel::Changes) => root.diff.is_some(),
            Some(SidePanel::File) => root.file.is_some(),
            _ => false,
        }
}

pub(super) fn actions(root: gpui::Div, owner: &Root, cx: &mut Context<Root>) -> gpui::Div {
    if !available(owner, cx) {
        return root;
    }
    root.on_action(
        cx.listener(|root, _: &super::ToggleNavigationPanel, window, cx| {
            if !available(root, cx) {
                return;
            }
            match root.side {
                Some(SidePanel::Changes) => {
                    if let Some(panel) = &root.diff {
                        panel.update(cx, |panel, cx| panel.toggle_file_tree(cx));
                    }
                }
                Some(SidePanel::File) => {
                    if let Some(file) = &root.file {
                        file.update(cx, |file, cx| file.toggle_file_tree(window, cx));
                    }
                }
                _ => {}
            }
        }),
    )
}

impl Root {
    pub(crate) fn open_file(
        &mut self,
        action: &OpenFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = super::active_thread(self, cx) else {
            return;
        };
        // An absolute path inside a review repository opens with that repository as root.
        let root = self
            .diff
            .as_ref()
            .and_then(|panel| {
                panel
                    .read(cx)
                    .repositories()
                    .iter()
                    .filter(|repository| {
                        std::path::Path::new(&action.path).starts_with(&repository.root)
                    })
                    .max_by_key(|repository| repository.root.len())
                    .map(|repository| repository.root.clone())
            })
            .unwrap_or_else(|| thread.read(cx).cwd().to_owned());
        let cwd = std::path::Path::new(&root)
            .components()
            .as_path()
            .to_string_lossy()
            .into_owned();
        let path = if std::path::Path::new(&action.path).is_absolute() {
            action.path.clone()
        } else {
            format!("{cwd}/{}", action.path.trim_start_matches("./"))
        };
        let file = if let Some(file) = self
            .file
            .as_ref()
            .filter(|file| file.read(cx).workspace_root() == cwd)
        {
            file.update(cx, |file, cx| {
                file.navigate_file(path.clone(), action.line, cx)
            });
            file.clone()
        } else {
            let file = cx.new(|_| FileView::new(&cwd, path.clone(), action.line));
            cx.subscribe_in(&file, window, |root, file, request, window, cx| {
                root.navigate_file_tree(file, request, window, cx);
            })
            .detach();
            file
        };
        let revision = file.read(cx).file_revision();
        self.load_blame(&file, cx);
        let host = self.host.clone();
        let request = host::Request::post("read-file", Some(json!({ "cwd": cwd, "path": path })));
        let read = cx.background_spawn(async move {
            host.fetch(&request)
                .and_then(|response| host::json_body(&response))
                .and_then(|body| {
                    body["contents"]
                        .as_str()
                        .map(str::to_owned)
                        .context("file response has no text contents")
                })
                .map_err(|error| format!("{error:#}"))
        });
        let target = file.downgrade();
        cx.spawn(async move |root, cx| {
            let contents = read.await;
            let failed = contents.is_err();
            if target
                .update(cx, |file, cx| {
                    file.set_file_contents(revision, contents, cx)
                })
                .ok()
                != Some(true)
            {
                return;
            }
            root.update(cx, |root, cx| {
                if failed {
                    root.toast(Toast::danger("Could not open file"), cx);
                } else {
                    root.remember_file(&cwd, &path, cx);
                }
            })
            .ok();
        })
        .detach();
        self.file = Some(file);
        if self.side == Some(SidePanel::File) {
            self.sync(window, cx);
        } else {
            self.set_side(SidePanel::File, window, cx);
        }
    }

    /// `git.toggleBlame`: flips the active-line blame and loads it for the open file.
    pub(crate) fn toggle_git_blame(&mut self, cx: &mut Context<Self>) {
        cx.set_global(GitBlame(!GitBlame::enabled(cx)));
        if let Some(file) = self.file.clone() {
            self.load_blame(&file, cx);
            file.update(cx, |_, cx| cx.notify());
        }
    }

    /// The host's `git-blame-file` for the open file, while blame is on.
    fn load_blame(&self, file: &Entity<FileView>, cx: &mut Context<Self>) {
        if !GitBlame::enabled(cx) {
            return;
        }
        let file_view = file.read(cx);
        let revision = file_view.file_revision();
        let request = host::Request::post(
            "git-blame-file",
            Some(json!({ "cwd": file_view.workspace_root(), "path": file_view.path })),
        );
        let host = self.host.clone();
        let task = cx.background_spawn(async move {
            host.fetch(&request)
                .and_then(|response| host::json_body(&response))
                .map_err(|error| format!("{error:#}"))
        });
        let file = file.downgrade();
        cx.spawn(async move |_, cx| {
            let result = task.await;
            file.update(cx, |file, cx| file.set_blame(revision, result, cx))
                .ok();
        })
        .detach();
    }

    fn navigate_file_tree(
        &mut self,
        file: &Entity<FileView>,
        request: &FileTreeRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if super::active_thread(self, cx).is_none() || self.file.as_ref() != Some(file) {
            if let FileTreeRequest::Directory(directory) = request {
                file.update(cx, |file, cx| {
                    file.set_tree_directory(
                        directory,
                        Err("The workspace is no longer active".into()),
                        cx,
                    );
                });
            }
            return;
        }
        match request {
            FileTreeRequest::PinPreview => {
                let focus = window.focused(cx);
                self.sync(window, cx);
                if let Some(focus) = focus {
                    window.focus(&focus, cx);
                }
            }
            FileTreeRequest::Directory(directory) => {
                let root = file.read(cx).workspace_root();
                let directory = directory.clone();
                let host = self.host.clone();
                let request = host::Request::post(
                    "workspace-directory-entries",
                    Some(json!({
                        "hostId": "local", "workspaceRoot": root,
                        "directoryPath": if directory.is_empty() { None } else { Some(directory.as_str()) },
                        "includeHidden": true,
                    })),
                );
                let task = cx.background_spawn(async move {
                    host.fetch(&request)
                        .and_then(|response| host::json_body(&response))
                        .map_err(|error| format!("{error:#}"))
                });
                let file = file.downgrade();
                cx.spawn(async move |_, cx| {
                    let result = task.await;
                    file.update(cx, |file, cx| {
                        file.set_tree_directory(&directory, result, cx)
                    })
                    .ok();
                })
                .detach();
            }
            FileTreeRequest::Search { query, generation } => {
                let root = file.read(cx).workspace_root().to_owned();
                let query = query.clone();
                let generation = *generation;
                let bridge = self.bridge.clone();
                let file = file.downgrade();
                cx.spawn(async move |_, cx| {
                    let result = bridge
                        .request::<_, Value>(
                            "fuzzyFileSearch",
                            json!({ "query": query, "roots": [root], "cancellationToken": null }),
                        )
                        .await
                        .and_then(search_paths)
                        .map_err(|error| format!("{error:#}"));
                    file.update(cx, |file, cx| {
                        file.set_tree_search(generation, &query, result, cx)
                    })
                    .ok();
                })
                .detach();
            }
            FileTreeRequest::Open(path) => {
                let path = std::path::Path::new(path);
                if path.is_absolute()
                    || path
                        .components()
                        .any(|component| matches!(component, std::path::Component::ParentDir))
                {
                    self.toast(Toast::danger("File path is outside the workspace"), cx);
                    return;
                }
                self.open_file(
                    &OpenFile {
                        path: PathBuf::from(file.read(cx).workspace_root())
                            .join(path)
                            .to_string_lossy()
                            .into_owned(),
                        line: None,
                    },
                    window,
                    cx,
                );
                if let Some(file) = &self.file {
                    file.update(cx, |file, cx| file.focus_file_tree(window, cx));
                }
            }
        }
    }
}

fn search_paths(result: Value) -> Result<Vec<String>> {
    result["files"]
        .as_array()
        .context("file search response has no files")?
        .iter()
        .map(|file| {
            file["path"]
                .as_str()
                .map(str::to_owned)
                .context("file search result has no path")
        })
        .collect()
}
