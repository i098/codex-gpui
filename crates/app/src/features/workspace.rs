use crate::*;
use ui::{ShowWorkspaceTabView, StepWorkspaceLayout};

pub(super) fn actions(root: gpui::Div, cx: &mut Context<Root>) -> gpui::Div {
    root.on_action(cx.listener(|this, _: &ShowWorkspaceTabView, window, cx| {
        this.set_workspace_layout(this.workspace_layout.toggle_full(), window, cx);
    }))
    .on_action(cx.listener(|this, _: &StepWorkspaceLayout, window, cx| {
        this.set_workspace_layout(this.workspace_layout.step(false), window, cx);
    }))
}

impl Root {
    fn set_workspace_layout(
        &mut self,
        layout: WorkspaceLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.split.is_none() || self.tabs.is_empty() || self.dialog.is_some() {
            return;
        }
        let was_chat = self.workspace_layout == WorkspaceLayout::Chat;
        self.workspace_layout = layout;
        self.sync(window, cx);
        if layout == WorkspaceLayout::Split
            && !was_chat
            && let Some(split) = &self.split
        {
            window.focus(&split.focus_handle(cx), cx);
        }
    }
}
