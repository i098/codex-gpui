use crate::*;
use std::cmp::Reverse;

actions!(
    app,
    [
        ToggleChatPin,
        RenameChat,
        MarkChatUnread,
        NextChatNeedingAttention,
        ClearAllUnreads,
        CopyChatWorkingDirectory,
        CopyChatDeeplink
    ]
);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("alt-cmd-p", ToggleChatPin, None),
        KeyBinding::new("alt-cmd-r", RenameChat, None),
        KeyBinding::new("cmd-shift-u", MarkChatUnread, None),
        KeyBinding::new("alt-cmd-a", NextChatNeedingAttention, None),
        KeyBinding::new("shift-escape", ClearAllUnreads, None),
        KeyBinding::new("cmd-shift-c", CopyChatWorkingDirectory, None),
        KeyBinding::new("alt-cmd-shift-c", CopyChatWorkingDirectory, None),
        KeyBinding::new("alt-cmd-l", CopyChatDeeplink, None),
    ]);
}

pub(super) fn actions(root: gpui::Div, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(cx.listener(|this, _: &ToggleChatPin, window, cx| {
        if let Route::Thread(id) = &this.state.route
            && let Some(thread) = this.threads.iter().find(|thread| thread.id == *id)
        {
            let pinned = thread
                .section
                .as_ref()
                .is_some_and(|section| section.id == PINNED_SECTION_ID);
            this.set_pinned(id.clone(), !pinned, window, cx);
        }
    }))
    .on_action(cx.listener(|this, _: &RenameChat, window, cx| {
        if let Route::Thread(id) = &this.state.route {
            this.open_rename(id.clone(), true, window, cx);
        }
    }))
    .on_action(cx.listener(|this, _: &MarkChatUnread, _, cx| {
        if let Route::Thread(id) = &this.state.route {
            this.sidebar
                .update(cx, |sidebar, cx| sidebar.set_unread(id, cx));
        }
    }))
    .on_action(
        cx.listener(|this, _: &NextChatNeedingAttention, window, cx| {
            let unread = &this.sidebar.read(cx).unread;
            let selected = match &this.state.route {
                Route::Thread(id) => Some(id.as_str()),
                _ => None,
            };
            let target = this
                .threads
                .iter()
                .filter(|thread| Some(thread.id.as_str()) != selected)
                .filter_map(|thread| {
                    let waiting = this
                        .approvals
                        .iter()
                        .any(|approval| approval.thread() == thread.id);
                    (waiting || unread.contains(&thread.id)).then_some((
                        !waiting,
                        Reverse(thread.updated_at),
                        &thread.id,
                    ))
                })
                .min_by_key(|(unread, recency, _)| (*unread, *recency))
                .map(|(_, _, id)| id.clone());
            if let Some(id) = target {
                this.navigate(Route::Thread(id), window, cx);
            }
        }),
    )
    .on_action(cx.listener(|this, _: &ClearAllUnreads, window, cx| {
        if this.dialog.is_some() {
            return;
        }
        let skip = cx
            .try_global::<host_settings::Settings>()
            .and_then(|settings| {
                settings
                    .read_all()
                    .get("skip-clear-all-unreads-confirmation")
            })
            .is_some_and(|value| value == true);
        if skip {
            this.clear_all_unreads(cx);
        } else {
            this.open_dialog(DialogKind::ClearAllUnreads, window, cx);
            this.dialog_to_composer = matches!(this.state.route, Route::Home | Route::Thread(_));
        }
    }))
    .on_action(cx.listener(|this, _: &CopyChatWorkingDirectory, _, cx| {
        if let Route::Thread(id) = &this.state.route {
            this.copy_working_directory(&id.clone(), cx);
        }
    }))
    .on_action(cx.listener(|this, _: &CopyChatDeeplink, _, cx| {
        if let Route::Thread(id) = &this.state.route {
            this.copy_deeplink(id, cx);
        }
    }))
}

impl Root {
    pub(crate) fn copy_working_directory(&mut self, id: &str, cx: &mut Context<Self>) {
        let cwd = self
            .thread
            .as_ref()
            .map(|thread| thread.read(cx))
            .filter(|thread| thread.id() == id)
            .map(|thread| thread.cwd())
            .or_else(|| {
                self.threads
                    .iter()
                    .find(|thread| thread.id == id)
                    .map(|thread| thread.cwd.as_str())
            });
        if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(cwd.to_owned()));
            self.toast(Toast::success("Copied working directory"), cx);
        }
    }

    pub(crate) fn copy_deeplink(&self, id: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(format!(
            "codex://threads/{id}"
        )));
    }

    pub(crate) fn clear_all_unreads(&mut self, cx: &mut Context<Self>) {
        let unread = self.sidebar.read(cx).unread.clone();
        let host = self.host.clone();
        let read_at = chrono::Utc::now().timestamp_millis();
        let write = cx.background_spawn(async move {
            host.fetch(&host::Request::post(
                "inbox-automation-runs-mark-all-read",
                Some(json!({"readAt": read_at})),
            ))
            .and_then(|response| host::json_body(&response))
        });
        cx.spawn(async move |this, cx| {
            let result = write.await;
            this.update(cx, |this, cx| match result {
                Ok(_) => {
                    this.sidebar.update(cx, |sidebar, cx| {
                        sidebar.unread.retain(|id| !unread.contains(id));
                        cx.notify();
                    });
                }
                Err(_) => this.toast(Toast::danger("Failed to mark chats as read"), cx),
            })
            .ok();
        })
        .detach();
    }
}
