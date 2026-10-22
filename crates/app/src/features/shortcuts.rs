use crate::*;
use gpui::{MouseButton, NavigationDirection, prelude::FluentBuilder as _};

actions!(
    app,
    [
        NextRecentChat,
        PreviousRecentChat,
        NewStandaloneChat,
        NewTemporaryChat
    ]
);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum NewChatMode {
    #[default]
    Project,
    Standalone,
    Temporary,
}

#[derive(Default)]
pub(crate) struct ShortcutState {
    pub new_chat_mode: NewChatMode,
    /// Temporary chats: never in Recents, never restored on launch.
    pub ephemeral: HashSet<String>,
    undo: Vec<AppEdit>,
    redo: Vec<AppEdit>,
    undo_pending: bool,
    history_generation: u64,
}

/// A chat edit `⌘Z` and `⇧⌘Z` reverse and reapply: rename, pin and archive.
#[derive(Clone)]
pub(crate) enum AppEdit {
    Rename {
        id: String,
        before: String,
        after: String,
    },
    Section {
        id: String,
        before: Option<String>,
        after: Option<String>,
    },
    Archive {
        thread: Box<Thread>,
        index: usize,
    },
}

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-tab", NextRecentChat, None),
        KeyBinding::new("ctrl-shift-tab", PreviousRecentChat, None),
        KeyBinding::new("cmd-shift-n", NewTemporaryChat, None),
        KeyBinding::new("alt-cmd-o", NewStandaloneChat, None),
        KeyBinding::new("ctrl-shift-g", OpenChangesPanel, None),
        KeyBinding::new("ctrl-`", OpenBackgroundTerminal, None),
    ]);
}

impl AppEdit {
    fn request(&self, undo: bool) -> (&'static str, Value) {
        match self {
            Self::Rename { id, before, after } => (
                "thread/name/set",
                json!({"threadId": id, "name": if undo { before } else { after }}),
            ),
            Self::Section { id, before, after } => (
                "thread/section/move",
                json!({"threadId": id, "sectionId": if undo { before } else { after }, "beforeThreadId": null}),
            ),
            Self::Archive { thread, .. } => (
                if undo {
                    "thread/unarchive"
                } else {
                    "thread/archive"
                },
                json!({"threadId": thread.id}),
            ),
        }
    }

    fn completion_message(&self, undo: bool) -> &'static str {
        match (self, undo) {
            (Self::Archive { .. }, true) => "Chat restored",
            (
                Self::Section {
                    before: None,
                    after: Some(_),
                    ..
                },
                true,
            ) => "Chat unpinned",
            (_, true) => "Action undone",
            (_, false) => "Action redone",
        }
    }
}

pub(super) fn actions(root: gpui::Div, owner: &Root, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(cx.listener(|this, _: &NextRecentChat, window, cx| {
        this.cycle_recent_chat(1, window, cx);
    }))
    .on_action(cx.listener(|this, _: &PreviousRecentChat, window, cx| {
        this.cycle_recent_chat(-1, window, cx);
    }))
    .on_action(cx.listener(|this, _: &NewStandaloneChat, window, cx| {
        this.new_special_chat(NewChatMode::Standalone, window, cx);
    }))
    .when(!owner.bridge.is_acp(), |root| {
        root.on_action(cx.listener(|this, _: &NewTemporaryChat, window, cx| {
            this.new_special_chat(NewChatMode::Temporary, window, cx);
        }))
    })
    .on_mouse_up(
        MouseButton::Navigate(NavigationDirection::Back),
        cx.listener(|this, _, window, cx| this.step(true, window, cx)),
    )
    .on_mouse_up(
        MouseButton::Navigate(NavigationDirection::Forward),
        cx.listener(|this, _, window, cx| this.step(false, window, cx)),
    )
}

impl Root {
    pub(crate) fn restore_start_draft(&mut self, submission: &Submission, cx: &mut Context<Self>) {
        let composer = self.home.read(cx).composer.clone();
        composer.update(cx, |composer, cx| {
            composer.restore_submission(&submission.text, cx)
        });
        self.toast(Toast::danger("Error starting chat"), cx);
    }

    fn new_special_chat(&mut self, mode: NewChatMode, window: &mut Window, cx: &mut Context<Self>) {
        self.new_chat(window, cx);
        self.shortcut_state.new_chat_mode = mode;
        self.home.update(cx, |home, cx| {
            home.project = None;
            cx.notify();
        });
        let composer = self.home.read(cx).composer.clone();
        composer.update(cx, |composer, cx| composer.set_search_roots(Vec::new(), cx));
    }

    /// A temporary chat is never the route a relaunch restores.
    pub(crate) fn save_shortcut_state(&self) {
        if matches!(&self.state.route, Route::Thread(id) if self.shortcut_state.ephemeral.contains(id))
        {
            let mut state = self.state.clone();
            state.route = Route::Home;
            state.save();
        } else {
            self.state.save();
        }
    }

