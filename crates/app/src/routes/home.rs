use crate::*;
use gpui::Div;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &NewChat, window, cx| this.new_chat(window, cx)))
        .on_action(cx.listener(|this, _: &OpenFolder, window, cx| this.open_folder(window, cx)))
        .on_action(cx.listener(|this, a: &SelectProject, window, cx| {
            this.select_project(a.0.clone(), window, cx)
        }))
}
