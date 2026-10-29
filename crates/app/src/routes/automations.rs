use crate::*;
use gpui::Div;
use ui::screens::automations::history::{History, HistoryAction, Operation, Run};

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    let root = root.on_action(cx.listener(|this, action: &HistoryAction, window, cx| {
        this.history_action(action, window, cx);
    }));
    root.on_action(cx.listener(|this, _: &OpenScheduled, window, cx| {
        this.navigate(Route::Scheduled, window, cx)
    }))
    .on_action(
        cx.listener(|this, a: &automations::OpenAutomation, window, cx| {
            this.navigate(Route::Automation(a.id.clone()), window, cx)
        }),
    )
    .on_action(
        cx.listener(|this, a: &automations::RunAutomationNow, window, cx| {
            this.run_automation_now(a.id.clone(), window, cx)
        }),
    )
    .on_action(
        cx.listener(|this, action: &automations::ToggleAutomation, _, cx| {
            this.toggle_automation(&action.id, cx);
        }),
    )
    .on_action(
        cx.listener(|this, action: &automations::ShareAutomation, _, cx| {
            this.share_automation(&action.id, cx);
        }),
    )
    .on_action(cx.listener(|this, action: &automations::EditAutomation, window, cx| {
        this.edit_automation(Some(action.id.clone()), window, cx);
    }))
    .on_action(cx.listener(|this, action: &automations::CreateAutomation, window, cx| {
        match action.mode {
            automations::CreationMode::Manual => this.edit_automation(None, window, cx),
            automations::CreationMode::Codex | automations::CreationMode::ChatGPT => {
                this.new_chat(window, cx);
                let composer = this.home.read(cx).composer.clone();
                composer.update(cx, |composer, cx| composer.set_text(
                    "Let's set up a scheduled task together. First, explain how scheduled tasks work in ChatGPT. Then interview me to figure out what I need scheduled and when it should run.", cx,
                ));
            }
        }
    }))
    .on_action(cx.listener(|this, _: &automations::NewTask, window, cx| {
        this.open_dialog(DialogKind::ScheduleTask, window, cx)
    }))
    .on_action(cx.listener(|this, _: &automations::RetryAutomations, _, cx| {
        this.automations = None;
        if let Some(sidebar) = &this.scheduled {
            sidebar.update(cx, |sidebar, cx| sidebar.set_load(ListState::Loading, cx));
        }
        this.load_automations(cx);
    }))
    .on_action(cx.listener(|this, a: &automations::OpenAutomationRun, window, cx| {
        this.navigate_with_sidebar(
            Route::Thread(a.thread_id.clone()),
            Some(a.automation_id.clone()),
            window,
            cx,
        );
    }))
    .on_action(
        cx.listener(|this, action: &automations::StartSuggestedTask, window, cx| {
            this.start_suggested_task(action.0, window, cx);
        }),
    )
}

/// Hands the list's load state, run history, search and filters to a newly built sidebar.
fn fill_sidebar(
    root: &mut Root,
    sidebar: Entity<automations::ScheduledSidebar>,
    cx: &mut Context<Root>,
) {
    let (state, runs, run) = (
        root.automation_list_state(),
        root.automation_runs.clone(),
        root.sidebar_run(),
    );
    let previous = root.scheduled.clone();
    sidebar.update(cx, |sidebar, cx| {
        sidebar.set_load(state, cx);
        sidebar.set_runs(runs, run, cx);
        if let Some(previous) = &previous {
            sidebar.keep_view(previous, cx);
        }
    });
    root.scheduled = Some(sidebar);
    root.sync_starting(cx);
}

pub(super) fn scheduled(root: &mut Root, cx: &mut Context<Root>) -> Entity<Shell> {
    let tasks = root.tasks();
    root.load_automations(cx);
    let (shell, sidebar, page) = automations::shell(tasks, None, utc_offset(), cx);
    fill_sidebar(root, sidebar, cx);
    root.suggestions = page;
    shell
}

pub(super) fn automation(
    root: &mut Root,
    id: String,
    window: &mut Window,
    cx: &mut Context<Root>,
) -> Entity<Shell> {
    // The list is the one the Scheduled page read; a cold start reads it now.
    if root.automations.is_none() && root.automation_load.is_none() {
        root.load_automations(cx);
    }
    let tasks = root.tasks();
    let selected = tasks.iter().find(|t| t.id == id).cloned().map(|task| {
        let composer: AnyView = cx.new(|cx| Composer::automation(window, cx)).into();
        (task, composer)
    });
    let (shell, sidebar, page) = automations::shell(tasks, selected, utc_offset(), cx);
    fill_sidebar(root, sidebar, cx);
    if let Some(history) = root
        .scheduled
        .as_ref()
        .and_then(|sidebar| sidebar.read(cx).history.clone())
    {
        root.load_history(history, cx);
    }
    root.suggestions = page;
    shell
}

