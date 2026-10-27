use super::*;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

pub(super) struct Watch {
    path: String,
    dirty: bool,
    _watcher: RecommendedWatcher,
    _events: Task<()>,
}

impl Root {
    pub(super) fn sync_diff_watch(&mut self, cx: &mut Context<Self>) {
        let path = (matches!(self.state.route, Route::Thread(_))
            && self.side == Some(SidePanel::Changes))
        .then(|| {
            self.diff
                .as_ref()
                .and_then(|panel| panel.read(cx).watch_root())
        })
        .flatten();
        if path == self.diff_watch.as_ref().map(|watch| watch.path.as_str()) {
            return;
        }
        let path = path.map(str::to_owned);
        self.diff_watch = None;
        let Some(path) = path else { return };
        let Some(panel) = self.diff.clone() else {
            return;
        };
        let (changed, changes) = async_channel::unbounded();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let event = match event {
                Ok(event) if matches!(event.kind, EventKind::Access(_)) => return,
                Ok(_) => Ok(()),
                Err(error) => Err(error.to_string()),
            };
            let _ = changed.try_send(event);
        })
        .and_then(|mut watcher| {
            watcher.watch(std::path::Path::new(&path), RecursiveMode::Recursive)?;
            Ok(watcher)
        });
        let watcher = match watcher {
            Ok(watcher) => watcher,
            Err(error) => {
                eprintln!("codex-gpui: cannot watch repository: {error}");
                panel.update(cx, |panel, cx| panel.set_watch_limited(true, cx));
                return;
            }
        };
        let watched_path = path.clone();
        let events = cx.spawn(async move |this, cx| {
            while let Ok(change) = changes.recv().await {
                if this
                    .update(cx, |this, cx| {
                        let Some(watch) = &mut this.diff_watch else {
                            return;
                        };
                        if watch.path != watched_path {
                            return;
                        }
                        if let Err(error) = change {
                            eprintln!("codex-gpui: repository watch coverage failed: {error}");
                            panel.update(cx, |panel, cx| panel.set_watch_limited(true, cx));
                        } else {
                            watch.dirty = true;
                            this.refresh_watched_diff(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        self.diff_watch = Some(Watch {
            path,
            dirty: true,
            _watcher: watcher,
            _events: events,
        });
        if let Some(panel) = &self.diff {
            panel.update(cx, |panel, cx| panel.set_watch_limited(false, cx));
        }
        self.refresh_watched_diff(cx);
    }

    pub(super) fn refresh_watched_diff(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.diff.clone() else {
            return;
        };
        if panel.read(cx).pending() {
            return;
        }
        let Some(watch) = &mut self.diff_watch else {
            return;
        };
        if !std::mem::take(&mut watch.dirty) {
            return;
        }
        let path = watch.path.clone();
        self.repository_request(panel, &path, &DiffRequest::Refresh, cx);
    }

    pub(super) fn refresh_diff(
        &mut self,
        panel: Entity<DiffPanel>,
        cwd: &str,
        cx: &mut Context<Self>,
    ) {
        if panel.read(cx).watch_limited() {
            self.diff_watch = None;
        }
        self.repository_request(panel, cwd, &DiffRequest::Refresh, cx);
    }

    pub(super) fn notify_git_init(&mut self, error: Option<&str>, cx: &mut Context<Self>) {
        self.toast(
            match error {
                Some(error) => Toast::danger(format!("Git init failed: {error}")),
                None => Toast::info("Git repository created"),
            },
            cx,
        );
    }

    pub(super) fn mark_diff_reverted(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = &self.diff {
            panel.update(cx, |panel, cx| {
                panel.set_last_turn_state(ui::screens::diff::panel::LastTurnState::Reverted, cx)
            });
        }
    }

    pub(super) fn mark_diff_committed(&mut self, cwd: &str, cx: &mut Context<Self>) {
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| thread.read(cx).cwd() == cwd)
            && let Some(panel) = &self.diff
        {
            panel.update(cx, |panel, cx| {
                panel.set_last_turn_state(
                    ui::screens::diff::panel::LastTurnState::CommittedOrReverted,
                    cx,
                )
            });
        }
    }
}
