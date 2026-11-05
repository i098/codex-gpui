use crate::*;
use gpui::Div;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &NewChat, window, cx| this.new_chat(window, cx)))
        .on_action(cx.listener(|this, _: &OpenFolder, window, cx| this.open_folder(window, cx)))
        .on_action(cx.listener(|this, a: &SelectProject, window, cx| {
            this.select_project(a.0.clone(), window, cx)
        }))
        .on_action(cx.listener(|this, _: &ClearProject, window, cx| this.clear_project(window, cx)))
}

impl Root {
    /// `⌘N`, the `New chat` row and rail Home: Home with an empty, focused composer.
    pub(crate) fn new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.shortcut_state.new_chat_mode = features::shortcuts::NewChatMode::Project;
        self.home.update(cx, |home, cx| {
            home.project = self.state.project_root.as_deref().map(project_name);
            cx.notify();
        });
        self.new_chat_section = None;
        if self.state.route == Route::Home {
            self.back
                .push((Route::Home, self.sidebar_automation.take()));
            self.forward.clear();
            self.navigation_epoch += 1;
            self.sync(window, cx);
        }
        self.navigate(Route::Home, window, cx);
        let composer = self.home.read(cx).composer.clone();
        composer.update(cx, |c, cx| c.clear_session(cx));
        window.focus(&composer.focus_handle(cx), cx);
    }
}
