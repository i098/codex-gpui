use crate::*;
use gpui::Div;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(cx.listener(|this, _: &ArchiveChat, window, cx| {
        if let Route::Thread(id) = this.state.route.clone() {
            this.archive(id, window, cx)
        }
    }))
    .on_action(
        cx.listener(|this, a: &ArchiveThread, window, cx| this.archive(a.0.clone(), window, cx)),
    )
}

pub(super) fn page(root: &mut Root, cx: &mut Context<Root>) -> Entity<Shell> {
    let (shell, page) = ui::screens::sidebar::archive_shell(cx);
    root.archive_page = Some(page);
    shell
}
