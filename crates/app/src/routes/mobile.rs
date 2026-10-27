use crate::*;
use gpui::Div;

pub(super) fn actions(root: Div, cx: &mut Context<Root>) -> Div {
    root.on_action(
        cx.listener(|this, _: &OpenMobileSetup, window, cx| {
            this.navigate(Route::Mobile, window, cx)
        }),
    )
    .on_action(cx.listener(|this, _: &mobile::GetStarted, _, cx| this.start_mobile_setup(cx)))
    .on_action(
        cx.listener(|this, _: &mobile::ManageConnections, window, cx| {
            this.open_settings_section("Connections", window, cx);
        }),
    )
    .on_action(cx.listener(|this, _: &mobile::FinishSetup, window, cx| {
        this.navigate(Route::Home, window, cx);
    }))
}

pub(super) fn page(root: &mut Root, cx: &mut Context<Root>) -> Entity<Shell> {
    let (shell, page) = mobile::shell(cx);
    root.mobile_page = Some(page);
    crate::connections::open_mobile(root, cx);
    shell
}
