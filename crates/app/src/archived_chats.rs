use super::*;
use gpui::Div;
use ui::screens::settings::archived_chats::{
    ArchiveAction, ArchiveData, CloudArchive, DeleteScope, OpenArchivedChats, Operation,
};
use ui::screens::sidebar::archived_threads;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &OpenArchivedChats, window, cx| {
        this.open_settings_section("Archived chats", window, cx);
    }))
    .on_action(cx.listener(|this, action: &ArchiveAction, window, cx| {
        this.archive_action(action.page.clone(), action.operation.clone(), window, cx);
    }))
}

impl Root {
    fn archive_action(
        &mut self,
        page: Entity<ArchivedSettings>,
        operation: Operation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        match operation {
            Operation::Load => {
                if self.archived_settings.as_ref() != Some(&page) {
                    cx.observe(&page, |_, _, cx| cx.notify()).detach();
                    cx.subscribe_in(
                        &page,
                        window,
                        |this, page, operation: &Operation, window, cx| {
                            this.archive_action(page.clone(), operation.clone(), window, cx);
                        },
                    )
                    .detach();
                    self.archived_settings = Some(page.clone());
                }
                let cloud_host = host.clone();
                let cloud_page = page.clone();
                let cloud = cx.background_spawn(async move { cloud_archived(&*cloud_host) });
                cx.spawn(async move |_, cx| {
                    let result = cloud.await;
                    cloud_page.update(cx, |page, cx| page.cloud_loaded(result, cx));
                })
                .detach();
                cx.spawn(async move |_, cx| {
                    let metadata = cx.background_spawn(async move { archive_metadata(&*host) });
                    let threads = archived_threads(&bridge, 100).await;
                    let (projects, metadata, metadata_error) = metadata.await;
                    let data = ArchiveData {
                        threads,
                        projects,
                        metadata,
                        metadata_error,
                    };
                    page.update(cx, |page, cx| page.loaded(data, cx));
                })
                .detach();
            }
            Operation::Restore(id) => {
                let pending = self.toaster.update(cx, |toaster, cx| {
                    toaster.push_persistent(pending_toast("Unarchiving chat"), cx)
                });
                cx.spawn_in(window, async move |this, cx| {
                    let result = bridge
                        .request::<_, ThreadReadResponse>(
                            "thread/unarchive",
                            json!({"threadId": id}),
                        )
                        .await;
                    page.update(cx, |page, cx| {
                        page.finished(std::slice::from_ref(&id), result.is_ok(), cx)
                    });
                    this.update_in(cx, |this, _, cx| {
                        let toast = match result {
                            Ok(response) => {
                                if let Some(thread) =
                                    this.threads.iter_mut().find(|thread| thread.id == id)
                                {
                                    *thread = response.thread;
                                } else {
                                    this.threads.insert(0, response.thread);
                                }
                                this.sidebar.update(cx, |sidebar, cx| {
                                    sidebar.set_threads(&this.threads, cx)
                                });
                                let root = cx.weak_entity();
                                Toast::info("Unarchived chat")
                                    .icon("sidebar-archive")
                                    .action("View", move |window, cx| {
                                        root.update(cx, |this, cx| {
                                            this.navigate(Route::Thread(id.clone()), window, cx)
                                        })
                                        .ok();
                                    })
                            }
                            Err(error) => {
                                eprintln!("thread/unarchive: {error:#}");
                                Toast::danger("Failed to unarchive chat")
                            }
                        };
                        this.toaster.update(cx, |toaster, cx| {
                            toaster.dismiss(pending, cx);
                            toaster.push(toast, cx);
                        });
                    })
                    .ok();
                })
                .detach();
            }
            Operation::RestoreCloud(id) => {
                let task_id = id.clone();
                let work = cx.background_spawn(async move {
                    let identity = host
                        .identity()
                        .context("Cloud tasks require a ChatGPT account")?;
                    let path = format!(
                        "/wham/tasks/{}/recover",
                        crate::cloud_task::segment(&task_id)
                    );
                    let response =
                        host.fetch(&crate::cloud_task::request(&identity, &path, "POST", None))?;
                    let wire = host::response_wire(&response);
                    ensure!(
                        wire["status"]
                            .as_u64()
                            .is_some_and(|status| (200..300).contains(&status)),
                        "Failed to unarchive cloud chat"
                    );
                    Ok::<_, anyhow::Error>(())
                });
                cx.spawn_in(window, async move |this, cx| {
                    let result = work.await;
                    page.update(cx, |page, cx| page.cloud_finished(&id, result.is_ok(), cx));
                    this.update(cx, |this, cx| {
                        let toast = match result {
                            Ok(()) => {
                                let root = cx.weak_entity();
                                Toast::info("Unarchived chat")
                                    .icon("sidebar-archive")
                                    .action("View", move |window, cx| {
                                        root.update(cx, |this, cx| {
                                            this.open_remote_task(id.clone(), false, window, cx)
                                        })
                                        .ok();
                                    })
                            }
                            Err(error) => {
                                eprintln!("recover cloud task: {error:#}");
                                Toast::danger("Failed to unarchive chat")
                            }
                        };
                        this.toaster
                            .update(cx, |toaster, cx| toaster.push(toast, cx));
                    })
                    .ok();
                })
                .detach();
            }
            Operation::Delete(scope) => {
                let single = matches!(scope, DeleteScope::Single(_));
                let pending = self.toaster.update(cx, |toaster, cx| {
                    toaster.push_persistent(
                        pending_toast(if single {
                            "Deleting archived chat"
                        } else {
                            "Deleting archived chats"
                        }),
                        cx,
                    )
                });
                cx.spawn(async move |this, cx| {
                    let (deleted, first_error) = delete_archived(&bridge, &scope).await;
                    page.update(cx, |page, cx| {
                        page.deletion_finished(&deleted, cx);
                    });
                    this.update(cx, |this, cx| {
                        let deleted_ids: HashSet<&str> =
                            deleted.iter().map(String::as_str).collect();
                        this.threads
                            .retain(|thread| !deleted_ids.contains(thread.id.as_str()));
                        this.sidebar
                            .update(cx, |sidebar, cx| sidebar.set_threads(&this.threads, cx));
                        let toast = deletion_toast(single, deleted.len(), first_error);
                        this.toaster.update(cx, |toaster, cx| {
                            toaster.dismiss(pending, cx);
                            toaster.push(toast, cx);
                        });
                    })
                    .ok();
                })
                .detach();
            }
        }
    }
}