    pub(crate) fn remember_recent_chat(&mut self) {
        let Route::Thread(id) = &self.state.route else {
            return;
        };
        if self.shortcut_state.ephemeral.contains(id) {
            return;
        }
        if self.recent_chats.first() == Some(id) {
            return;
        }
        self.recent_chats.retain(|previous| previous != id);
        self.recent_chats.insert(0, id.clone());
    }

    fn cycle_recent_chat(&mut self, direction: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = &self.command_menu {
            menu.update(cx, |menu, cx| menu.cycle_recent(direction, cx));
            return;
        }
        let threads: Vec<_> = self
            .recent_chats
            .iter()
            .filter_map(|id| self.threads.iter().find(|thread| &thread.id == id))
            .collect();
        if threads.len() < 2 {
            return;
        }
        let menu = cx.new(|cx| CommandMenu::recent_chats(&threads, direction, cx));
        cx.subscribe_in(&menu, window, |this, _, event, window, cx| {
            this.run_command(event, window, cx);
        })
        .detach();
        window.focus(&menu.focus_handle(cx), cx);
        self.command_menu = Some(menu);
        cx.notify();
    }
}

impl Root {
    pub(crate) fn record_app_edit(&mut self, edit: AppEdit) {
        self.shortcut_state.history_generation += 1;
        self.shortcut_state.redo.clear();
        self.shortcut_state.undo.push(edit);
    }

    pub(crate) fn forget_archive_edit(&mut self, id: &str) {
        if let Some(index) =
            self.shortcut_state.undo.iter().rposition(
                |edit| matches!(edit, AppEdit::Archive { thread, .. } if thread.id == id),
            )
        {
            let edit = self.shortcut_state.undo.remove(index);
            self.shortcut_state.redo.push(edit);
        }
    }

    pub(crate) fn apply_thread_title(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        if let Some(thread) = self.threads.iter_mut().find(|thread| thread.id == id) {
            thread.name = Some(name.to_owned());
        }
        if let Some(view) = &self.thread
            && view.read(cx).id() == id
        {
            view.update(cx, |view, cx| view.set_title(name.into(), cx));
        }
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.set_threads(&self.threads, cx));
    }

    fn apply_completed_edit(
        &mut self,
        edit: &AppEdit,
        undo: bool,
        result: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match edit {
            AppEdit::Rename { id, before, after } => {
                self.apply_thread_title(id, if undo { before } else { after }, cx);
            }
            AppEdit::Section { id, before, after } => {
                let section = if undo { before } else { after };
                if let Some(thread) = self.threads.iter_mut().find(|thread| &thread.id == id) {
                    thread.section = section.as_ref().map(|id| ThreadSection { id: id.clone() });
                }
                self.sidebar.update(cx, |sidebar, cx| {
                    sidebar.remove_section_thread(id, cx);
                    if let Some(section) = section {
                        sidebar.assign_section(id, section, cx);
                    }
                    sidebar.set_threads(&self.threads, cx);
                });
            }
            AppEdit::Archive { thread, index } => {
                self.threads.retain(|row| row.id != thread.id);
                if undo {
                    let restored = serde_json::from_value::<Thread>(result["thread"].clone())
                        .unwrap_or_else(|_| thread.as_ref().clone());
                    self.threads
                        .insert((*index).min(self.threads.len()), restored);
                    if let Some(page) = &self.archive_page {
                        page.update(cx, |page, cx| page.unarchived(&thread.id, cx));
                    }
                } else if self.state.route == Route::Thread(thread.id.clone()) {
                    self.new_chat(window, cx);
                }
                self.sidebar
                    .update(cx, |sidebar, cx| sidebar.set_threads(&self.threads, cx));
            }
        }
    }

    pub(crate) fn undo_app_edit(
        &mut self,
        undo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.shortcut_state.undo_pending {
            return true;
        }
        let source = if undo {
            &mut self.shortcut_state.undo
        } else {
            &mut self.shortcut_state.redo
        };
        let Some(edit) = source.pop() else {
            return false;
        };
        let index = source.len();
        let generation = self.shortcut_state.history_generation;
        self.shortcut_state.undo_pending = true;
        let (method, params) = edit.request(undo);
        let bridge = self.bridge.clone();
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let result = bridge.request::<_, Value>(method, params).await;
            this.update_in(cx, |this, window, cx| {
                this.shortcut_state.undo_pending = false;
                if result.is_err() {
                    let source = if undo {
                        &mut this.shortcut_state.undo
                    } else {
                        &mut this.shortcut_state.redo
                    };
                    source.insert(index.min(source.len()), edit);
                    this.toast(
                        Toast::danger(if undo {
                            "Could not undo action"
                        } else {
                            "Could not redo action"
                        }),
                        cx,
                    );
                    return;
                }
                this.apply_completed_edit(&edit, undo, result.as_ref().unwrap(), window, cx);
                let message = edit.completion_message(undo);
                if undo {
                    if this.shortcut_state.history_generation == generation {
                        this.shortcut_state.redo.push(edit);
                    }
                } else {
                    this.shortcut_state.undo.push(edit);
                }
                this.toast(Toast::info(message), cx);
            })?;
            result.map(drop)
        });
        log_err(task, cx);
        true
    }
}
