use crate::*;
use gpui::{ClipboardItem, MouseButton, NavigationDirection, prelude::FluentBuilder as _};
use ui::confirmation::{Confirmation, Outcome as ConfirmationOutcome};

actions!(
    app,
    [
        NextRecentChat,
        PreviousRecentChat,
        NewStandaloneChat,
        NewTemporaryChat,
        OpenShortcutSettings,
        ToggleBottomPanel,
        OpenInteractiveTerminal,
        EnvironmentAction1,
        EnvironmentAction2,
        EnvironmentAction3,
        EnvironmentAction4,
        EnvironmentAction5,
        EnvironmentAction6,
        EnvironmentAction7,
        EnvironmentAction8,
        EnvironmentAction9,
        ClearAllUnreads,
        NextAttentionChat,
        MarkOpenChatUnread,
        ZoomIn,
        ZoomOut,
        ActualSize,
        ToggleThreadPin,
        RenameOpenChat,
        NextSideTab,
        PreviousSideTab,
        CopyDeeplink,
        CopyLastCodeBlock,
        CloseWholeWindow,
        CloseOtherSideTabs,
        OpenCurrentSideChat,
        CopyPageLink,
        ForkCurrentThread,
        DeleteCurrentChat,
        CopyConversationMarkdown,
        FocusMainChat,
        FocusSideChat,
        ReopenClosedTab
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
    pub ephemeral: HashSet<String>,
    pub confirmation: Option<Entity<Confirmation>>,
    pub zoom_banner: Option<Entity<ui::zoom::ZoomBanner>>,
    undo: Vec<AppEdit>,
    redo: Vec<AppEdit>,
    undo_pending: bool,
    history_generation: u64,
}

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
        KeyBinding::new("ctrl-shift-g", OpenChangesPanel, None),
        KeyBinding::new("ctrl-`", OpenBackgroundTerminal, None),
        KeyBinding::new("cmd-+", ZoomIn, None),
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd-shift-=", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("cmd-0", ActualSize, None),
    ]);
}

