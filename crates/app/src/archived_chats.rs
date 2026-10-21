use super::*;
use gpui::Div;
use ui::screens::settings::archived_chats::{
    ArchiveAction, DeleteScope, OpenArchivedChats, Operation,
};
use ui::screens::sidebar::archived_threads;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &OpenArchivedChats, window, cx| {
        this.open_settings_section("Archived chats", window, cx);
    }))
    .on_action(cx.listener(|this, action: &ArchiveAction, window, cx| {
        this.archive_action(action.clone(), window, cx);
    }))
}

impl Root {
    fn archive_action(
        &mut self,
        action: ArchiveAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        match action.operation {
            Operation::Load => {
                self.archived_settings = Some(action.page.clone());
                cx.observe(&action.page, |_, _, cx| cx.notify()).detach();
                cx.spawn(async move |_, cx| {
                    let metadata = cx.background_spawn(async move {
                        let mut state = json!({});
                        for key in [
                            "projectless-thread-ids",
                            "thread-project-assignments",
                            "thread-workspace-root-hints",
                        ] {
                            let reply = host::json_body(&host.fetch(&host::Request::post(
                                "get-global-state",
                                Some(json!({"key": key})),
                            ))?)?;
                            state[key] = reply["value"].clone();
                        }
                        let inbox = host::json_body(&host.fetch(&host::Request::post(
                            "inbox-items",
                            Some(json!({"limit": 100000})),
                        ))?)?;
                        state["automationThreadIds"] = Value::Array(
                            inbox["items"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|run| run.get("threadId").cloned())
                                .collect(),
                        );
                        Ok::<_, anyhow::Error>((host.projects()?, state))
                    });
                    let threads = archived_threads(&bridge, 100).await;
                    let metadata = metadata.await;
                    let result = threads.and_then(|threads| {
                        metadata.map(|(projects, metadata)| (threads, projects, metadata))
                    });
                    action.page.update(cx, |page, cx| page.loaded(result, cx));
                })
                .detach();
            }
            Operation::Restore(id) => {
                let key = format!("unarchive-thread-{id}");
                self.toaster.update(cx, |toaster, cx| {
                    toaster.push_keyed(key.clone(), Toast::pending("Unarchiving chat"), cx)
                });
                cx.spawn_in(window, async move |this, cx| {
                    let result = bridge
                        .request::<_, ThreadReadResponse>(
                            "thread/unarchive",
                            json!({"threadId": id}),
                        )
                        .await;
                    action.page.update(cx, |page, cx| {
                        page.finished(std::slice::from_ref(&id), result.is_ok(), cx)
                    });
                    this.update_in(cx, |this, _, cx| match result {
                        Ok(response) => {
                            this.threads.retain(|thread| thread.id != id);
                            this.threads.insert(0, response.thread);
                            this.sidebar
                                .update(cx, |sidebar, cx| sidebar.set_threads(&this.threads, cx));
                            let root = cx.weak_entity();
                            let toast = Toast::info("Unarchived chat")
                                .icon("sidebar-archive")
                                .action("View", move |window, cx| {
                                    root.update(cx, |this, cx| {
                                        this.navigate(Route::Thread(id.clone()), window, cx)
                                    })
                                    .ok();
                                });
                            this.toaster
                                .update(cx, |toaster, cx| toaster.push_keyed(key, toast, cx));
                        }
                        Err(error) => {
                            eprintln!("thread/unarchive: {error:#}");
                            this.toaster.update(cx, |toaster, cx| {
                                toaster.push_keyed(
                                    key,
                                    Toast::danger("Failed to unarchive chat"),
                                    cx,
                                )
                            });
                        }
                    })
                    .ok();
                })
                .detach();
            }
            Operation::Delete(scope) => {
                let single = matches!(scope, DeleteScope::Single(_));
                let key = "delete-archived-chats";
                self.toaster.update(cx, |toaster, cx| {
                    toaster.push_keyed(
                        key,
                        Toast::pending(if single {
                            "Deleting archived chat"
                        } else {
                            "Deleting archived chats"
                        }),
                        cx,
                    )
                });
                cx.spawn(async move |this, cx| {
                    let (deleted, first_error) = delete_archived(&bridge, &scope).await;
                    action.page.update(cx, |page, cx| {
                        page.deletion_finished(&deleted, cx);
                    });
                    this.update(cx, |this, cx| {
                        this.threads.retain(|thread| !deleted.contains(&thread.id));
                        this.sidebar.update(cx, |sidebar, cx| sidebar.set_threads(&this.threads, cx));
                        let toast = if let Some(error) = first_error {
                            let message = if error.downcast_ref::<bridge::RpcError>().is_some_and(|error| error.code == -32601) {
                                "This host doesn’t support deleting chats. Update the Codex CLI on the host and try again."
                            } else if single { "Failed to delete archived chat" } else { "Failed to delete archived chats" };
                            eprintln!("thread/delete: {error:#}");
                            Toast::danger(message)
                        } else if single {
                            Toast::success("Deleted archived chat")
                        } else {
                            Toast::success(format!("Deleted {} archived {}", deleted.len(), if deleted.len() == 1 { "chat" } else { "chats" }))
                        };
                        this.toaster.update(cx, |toaster, cx| toaster.push_keyed(key, toast, cx));
                    }).ok();
                }).detach();
            }
        }
    }
}

