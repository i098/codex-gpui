use crate::*;
use ui::screens::settings::{environments, worktrees};

pub(super) fn init(root: WeakEntity<Root>, cx: &mut App) {
    let worktree_root = root.clone();
    cx.set_global(worktrees::State {
        dispatch: Some(Arc::new(move |action, window, cx| {
            let _ = worktree_root.update(cx, |root, cx| root.worktrees_action(action, window, cx));
        })),
        ..Default::default()
    });
    cx.set_global(environments::State {
        dispatch: Some(Arc::new(move |action, window, cx| {
            let _ = root.update(cx, |root, cx| root.environments_action(action, window, cx));
        })),
        ..Default::default()
    });
}

async fn worktree_owners(bridge: &Bridge, root: &str) -> Result<HashSet<String>> {
    let mut owners = HashSet::new();
    let mut cursor: Option<String> = None;
    let mut seen = HashSet::new();
    loop {
        let response: Value = bridge
            .request(
                "thread/attachmentOwner/list",
                json!({
                    "attachmentType": "worktree", "identityKey": root, "archived": false,
                    "cursor": cursor, "limit": 100
                }),
            )
            .await?;
        let data = response["data"]
            .as_array()
            .context("Missing attachment owners")?;
        for owner in data {
            if owner["archived"] != true {
                owners.insert(
                    owner["threadId"]
                        .as_str()
                        .context("Missing owner thread ID")?
                        .to_owned(),
                );
            }
        }
        cursor = response["nextCursor"].as_str().map(str::to_owned);
        let Some(next) = &cursor else {
            return Ok(owners);
        };
        anyhow::ensure!(
            seen.insert(next.clone()),
            "Repeated attachment owner cursor"
        );
    }
}

async fn threads(bridge: &Bridge) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen = HashSet::new();
    loop {
        let response: Value = bridge.request("thread/list", json!({"archived": false, "cursor": cursor, "limit": 100, "sortKey": "updated_at", "modelProviders": null, "sourceKinds": null, "useStateDbOnly": true})).await?;
        rows.extend(
            response["data"]
                .as_array()
                .context("Missing worktree conversations")?
                .iter()
                .cloned(),
        );
        cursor = response["nextCursor"].as_str().map(str::to_owned);
        let Some(next) = &cursor else {
            return Ok(rows);
        };
        ensure!(
            seen.insert(next.clone()),
            "App Server repeated a worktree conversation cursor"
        );
    }
}

fn in_worktree(thread: &Value, root: &str) -> bool {
    thread["cwd"]
        .as_str()
        .is_some_and(|cwd| std::path::Path::new(cwd).starts_with(root))
}

fn pinned(thread: &Value) -> bool {
    thread["section"]["id"] == PINNED_SECTION_ID
}