/// An operation in flight: persistent until its result toast replaces it.
fn pending_toast(title: &'static str) -> Toast {
    Toast::info(title).icon("sidebar-cmd-spinner")
}

fn deletion_toast(single: bool, deleted: usize, error: Option<anyhow::Error>) -> Toast {
    if let Some(error) = error {
        let message = if error
            .downcast_ref::<bridge::RpcError>()
            .is_some_and(|error| error.code == -32601)
        {
            "This host doesn’t support deleting chats. Update the Codex CLI on the host and try again."
        } else if single {
            "Failed to delete archived chat"
        } else {
            "Failed to delete archived chats"
        };
        eprintln!("thread/delete: {error:#}");
        Toast::danger(message)
    } else if single {
        Toast::success("Deleted archived chat")
    } else {
        Toast::success(format!(
            "Deleted {deleted} archived {}",
            if deleted == 1 { "chat" } else { "chats" }
        ))
    }
}

fn archive_metadata(
    host: &dyn host::Host,
) -> (Vec<ui::screens::sidebar::Project>, Value, Option<String>) {
    let mut error = None;
    let projects = host.projects().unwrap_or_else(|failure| {
        error = Some(failure.to_string());
        Vec::new()
    });
    let mut fetch = |request| {
        host.fetch(&request)
            .and_then(|reply| host::json_body(&reply))
            .map_err(|failure| {
                error.get_or_insert_with(|| failure.to_string());
            })
            .ok()
    };
    let mut state = json!({});
    for key in [
        "projectless-thread-ids",
        "thread-project-assignments",
        "thread-workspace-root-hints",
    ] {
        if let Some(reply) = fetch(host::Request::post(
            "get-global-state",
            Some(json!({"key": key})),
        )) {
            state[key] = reply["value"].clone();
        }
    }
    if let Some(inbox) = fetch(host::Request::post(
        "inbox-items",
        Some(json!({"limit": 100000})),
    )) {
        let ids: std::collections::BTreeSet<&str> = inbox["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|run| run["threadId"].as_str())
            .collect();
        state["automationThreadIds"] = json!(ids);
    }
    (projects, state, error)
}

fn cloud_archived(host: &dyn host::Host) -> Result<Vec<CloudArchive>> {
    #[derive(Deserialize)]
    struct Page {
        items: Vec<CloudArchive>,
        cursor: Option<String>,
    }
    let Some(identity) = host.identity() else {
        return Ok(Vec::new());
    };
    let mut tasks = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen = HashSet::new();
    loop {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("limit", "20");
        if let Some(cursor) = &cursor {
            query.append_pair("cursor", cursor);
        }
        query.append_pair("task_filter", "archived");
        let path = format!("/wham/tasks/list?{}", query.finish());
        let response = host.fetch(&crate::cloud_task::request(&identity, &path, "GET", None))?;
        let status = host::response_wire(&response)["status"].as_u64();
        if matches!(status, Some(401 | 403 | 404)) {
            return Ok(tasks);
        }
        let page: Page = serde_json::from_value(host::json_body(&response)?)?;
        tasks.extend(page.items);
        cursor = page.cursor;
        let Some(next) = &cursor else {
            return Ok(tasks);
        };
        ensure!(seen.insert(next.clone()), "Cloud archive repeated a cursor");
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
                first_error.get_or_insert(error);
            }
        }
    }
    (deleted, first_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records() -> Result<Vec<Value>> {
        include_str!("../../../fixtures/sessions/archived-chats.jsonl")
            .lines()
            .map(serde_json::from_str)
            .collect::<serde_json::Result<_>>()
            .map_err(Into::into)
    }

    fn replayed<'a>(lines: impl Iterator<Item = &'a Value>) -> Result<Bridge> {
        let file = tempfile::NamedTempFile::new()?;
        let content = lines.map(Value::to_string).collect::<Vec<_>>().join("\n");
        fs::write(file.path(), content)?;
        let (bridge, _) = Bridge::replay(file.path())?;
        Ok(bridge)
    }

    fn recorded_exchanges<'a>(records: &'a [Value], method: &str) -> Vec<(usize, [&'a Value; 2])> {
        records
            .iter()
            .enumerate()
            .filter_map(|(index, request)| {
                if request["dir"] != "send" || request["msg"]["method"] != method {
                    return None;
                }
                let reply = records[index + 1..].iter().find(|reply| {
                    reply["dir"] == "recv" && reply["msg"]["id"] == request["msg"]["id"]
                })?;
                Some((index, [request, reply]))
            })
            .collect()
    }

    #[test]
    fn inbox_failure_keeps_known_archive_projects() -> Result<()> {
        let fixture = include_str!("../../../refs/interactions/f1d_projects.json");
        let host = host::ReplayHost::from_recording(fixture)?;
        let (projects, _, error) = archive_metadata(&host);
        ensure!(
            projects
                .iter()
                .any(|project| project.label == "Parity project")
        );
        ensure!(error.is_some(), "missing metadata must remain visible");
        Ok(())
    }

    #[test]
    fn deletion_refuses_a_chat_that_is_no_longer_archived() -> Result<()> {
        let records = records()?;
        let (_, list) = recorded_exchanges(&records, "thread/list")
            .into_iter()
            .last()
            .context("recorded archived list")?;
        let archived = list[1]["msg"]["result"]["data"]
            .as_array()
            .context("archived list")?;
        let (_, delete) = recorded_exchanges(&records, "thread/delete")
            .into_iter()
            .rfind(|(_, pair)| {
                !archived
                    .iter()
                    .any(|thread| thread["id"] == pair[0]["msg"]["params"]["threadId"])
            })
            .context("recorded stale delete")?;
        let id = delete[0]["msg"]["params"]["threadId"]
            .as_str()
            .context("recorded thread id")?
            .to_owned();
        let bridge = replayed(list.into_iter().chain(delete))?;
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
        let records = records()?;
        let (start, pair) = recorded_exchanges(&records, "thread/list")
            .into_iter()
            .rfind(|(_, pair)| {
                pair[1]["msg"]["result"]["data"]
                    .as_array()
                    .is_some_and(|threads| !threads.is_empty())
            })
            .context("recorded current archive")?;
        let expected: Vec<&str> = pair[1]["msg"]["result"]["data"]
            .as_array()
            .context("recorded archived chats")?
            .iter()
            .filter_map(|thread| thread["id"].as_str())
            .collect();
        let bridge = replayed(records[start..].iter())?;
        let (deleted, error) = pollster::block_on(delete_archived(&bridge, &DeleteScope::All));
        ensure!(error.is_none(), "recorded delete failed: {error:?}");
        ensure!(deleted == expected);
        Ok(())
    }

    #[test]
    fn cloud_archive_stays_empty_without_a_cloud_identity() -> Result<()> {
        let home = tempfile::tempdir()?;
        let host = host::LocalHost::new(home.path());
        ensure!(cloud_archived(&host)?.is_empty());
        Ok(())
    }

    struct CloudPages(std::sync::Mutex<std::collections::VecDeque<(u16, Value)>>);

    impl host::Host for CloudPages {
        fn identity(&self) -> Option<host::Identity> {
            Some(host::Identity {
                account_id: "account".into(),
                user_id: "user".into(),
                device_id: "device".into(),
            })
        }

        fn fetch(&self, _: &host::Request) -> Result<Value> {
            let (status, body) = self
                .0
                .lock()
                .expect("test response lock")
                .pop_front()
                .context("unexpected cloud request")?;
            Ok(json!({"status": status, "bodyJsonString": body.to_string()}))
        }
    }

    #[test]
    fn cloud_archive_merges_pages_and_rejects_repeated_cursors() -> Result<()> {
        let page =
            |id: &str, cursor: Option<&str>| json!({"items": [{"id": id}], "cursor": cursor});
        let host = CloudPages(std::sync::Mutex::new(std::collections::VecDeque::from([
            (200, page("first", Some("next"))),
            (200, page("second", None)),
        ])));
        ensure!(
            cloud_archived(&host)?
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>()
                == ["first", "second"]
        );
        let host = CloudPages(std::sync::Mutex::new(std::collections::VecDeque::from([
            (200, page("first", Some("repeat"))),
            (200, page("second", Some("repeat"))),
        ])));
        let error = cloud_archived(&host)
            .err()
            .context("repeated cursor must fail")?;
        ensure!(error.to_string() == "Cloud archive repeated a cursor");
        let host = CloudPages(std::sync::Mutex::new(std::collections::VecDeque::from([(
            404,
            Value::Null,
        )])));
        ensure!(cloud_archived(&host)?.is_empty());
        Ok(())
    }
}
