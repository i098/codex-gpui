use crate::*;
use anyhow::Context as _;
use gpui::InteractiveElement as _;

pub(crate) fn actions(root: gpui::Div, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(
        cx.listener(|this, action: &automations::MarkAllRunsRead, _, cx| {
            this.mark_runs_read(&action.id, cx);
        }),
    )
    .on_action(
        cx.listener(|this, action: &automations::ArchiveAllRuns, window, cx| {
            let Some(history) = this.run_history(cx).filter(|h| h.read(cx).id == action.id) else {
                return;
            };
            if this.automation_history_updates.contains(&action.id) {
                return;
            }
            let threads: Vec<String> = history
                .read(cx)
                .items()
                .iter()
                .filter(|run| automations::archiveable(run))
                .filter_map(|run| run["threadId"].as_str().map(str::to_owned))
                .collect();
            if threads.is_empty() {
                return;
            }
            let dialog = cx.new(|cx| Dialog::archive_runs(action.id.clone(), threads, cx));
            this.show_dialog(dialog, window, cx);
        }),
    )
}

impl Root {
    pub fn run_history(&self, cx: &App) -> Option<Entity<automations::RunHistory>> {
        self.scheduled.as_ref()?.read(cx).history.clone()
    }

    pub(crate) fn load_run_history(&mut self, cx: &mut Context<Self>) {
        let Some(history) = self.run_history(cx) else {
            return;
        };
        let id = history.read(cx).id.clone();
        let host = self.host.clone();
        let read = cx.background_spawn(async move {
            let response = host.fetch(&host::Request::post(
                "inbox-items",
                Some(json!({"limit": 200})),
            ))?;
            let mut body = host::json_body(&response)?;
            let items = body["items"]
                .as_array_mut()
                .context("missing inbox items")?;
            Ok::<_, anyhow::Error>(
                std::mem::take(items)
                    .into_iter()
                    .filter(|run| run["automationId"] == id)
                    .collect(),
            )
        });
        cx.spawn(async move |_, cx| {
            let result = read.await.map_err(|error| error.to_string());
            history.update(cx, |history, cx| history.set_items(result, cx));
        })
        .detach();
    }

    fn mark_runs_read(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(history) = self.run_history(cx).filter(|h| h.read(cx).id == id) else {
            return;
        };
        if !self.automation_history_updates.insert(id.to_owned()) {
            return;
        }
        let runs: Vec<String> = history
            .read(cx)
            .items()
            .iter()
            .filter(|run| run["readAt"].is_null())
            .filter_map(|run| run["id"].as_str().map(str::to_owned))
            .collect();
        history.update(cx, |history, cx| history.set_busy(true, cx));
        let host = self.host.clone();
        let id = id.to_owned();
        let write = cx.background_spawn(async move {
            let mut failures = 0;
            for id in runs {
                let result = host
                    .fetch(&host::Request::post(
                        "inbox-item-set-read-state",
                        Some(json!({"id":id,"isRead":true})),
                    ))
                    .and_then(|response| host::json_body(&response));
                if !matches!(result, Ok(body) if body["success"] == true) {
                    failures += 1;
                }
            }
            failures
        });
        cx.spawn(async move |this, cx| {
            let failures = write.await;
            this.update(cx, |this, cx| {
                this.automation_history_updates.remove(&id);
                history.update(cx, |history, cx| history.set_busy(false, cx));
                if failures > 0 {
                    this.toast(Toast::danger("Could not mark all runs as read"), cx);
                }
                this.load_run_history(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn archive_runs(
        &mut self,
        id: String,
        threads: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if !self.automation_history_updates.insert(id.clone()) {
            return;
        }
        let history = self.run_history(cx).filter(|h| h.read(cx).id == id);
        if let Some(history) = &history {
            history.update(cx, |h, cx| h.set_busy(true, cx));
        }
        let bridge = self.bridge.clone();
        let host = self.host.clone();
        cx.spawn(async move |this, cx| {
            let mut archived = Vec::new();
            let mut failures = 0;
            let mut succeeded = 0;
            for thread in threads {
                let result = bridge
                    .request::<_, Value>("thread/archive", json!({"threadId":thread}))
                    .await;
                if result.is_err() {
                    failures += 1;
                    continue;
                }
                archived.push(thread.clone());
                let request =
                    host::Request::post("automation-run-archive", Some(json!({"threadId":thread})));
                let host = host.clone();
                let stored = cx
                    .background_spawn(async move {
                        host.fetch(&request)
                            .and_then(|response| host::json_body(&response))
                    })
                    .await;
                if matches!(stored, Ok(body) if body["success"] == true) {
                    succeeded += 1;
                } else {
                    failures += 1;
                    if bridge
                        .request::<_, Value>("thread/unarchive", json!({"threadId":thread}))
                        .await
                        .is_ok()
                    {
                        archived.pop();
                    }
                }
            }
            this.update(cx, |this, cx| {
                this.automation_history_updates.remove(&id);
                if let Some(history) = history {
                    history.update(cx, |h, cx| h.set_busy(false, cx));
                }
                this.threads.retain(|thread| !archived.contains(&thread.id));
                this.sidebar
                    .update(cx, |sidebar, cx| sidebar.set_threads(&this.threads, cx));
                if failures == 0 {
                    let count = archived.len();
                    this.toast(
                        Toast::success(format!(
                            "Archived {count} {}",
                            if count == 1 { "run" } else { "runs" }
                        )),
                        cx,
                    );
                } else {
                    this.toast(
                        Toast::danger(format!(
                            "Archived {succeeded}; could not archive {failures}"
                        )),
                        cx,
                    );
                }
                this.load_run_history(cx);
            })
            .ok();
        })
        .detach();
    }
}