pub(super) fn actions(root: gpui::Div, owner: &Root, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(cx.listener(|this, _: &NextRecentChat, window, cx| {
        this.cycle_recent_chat(1, window, cx);
    }))
    .on_action(cx.listener(|this, _: &PreviousRecentChat, window, cx| {
        this.cycle_recent_chat(-1, window, cx);
    }))
    .on_action(cx.listener(|this, _: &ClearAllUnreads, window, cx| {
        this.confirm_clear_unreads(window, cx);
    }))
    .on_action(cx.listener(|this, _: &NextAttentionChat, window, cx| {
        let current = match &this.state.route {
            Route::Thread(id) => Some(id.as_str()),
            _ => None,
        };
        let next = this
            .approvals
            .iter()
            .map(|request| request.thread())
            .chain(this.sidebar.read(cx).unread.iter().map(String::as_str))
            .find(|id| Some(*id) != current)
            .map(str::to_owned);
        if let Some(next) = next {
            this.navigate(Route::Thread(next), window, cx);
        }
    }))
    .on_action(cx.listener(|this, _: &MarkOpenChatUnread, _, cx| {
        if let Route::Thread(id) = &this.state.route {
            this.sidebar
                .update(cx, |sidebar, cx| sidebar.toggle_thread_unread(id, cx));
        }
    }))
    .on_action(cx.listener(|this, _: &ToggleThreadPin, window, cx| {
        if let Route::Thread(id) = this.state.route.clone() {
            let pinned = this
                .threads
                .iter()
                .find(|thread| thread.id == id)
                .and_then(|thread| thread.section.as_ref())
                .is_some_and(|section| section.id == PINNED_SECTION_ID);
            this.set_pinned(id, !pinned, window, cx);
        }
    }))
    // The original's `copy-deeplink` (local host, no thread access) copies
    // `codex://threads/{id}` without a toast.
    .on_action(cx.listener(|this, _: &CopyDeeplink, _, cx| {
        if let Route::Thread(id) = &this.state.route {
            cx.write_to_clipboard(ClipboardItem::new_string(format!("codex://threads/{id}")));
        }
    }))
    .on_action(cx.listener(|this, _: &CopyLastCodeBlock, _, cx| {
        let code = this
            .thread
            .as_ref()
            .and_then(|view| view.read(cx).last_code_block());
        if let Some(code) = code {
            cx.write_to_clipboard(ClipboardItem::new_string(code));
        }
    }))
    .on_action(
        cx.listener(|this, _: &CloseWholeWindow, window, cx| this.close_whole_window(window, cx)),
    )
    .on_action(cx.listener(|this, _: &CloseOtherSideTabs, window, cx| {
        if let Some(side) = this.side {
            this.tabs.retain(|tab| *tab == side);
            this.sync(window, cx);
        }
    }))
    .on_action(
        cx.listener(|this, _: &ReopenClosedTab, window, cx| this.reopen_closed_tab(window, cx)),
    )
    .on_action(cx.listener(
        |this, a: &ui::screens::diff::panel::MoveSideTabToBottom, window, cx| {
            if this.tabs.get(a.0) == Some(&SidePanel::Console) {
                this.show_terminal(true, window, cx);
            }
        },
    ))
    .on_action(cx.listener(|this, _: &NextSideTab, window, cx| this.step_side(1, window, cx)))
    .on_action(cx.listener(|this, _: &PreviousSideTab, window, cx| this.step_side(-1, window, cx)))
    .on_action(cx.listener(|this, _: &RenameOpenChat, window, cx| {
        if let Route::Thread(id) = this.state.route.clone() {
            this.open_rename(id, false, window, cx);
        }
    }))
    .on_action(cx.listener(|this, _: &OpenShortcutSettings, window, cx| {
        this.open_settings_section("Keyboard shortcuts", window, cx);
    }))
    .on_action(cx.listener(|this, _: &NewStandaloneChat, window, cx| {
        this.new_special_chat(NewChatMode::Standalone, window, cx);
    }))
    .when(!owner.bridge.is_acp(), |root| {
        root.on_action(cx.listener(|this, _: &NewTemporaryChat, window, cx| {
            this.new_special_chat(NewChatMode::Temporary, window, cx);
        }))
    })
    .on_action(
        cx.listener(|this, _: &ToggleBottomPanel, window, cx| this.toggle_bottom_panel(window, cx)),
    )
    .on_action(
        cx.listener(|this, _: &OpenInteractiveTerminal, window, cx| {
            this.show_terminal(false, window, cx)
        }),
    )
    .on_action(cx.listener(|this, _: &EnvironmentAction1, window, cx| {
        this.run_selected_environment_action(1, window, cx)
    }))
    .on_action(cx.listener(|this, _: &EnvironmentAction2, window, cx| {
        this.run_selected_environment_action(2, window, cx)
    }))
    .on_action(cx.listener(|this, _: &EnvironmentAction3, window, cx| {
        this.run_selected_environment_action(3, window, cx)
    }))
    .on_action(cx.listener(|this, _: &EnvironmentAction4, window, cx| {
        this.run_selected_environment_action(4, window, cx)
    }))
    .on_action(cx.listener(|this, _: &EnvironmentAction5, window, cx| {
        this.run_selected_environment_action(5, window, cx)
    }))
    .on_action(cx.listener(|this, _: &EnvironmentAction6, window, cx| {
        this.run_selected_environment_action(6, window, cx)
    }))
    .on_action(cx.listener(|this, _: &EnvironmentAction7, window, cx| {
        this.run_selected_environment_action(7, window, cx)
    }))
    .on_action(cx.listener(|this, _: &EnvironmentAction8, window, cx| {
        this.run_selected_environment_action(8, window, cx)
    }))
    .on_action(cx.listener(|this, _: &EnvironmentAction9, window, cx| {
        this.run_selected_environment_action(9, window, cx)
    }))
    .on_action(cx.listener(|this, _: &ZoomIn, window, cx| this.change_zoom(Some(1), window, cx)))
    .on_action(cx.listener(|this, _: &ZoomOut, window, cx| this.change_zoom(Some(-1), window, cx)))
    .on_action(cx.listener(|this, _: &ActualSize, window, cx| this.change_zoom(None, window, cx)))
    .on_action(cx.listener(|this, _: &OpenCurrentSideChat, window, cx| {
        this.open_current_side_chat(window, cx);
    }))
    .on_action(cx.listener(|this, _: &CopyPageLink, _, cx| {
        if let Some(url) = this
            .active_browser_tab()
            .and_then(|tab| tab.read(cx).url().cloned())
        {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(url.to_string()));
        }
    }))
    .on_action(cx.listener(|this, _: &ForkCurrentThread, window, cx| {
        if let Route::Thread(id) = this.state.route.clone() {
            this.fork_thread(id, None, None, window, cx);
        }
    }))
    .on_action(cx.listener(|this, _: &DeleteCurrentChat, window, cx| {
        if let Route::Thread(id) = this.state.route.clone() {
            window.dispatch_action(
                Box::new(ui::screens::sidebar::ThreadMenuAction {
                    thread: id,
                    command: ui::screens::sidebar::ThreadCommand::Delete,
                }),
                cx,
            );
        }
    }))
    .on_action(
        cx.listener(|this, _: &CopyConversationMarkdown, window, cx| {
            if let Route::Thread(id) = this.state.route.clone() {
                window.dispatch_action(
                    Box::new(ui::screens::sidebar::ThreadMenuAction {
                        thread: id,
                        command: ui::screens::sidebar::ThreadCommand::CopyMarkdown,
                    }),
                    cx,
                );
            }
        }),
    )
    .on_action(cx.listener(|this, _: &FocusMainChat, window, cx| {
        window.focus(&this.route_focus(cx), cx);
    }))
    .on_action(cx.listener(|this, _: &FocusSideChat, window, cx| {
        this.focus_side_chat(window, cx);
    }))
    .on_mouse_up(
        MouseButton::Navigate(NavigationDirection::Back),
        cx.listener(|_, _, window, cx| super::keymap::dispatch_mouse("MouseBack", window, cx)),
    )
    .on_mouse_up(
        MouseButton::Navigate(NavigationDirection::Forward),
        cx.listener(|_, _, window, cx| super::keymap::dispatch_mouse("MouseForward", window, cx)),
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

    pub(crate) fn new_special_chat(
        &mut self,
        mode: NewChatMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_chat(window, cx);
        self.shortcut_state.new_chat_mode = mode;
        self.home.update(cx, |home, cx| {
            home.project = None;
            cx.notify();
        });
        let composer = self.home.read(cx).composer.clone();
        composer.update(cx, |composer, cx| composer.set_search_roots(Vec::new(), cx));
    }

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
        let (method, params) = match &edit {
            AppEdit::Rename { id, before, after } => (
                "thread/name/set",
                json!({"threadId": id, "name": if undo { before } else { after }}),
            ),
            AppEdit::Section { id, before, after } => (
                "thread/section/move",
                json!({"threadId": id, "sectionId": if undo { before } else { after }, "beforeThreadId": null}),
            ),
            AppEdit::Archive { thread, .. } => (
                if undo {
                    "thread/unarchive"
                } else {
                    "thread/archive"
                },
                json!({"threadId": thread.id}),
            ),
        };
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
                match &edit {
                    AppEdit::Rename { id, before, after } => {
                        this.apply_thread_title(id, if undo { before } else { after }, cx);
                    }
                    AppEdit::Section { id, before, after } => {
                        let section = if undo { before } else { after };
                        if let Some(thread) =
                            this.threads.iter_mut().find(|thread| &thread.id == id)
                        {
                            thread.section =
                                section.as_ref().map(|id| ThreadSection { id: id.clone() });
                        }
                        this.sidebar.update(cx, |sidebar, cx| {
                            sidebar.remove_section_thread(id, cx);
                            if let Some(section) = section {
                                sidebar.assign_section(id, section, cx);
                            }
                            sidebar.set_threads(&this.threads, cx);
                        });
                    }
                    AppEdit::Archive { thread, index } => {
                        this.threads.retain(|row| row.id != thread.id);
                        if undo {
                            let restored = result
                                .as_ref()
                                .ok()
                                .and_then(|value| {
                                    serde_json::from_value::<Thread>(value["thread"].clone()).ok()
                                })
                                .unwrap_or_else(|| thread.as_ref().clone());
                            this.threads
                                .insert((*index).min(this.threads.len()), restored);
                            if let Some(page) = &this.archive_page {
                                page.update(cx, |page, cx| page.unarchived(&thread.id, cx));
                            }
                        } else if this.state.route == Route::Thread(thread.id.clone()) {
                            this.new_chat(window, cx);
                        }
                        this.sidebar
                            .update(cx, |sidebar, cx| sidebar.set_threads(&this.threads, cx));
                    }
                }
                let message = match (&edit, undo) {
                    (AppEdit::Archive { .. }, true) => "Chat restored",
                    (
                        AppEdit::Section {
                            before: None,
                            after: Some(_),
                            ..
                        },
                        true,
                    ) => "Chat unpinned",
                    (_, true) => "Action undone",
                    (_, false) => "Action redone",
                };
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

impl Root {
    fn confirm_clear_unreads(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let skip = cx
            .global::<host_settings::Settings>()
            .read_all()
            .get("skip-clear-all-unreads-confirmation")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if skip {
            self.clear_unreads(cx);
            return;
        }
        if self.shortcut_state.confirmation.is_some() {
            return;
        }
        let confirmation = cx.new(|cx| {
            Confirmation::new(
                "Clear all unreads?",
                "This will mark all chats and scheduled task updates in Codex as read",
                "Clear all",
                window,
                cx,
            )
            .with_opt_out("Don’t ask me again", cx)
        });
        cx.subscribe(&confirmation, |this, modal, result, cx| {
            if *result == ConfirmationOutcome::Confirm {
                if modal.read(cx).opt_out_checked() {
                    let saved = cx.update_global::<host_settings::Settings, _>(|settings, _| {
                        settings.write("skip-clear-all-unreads-confirmation", json!(true))
                    });
                    if let Err(error) = saved {
                        this.toast(Toast::danger(error.to_string()), cx);
                    }
                }
                this.clear_unreads(cx);
            }
            this.shortcut_state.confirmation = None;
            cx.notify();
        })
        .detach();
        self.shortcut_state.confirmation = Some(confirmation);
        cx.notify();
    }

    fn clear_unreads(&self, cx: &mut Context<Self>) {
        let host = self.host.clone();
        let task = cx.spawn(async move |this, cx| -> Result<()> {
            let result = cx
                .background_executor()
                .spawn(async move { host.clear_inbox_unreads() })
                .await;
            this.update(cx, |this, cx| {
                if result.is_ok() {
                    this.sidebar.update(cx, |sidebar, cx| {
                        sidebar.unread.clear();
                        cx.notify();
                    });
                    this.load_automations(cx);
                } else {
                    this.toast(Toast::danger("Could not clear all unreads"), cx);
                }
            })?;
            result
        });
        log_err(task, cx);
    }
}

pub(crate) fn initialize_window(state: &mut State, window: &mut Window) {
    if !state.zoom.is_finite() {
        state.zoom = 1.;
    }
    state.zoom = state.zoom.clamp(0.5, 3.);
    window.set_ui_scale(state.zoom);
    place_traffic_lights(state.zoom, window);
}

/// The original's `trafficLightPosition`: x 16, centred in the 44 px top bar at the zoom level.
fn place_traffic_lights(zoom: f32, window: &Window) {
    #[cfg(target_os = "macos")]
    window.set_traffic_light_position(gpui::point(px(16.), px(((44. * zoom - 14.) / 2.).round())));
    #[cfg(not(target_os = "macos"))]
    let _ = (zoom, window);
}

impl Root {
    fn change_zoom(&mut self, step: Option<i32>, window: &mut Window, cx: &mut Context<Self>) {
        self.state.zoom = step.map_or(1., |step| {
            ((self.state.zoom * 100. + step as f32 * 10.).round() / 100.).clamp(0.5, 3.)
        });
        window.set_ui_scale(self.state.zoom);
        place_traffic_lights(self.state.zoom, window);
        self.fit(window, cx);
        self.save(window, cx);
        if let Some(banner) = &self.shortcut_state.zoom_banner {
            banner.update(cx, |banner, cx| banner.show(self.state.zoom, cx));
        } else {
            let banner = cx.new(|cx| ui::zoom::ZoomBanner::new(self.state.zoom, cx));
            cx.subscribe_in(
                &banner,
                window,
                |this, _, change: &ui::zoom::Change, window, cx| {
                    this.change_zoom(
                        match change {
                            ui::zoom::Change::Step(step) => Some(*step),
                            ui::zoom::Change::Reset => None,
                        },
                        window,
                        cx,
                    );
                },
            )
            .detach();
            self.shortcut_state.zoom_banner = Some(banner);
        }
        cx.notify();
    }

    pub fn zoom_banner(&self) -> Option<&Entity<ui::zoom::ZoomBanner>> {
        self.shortcut_state.zoom_banner.as_ref()
    }
    /// Home composer: `thread/start` with the chosen model and permissions, open it, then
    /// `turn/start`; the turn streams in through [`Self::on_event`].
    pub(crate) fn start_thread(
        &mut self,
        s: Submission,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.qa_work_started = true;
        let bridge = self.bridge.clone();
        let cwd = self
            .state
            .project_root
            .as_ref()
            .unwrap_or(&self.cwd)
            .clone();
        let section = self.new_chat_section.clone();
        let computer_use = computer_use_overrides(&self.config);
        let mode = self.shortcut_state.new_chat_mode;
        let host = self.host.clone();
        let hotkey_options = hotkey::start_options(self, cx);
        let preparing_worktree = mode != NewChatMode::Standalone
            && hotkey_options
                .as_ref()
                .is_some_and(|options| options.mode == ui::screens::hotkey::Mode::Worktree);
        if preparing_worktree {
            self.toast(Toast::info("Creating worktree…"), cx);
        }
        let memory = self.new_chat_memory;
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let result = async {
            let workspace = if mode == features::shortcuts::NewChatMode::Standalone {
                let prepare = host.clone();
                let prompt = s.text.clone();
                match cx.background_executor().spawn(async move { prepare.prepare_projectless(&prompt) }).await {
                    Ok(workspace) => Some(workspace),
                    Err(error) => {
                        this.update(cx, |this, cx| this.restore_start_draft(&s, cx))?;
                        return Err(error);
                    }
                }
            } else { None };
            let prepared = if mode != NewChatMode::Standalone {
                if let Some(options) = hotkey_options {
                    let host = host.clone();
                    let prompt = s.text.clone();
                    match cx.background_executor().spawn(async move { hotkey::prepare_start(host, options, &prompt) }).await {
                        Ok(prepared) => {
                            if preparing_worktree {
                                this.update(cx, |this, cx| {
                                    this.toast(Toast::info("Attaching worktree…"), cx);
                                })?;
                            }
                            Some(prepared)
                        },
                        Err(error) => {
                            this.update(cx, |this, cx| this.restore_start_draft(&s, cx))?;
                            return Err(error);
                        }
                    }
                } else { None }
            } else { None };
            let mut params = json!({ "cwd": workspace.as_ref().map(|workspace| workspace.cwd.as_str()).unwrap_or(&cwd), "model": model(&s) });
            if let Some(workspace) = &workspace {
                params["developerInstructions"] = json!(workspace.instructions());
            }
            if let Some(prepared) = &prepared { prepared.apply(&mut params); }
            if mode == features::shortcuts::NewChatMode::Temporary {
                params["ephemeral"] = json!(true);
            }
            apply_permissions(&mut params, &s.permission);
            if let Some(config) = computer_use {
                params["config"] = config;
            }
            params["dynamicTools"] = json!([crate::terminal::read_thread_terminal_tool()]);
            if let Some(memory) = memory {
                memory.apply(&mut params);
            }
            let mut thread = match bridge
                .request::<_, ThreadStartResponse>("thread/start", params)
                .await
            {
                Ok(started) => started.thread,
                Err(error) => {
                    if let Some(prepared) = prepared {
                        let cleanup = cx.background_executor().spawn(async move { prepared.discard() }).await;
                        if let Err(cleanup) = cleanup {
                            this.update(cx, |this, cx| this.restore_start_draft(&s, cx))?;
                            return Err(error.context(format!("Worktree cleanup failed: {cleanup:#}")));
                        }
                    }
                    // The draft, its attachments and the chosen project stay as they were.
                    this.update(cx, |this, cx| this.restore_start_draft(&s, cx))?;
                    return Err(error);
                }
            };
            // Title the new chat (Recents row, header) with its prompt until it has a preview.
            if thread.preview.is_empty() {
                thread.preview = s.text.clone();
            }
            let id = thread.id.clone();
            if let Some(workspace) = workspace {
                let record = host.clone();
                let thread_id = id.clone();
                if let Err(error) = cx.background_executor().spawn(async move { record.record_projectless(&thread_id, &workspace) }).await {
                    this.update(cx, |this, cx| this.restore_start_draft(&s, cx))?;
                    return Err(error);
                }
            }
            if let Some(prepared) = prepared.filter(|prepared| prepared.needs_recording()) {
                let thread_id = id.clone();
                if let Err(error) = cx.background_executor().spawn(async move { prepared.record(&thread_id) }).await {
                    this.update(cx, |this, cx| this.restore_start_draft(&s, cx))?;
                    return Err(error);
                }
            }
            let section = if let Some(section) = section {
                match bridge
                    .request::<_, Value>(
                        "thread/section/move",
                        json!({
                            "threadId": id, "sectionId": section, "beforeThreadId": null
                        }),
                    )
                    .await
                {
                    Ok(_) => {
                        thread.section = Some(ThreadSection {
                            id: section.clone(),
                        });
                        Some(section)
                    }
                    Err(_) => {
                        this.update(cx, |this, cx| {
                            this.toast(Toast::danger("Couldn't move chat to section"), cx)
                        })?;
                        None
                    }
                }
            } else {
                None
            };
            this.update_in(cx, |this, window, cx| {
                this.new_chat_section = None;
                if let Some(memory) = memory {
                    this.new_chat_memory = None;
                    this.thread_memory.insert(id.clone(), memory);
                }
                this.loaded.insert(id.clone());
                if preparing_worktree {
                    this.worktree_statuses.insert(id.clone(), "Checkout created");
                }
                if mode == features::shortcuts::NewChatMode::Temporary {
                    this.shortcut_state.ephemeral.insert(id.clone());
                } else {
                    this.add_thread(&thread, cx);
                }
                if let Some(section) = &section {
                    this.sidebar
                        .update(cx, |s, cx| s.assign_section(&id, section, cx));
                }
                // `composer.submitInBackground`: the chat starts and shows in Recents; Home stays.
                if !s.background {
                    this.show_thread(thread, window, cx);
                }
            })?;
            let result = turn_start(&bridge, &id, &s, user_input(&s), None).await;
            if result.is_err() {
                this.update(cx, |this, cx| this.restore_failed_send(&s, cx))?;
            }
            result
            }.await;
            if preparing_worktree {
                this.update(cx, |this, cx| {
                    if let Err(error) = &result
                        && format!("{error:#}").contains("Setup script")
                    {
                        this.toast(Toast::danger("Checkout created; setup failed"), cx);
                    }
                })?;
            }
            result
        });
        log_err(task, cx);
    }

    fn open_current_side_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Route::Thread(id) = self.state.route.clone() else {
            return;
        };
        if self.fork_pending {
            return;
        }
        let title: SharedString = if self.side_chats.is_empty() {
            "Side chat".into()
        } else {
            format!("Side chat {}", self.side_chats.len() + 1).into()
        };
        let bridge = self.bridge.clone();
        self.fork_pending = true;
        let task = cx.spawn_in(window, async move |this, cx| -> Result<()> {
            let result = bridge
                .request::<_, Value>("thread/fork", json!({"threadId": id}))
                .await
                .and_then(routes::forked_thread);
            match result {
                Ok((thread, settings)) => {
                    this.update_in(cx, |this, window, cx| {
                        this.fork_pending = false;
                        this.add_side_chat(thread, settings, title, window, cx);
                    })?;
                    Ok(())
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.fork_pending = false;
                        this.toast(Toast::danger("Failed to open side chat"), cx);
                    })?;
                    Err(error)
                }
            }
        });
        log_err(task, cx);
    }

    fn focus_side_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = match self.side {
            Some(SidePanel::SideChat(key)) if self.chat(key).is_some() => Some(key),
            _ => self.side_chats.last().map(|chat| chat.key),
        };
        let Some(key) = key else {
            return;
        };
        self.set_side(SidePanel::SideChat(key), window, cx);
        if let Some(chat) = self.chat(key) {
            window.focus(&chat.composer.focus_handle(cx), cx);
        }
    }
}
