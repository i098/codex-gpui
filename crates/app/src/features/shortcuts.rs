use crate::*;
use gpui::{MouseButton, NavigationDirection};

actions!(app, [NextRecentChat, PreviousRecentChat]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-tab", NextRecentChat, None),
        KeyBinding::new("ctrl-shift-tab", PreviousRecentChat, None),
        KeyBinding::new("ctrl-shift-g", OpenChangesPanel, None),
        KeyBinding::new("ctrl-`", OpenBackgroundTerminal, None),
    ]);
}

pub(super) fn actions(root: gpui::Div, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(cx.listener(|this, _: &NextRecentChat, window, cx| {
        this.cycle_recent_chat(1, window, cx);
    }))
    .on_action(cx.listener(|this, _: &PreviousRecentChat, window, cx| {
        this.cycle_recent_chat(-1, window, cx);
    }))
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
    pub(crate) fn remember_recent_chat(&mut self) {
        let Route::Thread(id) = &self.state.route else {
            return;
        };
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
