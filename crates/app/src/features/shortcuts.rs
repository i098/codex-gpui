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