pub(super) fn thread(root: &mut Root, cx: &mut Context<Root>) -> Entity<Shell> {
    let tasks = root.tasks();
    let task = tasks
        .iter()
        .find(|t| Some(&t.id) == root.sidebar_automation.as_ref())
        .cloned();
    let selected = task.map(|task| (task, cx.new(|_| ui::EmptyMain).into()));
    let (shell, sidebar, _) = automations::shell(tasks, selected, utc_offset(), cx);
    fill_sidebar(root, sidebar, cx);
    shell
}

impl Root {
    pub(crate) fn scheduled_thread(&self) -> bool {
        self.sidebar_automation.is_some()
            || matches!(&self.state.route, Route::Thread(id) if self.suggested_threads.contains(id))
    }

    pub(crate) fn start_suggested_task(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self.suggestions.clone() else {
            return;
        };
        let Some(mut suggestion) = page.update(cx, |page, cx| page.begin_start(index, cx)) else {
            return;
        };
        suggestion["cwd"] = json!(self.state.project_root.as_ref().unwrap_or(&self.cwd));
        let source = self.host.clone();
        let host = source.clone();
        let epoch = self.navigation_epoch;
        let start = cx.background_spawn(async move {
            let reply = host.fetch(&host::Request::post(
                "start-suggested-task",
                Some(suggestion),
            ))?;
            let mut body = host::json_body(&reply)?;
            serde_json::from_value::<Thread>(body["thread"].take()).map_err(anyhow::Error::from)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = start.await;
            this.update_in(cx, |this, window, cx| {
                page.update(cx, |page, cx| page.finish_start(cx));
                if !Arc::ptr_eq(&this.host, &source) || this.navigation_epoch != epoch {
                    return;
                }
                match result {
                    Ok(thread) => {
                        this.suggested_threads.insert(thread.id.clone());
                        this.automation_threads.insert(thread.id.clone());
                        this.loaded.insert(thread.id.clone());
                        this.add_thread(&thread, cx);
                        this.listen_to_app_tools(window, cx);
                        this.navigate(Route::Thread(thread.id), window, cx);
                    }
                    Err(error) => {
                        eprintln!("codex-gpui: suggested task start failed: {error:#}");
                        this.toast(Toast::danger("Couldn’t start this suggested task"), cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn load_history(&self, page: Entity<History>, cx: &mut Context<Self>) {
        if self.automation_source == "cloud" {
            page.update(cx, |page, cx| {
                page.loaded(
                    Err(anyhow::anyhow!(
                        "Run history is only available for local tasks"
                    )),
                    cx,
                )
            });
            return;
        }
        page.update(cx, |page, cx| {
            page.busy = true;
            cx.notify();
        });
        let host = self.host.clone();
        let source = host.clone();
        let automation = page.read(cx).automation_id.clone();
        let read = cx.background_spawn(async move { read_history(&*host, &automation) });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            this.update(cx, |this, cx| {
                if Arc::ptr_eq(&this.host, &source) && !this.signed_out() {
                    page.update(cx, |page, cx| page.loaded(result, cx));
                } else {
                    page.update(cx, |page, cx| {
                        page.busy = false;
                        cx.notify();
                    });
                }
            })
            .ok();
        })
        .detach();
    }

    fn history_action(
        &mut self,
        action: &HistoryAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let HistoryAction(page, operation) = action;
        if self.signed_out() || self.automation_source == "cloud" || page.read(cx).busy {
            return;
        }
        match operation {
            Operation::Load => self.load_history(page.clone(), cx),
            Operation::View(id) => self.navigate(Route::Thread(id.clone()), window, cx),
            Operation::MarkRead => self.mark_history_read(page, cx),
            Operation::Archive => self.confirm_history_archive(page, window, cx),
        }
    }

    fn mark_history_read(&self, page: &Entity<History>, cx: &mut Context<Self>) {
        page.update(cx, |page, cx| {
            page.busy = true;
            cx.notify();
        });
        let automation = page.read(cx).automation_id.clone();
        let page = page.clone();
        let host = self.host.clone();
        let source = host.clone();
        let write = cx.background_spawn(async move {
            host.fetch(&host::Request::post(
                "inbox-automation-runs-mark-all-read",
                Some(json!({"automationId": automation})),
            ))
            .and_then(|reply| host::json_body(&reply))
        });
        cx.spawn(async move |this, cx| {
            let result = write.await;
            this.update(cx, |this, cx| {
                if !Arc::ptr_eq(&this.host, &source) || this.signed_out() {
                    page.update(cx, |page, cx| {
                        page.busy = false;
                        cx.notify();
                    });
                    return;
                }
                if let Err(error) = result {
                    this.toast(
                        Toast::danger(format!("Could not mark runs as read: {error}")),
                        cx,
                    );
                }
                this.load_history(page, cx);
                this.refresh_automation_unreads(cx);
            })
            .ok();
        })
        .detach();
    }

    fn confirm_history_archive(
        &self,
        page: &Entity<History>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids: HashSet<String> = page
            .read(cx)
            .runs
            .iter()
            .filter(|run| run.actionable())
            .map(|run| run.thread_id.clone())
            .collect();
        if ids.is_empty() {
            return;
        }
        let count = ids.len();
        let title = if count == 1 {
            "Archive this run?".to_owned()
        } else {
            format!("Archive {count} runs?")
        };
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &title,
            Some("You can find these runs’ chats later in archived chats"),
            &["Cancel", if count == 1 { "Archive" } else { "Archive all" }],
            cx,
        );
        let page = page.clone();
        let source = self.host.clone();
        cx.spawn_in(window, async move |this, cx| {
            if answer.await.ok() != Some(1) {
                return;
            }
            let setup = this
                .update(cx, |this, cx| {
                    if !Arc::ptr_eq(&this.host, &source) || this.signed_out() || page.read(cx).busy
                    {
                        return None;
                    }
                    page.update(cx, |page, cx| {
                        page.busy = true;
                        cx.notify();
                    });
                    Some((
                        this.bridge.clone(),
                        this.host.clone(),
                        page.read(cx).automation_id.clone(),
                    ))
                })
                .ok()
                .flatten();
            let Some((bridge, host, automation)) = setup else {
                return;
            };
            let result = cx
                .background_spawn(async move {
                    archive_history(&bridge, &*host, &automation, &ids).await
                })
                .await;
            this.update(cx, |this, cx| {
                if !Arc::ptr_eq(&this.host, &source) || this.signed_out() {
                    page.update(cx, |page, cx| {
                        page.busy = false;
                        cx.notify();
                    });
                    return;
                }
                this.toast(archive_toast(result), cx);
                this.load_history(page, cx);
                this.refresh_threads(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Builds a task URL with the selected storage source and a query-encoded ID.
    pub fn automation_link(id: &str, source: &str) -> String {
        let mut link = url::Url::parse("https://chatgpt.com/scheduled").expect("constant URL");
        link.query_pairs_mut()
            .append_pair("automationId", id)
            .append_pair("automationSource", source);
        link.into()
    }

    fn share_automation(&mut self, id: &str, cx: &mut Context<Self>) {
        let link = Self::automation_link(id, self.automation_source);
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(link.clone()));
        // The original confirms only a successful clipboard write.
        if cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref()
            == Some(&link)
        {
            self.toast(Toast::success("Link copied"), cx);
        } else {
            eprintln!("codex-gpui: share task: the clipboard did not take the link");
        }
    }
}

fn read_history(host: &dyn host::Host, automation: &str) -> Result<Value> {
    let reply = host.fetch(&host::Request::post(
        "inbox-items",
        Some(json!({"automationId": automation, "limit": i64::MAX})),
    ))?;
    host::json_body(&reply)
}

fn archive_toast(result: Result<(usize, usize)>) -> Toast {
    match result {
        Ok((0, 0)) => Toast::info("No completed runs to archive"),
        Ok((1, 0)) => Toast::success("Archived 1 run"),
        Ok((succeeded, 0)) => Toast::success(format!("Archived {succeeded} runs")),
        Ok((succeeded, failed)) => {
            Toast::danger(format!("Archived {succeeded}; could not archive {failed}"))
        }
        Err(error) => Toast::danger(format!("Could not archive runs: {error}")),
    }
}

async fn archive_history(
    bridge: &Bridge,
    host: &dyn host::Host,
    automation: &str,
    confirmed: &HashSet<String>,
) -> Result<(usize, usize)> {
    let history = read_history(host, automation)?;
    let runs: Vec<Run> = serde_json::from_value(history["items"].clone())?;
    let mut succeeded = 0;
    let mut failed = 0;
    for run in runs
        .iter()
        .filter(|run| run.actionable() && confirmed.contains(&run.thread_id))
    {
        let result = async {
            let response = bridge
                .request::<_, ThreadReadResponse>(
                    "thread/read",
                    json!({"threadId": run.thread_id, "includeTurns": false}),
                )
                .await?;
            if response.thread.status == ThreadStatus::Active {
                return Ok(false);
            }
            bridge
                .request::<_, Value>("thread/archive", json!({"threadId": run.thread_id}))
                .await?;
            let reply = host.fetch(&host::Request::post(
                "automation-run-archive",
                Some(json!({"automationId": automation, "threadId": run.thread_id})),
            ))?;
            host::json_body(&reply)?;
            Ok::<_, anyhow::Error>(true)
        }
        .await;
        match result {
            Ok(true) => succeeded += 1,
            Ok(false) => {}
            Err(error) => {
                eprintln!(
                    "codex-gpui: could not archive run {}: {error:#}",
                    run.thread_id
                );
                failed += 1;
            }
        }
    }
    Ok((succeeded, failed))
}