async fn delete_archived(
    bridge: &Bridge,
    scope: &DeleteScope,
) -> (Vec<String>, Option<anyhow::Error>) {
    let archived: Vec<String> = match archived_threads(bridge, 100).await {
        Ok(threads) => threads.into_iter().map(|thread| thread.id).collect(),
        Err(error) => return (Vec::new(), Some(error)),
    };
    let ids: &[String] = match scope {
        DeleteScope::Single(id) => std::slice::from_ref(id),
        DeleteScope::Project(ids) => ids,
        DeleteScope::All => &archived,
    };
    let mut available: HashSet<&str> = archived.iter().map(String::as_str).collect();
    let mut deleted = Vec::new();
    let mut first_error = None;
    for id in ids {
        let result = if available.remove(id.as_str()) {
            bridge
                .request::<_, Value>("thread/delete", json!({"threadId": id}))
                .await
        } else {
            Err(anyhow::anyhow!("Chat is no longer archived"))
        };
        match result {
            Ok(_) => deleted.push(id.clone()),
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    (deleted, first_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_refuses_a_chat_that_is_no_longer_archived() -> Result<()> {
        let records: Vec<Value> = include_str!("../../../fixtures/sessions/archived-chats.jsonl")
            .lines()
            .map(serde_json::from_str)
            .collect::<serde_json::Result<_>>()?;
        let pair = |method: &str| {
            records
                .windows(2)
                .filter(|pair| {
                    pair[0]["dir"] == "send"
                        && pair[0]["msg"]["method"] == method
                        && pair[1]["dir"] == "recv"
                })
                .next_back()
                .expect("recorded transaction")
        };
        let list = pair("thread/list");
        let archived = list[1]["msg"]["result"]["data"]
            .as_array()
            .context("archived list")?;
        let delete = records
            .windows(2)
            .filter(|pair| {
                pair[0]["msg"]["method"] == "thread/delete"
                    && pair[1]["dir"] == "recv"
                    && !archived
                        .iter()
                        .any(|thread| thread["id"] == pair[0]["msg"]["params"]["threadId"])
            })
            .next_back()
            .context("recorded stale delete")?;
        let id = delete[0]["msg"]["params"]["threadId"]
            .as_str()
            .context("recorded thread id")?
            .to_owned();
        let file = tempfile::NamedTempFile::new()?;
        let content = list
            .iter()
            .chain(delete.iter())
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(file.path(), content)?;
        let (bridge, _) = Bridge::replay(file.path())?;
        let (deleted, error) =
            pollster::block_on(delete_archived(&bridge, &DeleteScope::Single(id)));
        ensure!(deleted.is_empty());
        ensure!(
            error.context("stale archive must fail")?.to_string() == "Chat is no longer archived"
        );
        Ok(())
    }

    #[test]
    fn delete_all_uses_the_current_archived_inventory() -> Result<()> {
        let records: Vec<Value> = include_str!("../../../fixtures/sessions/archived-chats.jsonl")
            .lines()
            .map(serde_json::from_str)
            .collect::<serde_json::Result<_>>()?;
        let start = records
            .iter()
            .rposition(|line| line["msg"]["method"] == "thread/list")
            .context("recorded current archive")?;
        let file = tempfile::NamedTempFile::new()?;
        fs::write(
            file.path(),
            records[start..]
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )?;
        let (bridge, _) = Bridge::replay(file.path())?;
        let (deleted, error) = pollster::block_on(delete_archived(&bridge, &DeleteScope::All));
        ensure!(error.is_none(), "recorded delete failed: {error:?}");
        ensure!(deleted == ["01a13de9-35ed-7c72-a36a-68eeb2f4e1ee"]);
        Ok(())
    }
}
