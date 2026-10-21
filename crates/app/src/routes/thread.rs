use crate::*;
use gpui::Div;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &FindInThread, window, cx| {
        if matches!(this.state.route, Route::Thread(_)) {
            if this.side == Some(SidePanel::Changes)
                && let Some(panel) = &this.diff
                && panel.update(cx, |panel, cx| panel.open_find(window, cx))
            {
                return;
            }
            if let Some(thread) = &this.thread {
                thread.update(cx, |thread, cx| thread.open_find(window, cx));
            }
        }
    }))
    .on_action(cx.listener(|this, _: &FindNext, _, cx| {
        if let Some(thread) = &this.thread {
            thread.update(cx, |thread, cx| thread.next_find(false, cx));
        }
    }))
    .on_action(cx.listener(|this, _: &FindPrevious, _, cx| {
        if let Some(thread) = &this.thread {
            thread.update(cx, |thread, cx| thread.next_find(true, cx));
        }
    }))
    .on_action(cx.listener(|this, _: &ui::composer::Stop, _, cx| this.stop(cx)))
    .on_action(
        cx.listener(|this, _: &ui::composer::RequestFullAccess, window, cx| {
            this.open_dialog(DialogKind::FullAccess, window, cx);
        }),
    )
    .on_action(cx.listener(|this, _: &ToggleChangesPanel, window, cx| {
        if this.side == Some(SidePanel::Changes) {
            this.close_side(SidePanel::Changes, window, cx)
        } else {
            this.set_side(SidePanel::Changes, window, cx)
        }
    }))
    .on_action(cx.listener(|this, _: &OpenChangesPanel, window, cx| {
        // The control that opened the panel keeps its focus, as in the original; the route
        // refresh would move it to the composer.
        let focused = window.focused(cx);
        this.set_side(SidePanel::Changes, window, cx);
        if let Some(focused) = focused {
            window.focus(&focused, cx);
        }
    }))
    .on_action(cx.listener(|this, action: &ui::ViewChanges, window, cx| {
        if let Some(panel) = &this.diff {
            panel.update(cx, |panel, cx| panel.view_changes(&action.0, cx));
        }
        // The pressed button keeps its focus, as for `OpenChangesPanel`.
        let focused = window.focused(cx);
        this.set_side(SidePanel::Changes, window, cx);
        if let Some(focused) = focused {
            window.focus(&focused, cx);
        }
    }))
    .on_action(cx.listener(Root::open_file))
    .on_action(cx.listener(|this, _: &OpenBackgroundTerminal, window, cx| {
        this.set_side(SidePanel::Terminal, window, cx)
    }))
    .on_action(cx.listener(|this, action: &OpenTerminalItem, window, cx| {
        if let Some(thread) = &this.thread {
            thread.update(cx, |thread, cx| thread.select_terminal(Some(&action.0), cx));
        }
        if this.side == Some(SidePanel::Terminal) {
            this.sync(window, cx);
        } else {
            this.set_side(SidePanel::Terminal, window, cx);
        }
    }))
    .on_action(cx.listener(|this, _: &StopBackgroundTerminals, _, cx| {
        this.stop_background_terminals(cx);
    }))
    .on_action(cx.listener(|this, a: &ActivateSideTab, window, cx| {
        if let Some(side) = this.tabs.get(a.0).copied() {
            this.set_side(side, window, cx)
        }
    }))
    .on_action(cx.listener(|this, a: &CloseSideTab, window, cx| {
        if let Some(side) = this.tabs.get(a.0).copied() {
            this.close_side(side, window, cx)
        }
    }))
    .on_action(
        cx.listener(|this, _: &OpenCommandMenu, window, cx| this.open_command_menu(window, cx)),
    )
    .on_action(cx.listener(|this, _: &PreviousChat, window, cx| this.step_chat(-1, window, cx)))
    .on_action(cx.listener(|this, _: &NextChat, window, cx| this.step_chat(1, window, cx)))
    .on_action(
        cx.listener(|this, a: &GoToRecentChat, window, cx| this.go_to_recent_chat(a.0, window, cx)),
    )
    .on_action(
        cx.listener(|this, a: &PinThread, window, cx| {
            this.set_pinned(a.0.clone(), a.1, window, cx)
        }),
    )
    .on_action(cx.listener(|this, a: &RenameThread, window, cx| {
        this.open_rename(a.0.clone(), false, window, cx)
    }))
    .on_action(cx.listener(|this, a: &ForkThread, window, cx| this.fork(&a.0, window, cx)))
    .on_action(cx.listener(|this, edit: &EditMessage, window, cx| {
        this.edit_message(edit.clone(), window, cx)
    }))
    .on_action(cx.listener(|this, a: &RevertTurn, window, cx| this.revert(a.0.clone(), window, cx)))
    .on_action(cx.listener(|this, a: &ThreadPressed, _, cx| {
        this.sidebar
            .update(cx, |sidebar, cx| sidebar.set_read(&a.0, cx));
    }))
    .on_action(cx.listener(|this, a: &GoToChat, window, cx| this.go_to_chat(Some(a.0), window, cx)))
}
