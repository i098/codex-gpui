use gpui::{App, Div, Global, IntoElement};
use theme::Theme;

#[derive(Default)]
pub struct Updates {
    pub managed: bool,
}

impl Global for Updates {}

pub fn managed_row(t: &Theme, cx: &App) -> Option<Div> {
    cx.try_global::<Updates>()
        .filter(|updates| updates.managed)
        .map(|_| {
            crate::screens::settings::settings_row(
                "In-app updates",
                Some("Your organization has turned off in-app updates"),
                crate::text("Managed", 13., 18.571_428, 430., t.text_secondary).into_any_element(),
                t,
            )
        })
}
