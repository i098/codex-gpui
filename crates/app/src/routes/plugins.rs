use crate::*;
use gpui::Div;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &ui::OpenPlugins, window, cx| {
        this.open_plugins("/plugins", window, cx);
    }))
}