impl Root {
    fn worktrees_action(
        &mut self,
        action: worktrees::Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            worktrees::Action::Refresh => self.load_worktrees(cx),
            worktrees::Action::NewChat(root) => {
                self.new_chat(window, cx);
                self.select_project(root, window, cx);
            }
            worktrees::Action::OpenThread(id) => self.navigate(Route::Thread(id), window, cx),
            worktrees::Action::Delete(path) => self.delete_worktree(path, cx),
        }
    }

    fn load_worktrees(&self, cx: &mut Context<Self>) {
        cx.update_global::<worktrees::State, _>(|state, _| state.worktrees = Load::Loading);
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let read = cx.background_spawn(async move {
            let mut response = host::json_body(&host.fetch(&host::Request::post("codex-worktrees", Some(json!({"hostId": "local"}))))?)?;
            let default_root = response["defaultRoot"].as_str().unwrap_or_default().to_owned();
            let threads = threads(&bridge).await?;
            let rows = response["worktrees"].as_array_mut().context("Missing worktree list")?;
            for row in rows.iter_mut() {
                let root = row["dir"].as_str().context("Missing worktree path")?.to_owned();
                let owners = worktree_owners(&bridge, &root).await?;
                let mut conversations = threads.iter()
                    .filter(|thread| in_worktree(thread, &root) || thread["id"].as_str().is_some_and(|id| owners.contains(id)))
                    .map(|thread| json!({
                        "id": thread["id"], "title": thread["name"].as_str().map(str::trim).filter(|name| !name.is_empty()).or_else(|| thread["preview"].as_str()).unwrap_or_default(),
                        "isPinned": pinned(thread), "isSubagent": thread["source"].get("subAgent").is_some(),
                        "isAttachmentOwner": thread["id"].as_str().is_some_and(|id| owners.contains(id))
                    })).collect::<Vec<_>>();
                for id in &owners {
                    if !conversations.iter().any(|thread| thread["id"].as_str() == Some(id)) {
                        conversations.push(json!({"id": id, "title": id, "isAttachmentOwner": true}));
                    }
                }
                row["conversations"] = json!(conversations);
            }
            Ok::<_, anyhow::Error>((default_root, std::mem::take(rows)))
        });
        cx.spawn(async move |_, cx| {
            let result = read.await;
            cx.update_global::<worktrees::State, _>(|state, _| {
                state.worktrees = match result {
                    Ok((default_root, rows)) => {
                        state.default_root = default_root;
                        Load::Ready(rows.into())
                    }
                    Err(error) => Load::Failed(error.to_string()),
                };
            });
        })
        .detach();
    }

    fn delete_worktree(&self, path: String, cx: &mut Context<Self>) {
        if cx.global::<worktrees::State>().pending_delete.is_some() {
            return;
        }
        cx.update_global::<worktrees::State, _>(|state, _| {
            state.pending_delete = Some(path.clone())
        });
        let host = self.host.clone();
        let bridge = self.bridge.clone();
        let remove = cx.background_spawn(async move {
            let threads = threads(&bridge).await?;
            let owners: Vec<_> = threads.iter().filter(|thread| in_worktree(thread, &path)).collect();
            ensure!(!owners.iter().any(|thread| pinned(thread)), "Unpin the chats in this worktree before deleting it");
            for thread in owners {
                bridge.request::<_, Value>("thread/archive", json!({"threadId": thread["id"]})).await?;
            }
            host::json_body(&host.fetch(&host::Request::post("worktree-delete", Some(json!({"hostId": "local", "worktree": path, "reason": "settings-delete-targeted"}))))?)?;
            Ok::<_, anyhow::Error>(())
        });
        cx.spawn(async move |this, cx| {
            let result = remove.await;
            let _ = this.update(cx, |this, cx| {
                cx.update_global::<worktrees::State, _>(|state, _| state.pending_delete = None);
                match result {
                    Ok(()) => {
                        this.toast(Toast::info("Worktree deleted"), cx);
                        this.refresh_threads(cx);
                        this.load_worktrees(cx);
                    }
                    Err(error) => {
                        eprintln!("codex-gpui: delete worktree: {error:#}");
                        this.toast(Toast::danger(error.to_string()), cx);
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn git_setting_saved(&self, key: &str, cx: &mut Context<Self>) {
        if !matches!(key, "git-pr-watch-auto-merge" | "git-pr-watch-instructions") {
            return;
        }
        let Some(settings) = cx.try_global::<host_settings::Settings>() else {
            return;
        };
        let values = settings.read_all();
        let preferences = json!({"autoMerge": values.get("git-pr-watch-auto-merge").and_then(Value::as_bool).unwrap_or(false), "instructions": values.get("git-pr-watch-instructions").and_then(Value::as_str).unwrap_or_default()});
        let host = self.host.clone();
        let update = cx.background_spawn(async move {
            host::json_body(&host.fetch(&host::Request::post(
                "git-pr-watch-update",
                Some(preferences),
            ))?)
        });
        cx.spawn(async move |this, cx| {
            if let Err(error) = update.await {
                eprintln!("codex-gpui: update active PR watchers: {error:#}");
                let _ = this.update(cx, |this, cx| {
                    this.toast(
                        Toast::danger("Failed to update active pull request watchers"),
                        cx,
                    )
                });
            }
        })
        .detach();
    }

    fn environments_action(
        &mut self,
        action: environments::Action,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            environments::Action::AddProject => self.add_environment_project(cx),
            environments::Action::Refresh => self.load_environments(cx),
            environments::Action::Read { path } => self.read_environment(path, cx),
            environments::Action::Save {
                root,
                path,
                revision,
                definition,
            } => {
                if cx.global::<environments::State>().pending {
                    return;
                }
                cx.update_global::<environments::State, _>(|state, _| {
                    state.pending = true;
                    state.error = None;
                });
                let host = self.host.clone();
                let save_root = root;
                let path: Arc<str> = path.into();
                let expected_path = path.clone();
                let save = cx.background_spawn(async move {
                    let raw = format!(
                        "# THIS IS AUTOGENERATED. DO NOT EDIT MANUALLY\n{}",
                        toml::to_string(&toml::Value::try_from(definition)?)?
                    );
                    let mut result = host::json_body(&host.fetch(&host::Request::post(
                        "local-environment-config-save",
                        Some(json!({"configPath": path.as_ref(), "expectedRevision": revision, "raw": raw})),
                    ))?)?;
                    if revision.is_none() && result["type"] == "success" {
                        let selection = host
                            .fetch(&host::Request::post(
                                "local-environment-select",
                                Some(json!({"workspaceRoot": save_root, "configPath": path.as_ref()})),
                            ))
                            .and_then(|response| host::json_body(&response));
                        result["selectionFailed"] = json!(selection.is_err());
                    }
                    Ok::<_, anyhow::Error>(result)
                });
                cx.spawn(async move |this, cx| {
                    let result = save.await;
                    let _ = this.update(cx, |this, cx| {
                        let mut saved_path = None;
                        let mut selection_failed = false;
                        cx.update_global::<environments::State, _>(|state, _| {
                            state.pending = false;
                            let selected = state.selected_path.as_deref() == Some(expected_path.as_ref());
                            match result {
                                Ok(data) if data["type"] == "conflict" => {
                                    if selected {
                                        state.conflict = true;
                                        state.error = Some("This environment changed on disk. Discard your edits before saving again".into());
                                    }
                                }
                                Ok(data) => {
                                    saved_path = data["configPath"].as_str().map(str::to_owned);
                                    selection_failed = data["selectionFailed"] == true;
                                    if selected {
                                        state.error = selection_failed.then(|| "Saved the environment file, but could not select it".into());
                                        state.saved_generation += 1;
                                    }
                                }
                                Err(error) => if selected { state.error = Some(error.to_string()); },
                            }
                        });
                        if let Some(path) = saved_path {
                            this.toast(if selection_failed {
                                Toast::danger("Saved the environment file, but could not select it")
                            } else {
                                Toast::info("Saved local environment")
                            }, cx);
                            this.load_environments(cx);
                            if cx.global::<environments::State>().selected_path.as_deref() == Some(path.as_str()) {
                                this.read_environment(path, cx);
                            }
                        }
                    });
                }).detach();
            }
        }
    }

    fn read_environment(&self, path: String, cx: &mut Context<Self>) {
        let requested: Arc<str> = path.into();
        cx.update_global::<environments::State, _>(|state, _| {
            state.selected_path = Some(requested.clone());
            state.selected = Load::Loading;
            state.error = None;
            state.conflict = false;
        });
        let host = self.host.clone();
        let path = requested.clone();
        let read = cx.background_spawn(async move {
            host::json_body(&host.fetch(&host::Request::post(
                "local-environment-config",
                Some(json!({"configPath": path.as_ref()})),
            ))?)
        });
        cx.spawn(async move |_, cx| {
            let result = read.await;
            cx.update_global::<environments::State, _>(|state, _| {
                if state.selected_path.as_deref() == Some(requested.as_ref()) {
                    state.selected = match result {
                        Ok(data) => Load::Ready(Arc::new(data)),
                        Err(error) => Load::Failed(error.to_string()),
                    };
                }
            });
        })
        .detach();
    }

    fn add_environment_project(&mut self, cx: &mut Context<Self>) {
        if self.picking_project {
            return;
        }
        self.picking_project = true;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose project".into()),
        });
        let host = self.host.clone();
        cx.spawn(async move |this, cx| {
            let result = async {
                let Some(path) = paths.await??.and_then(|paths| paths.into_iter().next()) else {
                    return Ok(None);
                };
                cx.background_spawn(async move {
                    host.add_project(&path)?;
                    host.projects().map(Some)
                })
                .await
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                this.picking_project = false;
                match result {
                    Ok(Some(projects)) => {
                        this.set_projects(projects, cx);
                        this.load_environments(cx);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("codex-gpui: cannot choose environment project: {error:#}");
                        this.toast(Toast::danger("Cannot open project folder"), cx);
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn load_environments(&self, cx: &mut Context<Self>) {
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            let mut result = Vec::new();
            for project in host.projects()? {
                let Some(root) = project.root.as_ref() else { continue; };
                let mut response = host::json_body(&host.fetch(&host::Request::post("local-environments", Some(json!({"workspaceRoot": root.as_ref()}))))?)?;
                let rows = response["environments"].as_array_mut().context("Missing environment list")?;
                for row in rows.iter_mut() {
                    let path = std::path::Path::new(row["configPath"].as_str().context("Missing environment path")?);
                    let folder = path.parent().and_then(|p| p.parent()).and_then(|p| p.parent()).context("Invalid environment path")?;
                    let inherited = folder != std::path::Path::new(root.as_ref());
                    let file_name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    let source = format!("From {} · {file_name}", folder.file_name().unwrap_or_default().to_string_lossy());
                    row["inherited"] = json!(inherited); row["fileName"] = json!(file_name);
                    if inherited { row["source"] = json!(source); }
                }
                let directory = std::path::Path::new(root.as_ref()).join(".codex/environments");
                let mut path = directory.join("environment.toml");
                let mut suffix = 2;
                while rows.iter().any(|row| row["configPath"].as_str() == path.to_str()) { path = directory.join(format!("environment-{suffix}.toml")); suffix += 1; }
                result.push(json!({"name": project.label.as_ref(), "root": root.as_ref(), "environments": rows, "nextConfigPath": path}));
            }
            Ok::<_, anyhow::Error>(result)
        });
        cx.spawn(async move |_, cx| {
            let result = read.await;
            cx.update_global::<environments::State, _>(|state, _| {
                state.projects = match result {
                    Ok(rows) => Load::Ready(rows.into()),
                    Err(error) => Load::Failed(error.to_string()),
                };
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod owner_tests {
    use super::*;

    #[test]
    fn owners_exclude_archived_threads_and_merge_pages() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let fixture = directory.path().join("owners.jsonl");
        let pages = [
            json!({"data": [{"threadId": "one", "archived": false}, {"threadId": "archived", "archived": true}], "nextCursor": "next"}),
            json!({"data": [{"threadId": "one", "archived": false}, {"threadId": "two", "archived": false}], "nextCursor": null}),
        ];
        let mut records = String::new();
        for (id, page) in pages.into_iter().enumerate() {
            for (dir, msg) in [
                (
                    "send",
                    json!({"id": id, "method": "thread/attachmentOwner/list", "params": {
                        "attachmentType": "worktree", "identityKey": "/worktree", "archived": false,
                        "cursor": if id == 0 { Value::Null } else { json!("next") }, "limit": 100
                    }}),
                ),
                ("recv", json!({"id": id, "result": page})),
            ] {
                records.push_str(&json!({"dir": dir, "msg": msg}).to_string());
                records.push('\n');
            }
        }
        std::fs::write(&fixture, records)?;
        let (bridge, _) = Bridge::replay(&fixture)?;
        let owners = pollster::block_on(worktree_owners(&bridge, "/worktree"))?;
        assert_eq!(owners, HashSet::from(["one".to_owned(), "two".to_owned()]));
        Ok(())
    }
}
