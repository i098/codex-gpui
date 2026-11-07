//! Sidebar `Your dot` (`/dots/home`). The desktop original opens the dot when
//! `/accounts/check` reports `tbo_config.tbo_available`; otherwise its dot home shows
//! `Couldn't load your dot` with `Try again` (`restricted.aeon.accountUnavailable`). The
//! bundled `DotsLandingRoute` eligibility sheet is not mounted on desktop: `/dots` redirects to
//! `/dots/home`.

use crate::{Shell, text};
use gpui::{
    AnyView, App, AppContext as _, Context, Entity, EventEmitter, Global, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, StatefulInteractiveElement, Styled, Window,
    div, prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use theme::ActiveTheme as _;

/// Whether the signed-in account can use its dot, once an `/accounts/check` read answered.
/// The sidebar hides `Your dot` when it is `false`, as the original's `isCapable` does.
pub struct Capable(pub bool);

impl Global for Capable {}

/// The original's `isCapable`: `tbo_config.tbo_available === true` for `account_id`.
pub fn capable(accounts: &Value, account_id: &str) -> bool {
    accounts["accounts"][account_id]["account"]["tbo_config"]["tbo_available"] == true
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Load {
    #[default]
    Loading,
    /// The account cannot use its dot, or the read failed.
    Failed(String),
    /// The dot opened elsewhere; the page is about to leave.
    Opened,
}

/// `Try again`.
pub struct Refresh;

pub struct Page {
    pub load: Load,
    /// Bumped per load so a stale response cannot overwrite a newer one.
    pub generation: u64,
}

impl EventEmitter<Refresh> for Page {}

/// The dot home's content column (`max-w-[708px]` at 1280 px).
const WIDTH: f32 = 708.;

impl Page {
    pub fn new() -> Self {
        Self {
            load: Load::Loading,
            generation: 0,
        }
    }

    /// The original's error row: `text-token-text-error` on a faint error fill, with a
    /// bordered `Try again`.
    fn error(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let retry = div()
            .id("dots-retry")
            .debug_selector(|| "dots-retry".into())
            .role(gpui::Role::Button)
            .aria_label("Try again")
            .tab_index(0)
            .h(px(28.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded_full()
            .border_1()
            .border_color(t.text_error.opacity(0.2))
            .hover(|s| s.bg(t.text_error.opacity(0.1)))
            .child(text("Try again", 14., 20., 430., t.text_error))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(Refresh)))
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.emit(Refresh);
                    cx.stop_propagation();
                }
            }));
        div()
            .id("dots-error")
            .role(gpui::Role::Alert)
            .h(px(57.))
            .pl(px(20.))
            .pr(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(16.))
            .border_1()
            .border_color(t.border)
            .bg(t.text_error.opacity(0.04))
            .child(text("Couldn't load your dot", 14., 20., 430., t.text_error))
            .child(retry)
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let failed = matches!(self.load, Load::Failed(_));
        let error = failed.then(|| self.error(cx));
        div()
            .id("dots-page")
            .size_full()
            .bg(t.surface)
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(WIDTH))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(35.))
                    .pt(px(18.))
                    .child(text("Your dot", 28., 36., 430., t.text))
                    .when_some(error, |column, error| column.child(error)),
            )
    }
}

/// `/dots/home`: the Home sidebar beside the page, as the original keeps it.
pub fn shell(page: Entity<Page>, sidebar: AnyView, cx: &mut App) -> Entity<Shell> {
    cx.new(|_| {
        let mut shell = Shell::new(sidebar, page.into());
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_an_available_dot_is_capable() {
        let check = |tbo: Value| json!({"accounts": {"a1": {"account": {"tbo_config": tbo}}}});
        assert!(capable(&check(json!({"tbo_available": true})), "a1"));
        assert!(!capable(
            &check(json!({"tbo_available": false, "plan_eligible": true})),
            "a1"
        ));
        assert!(!capable(&check(json!({})), "a1"));
        assert!(!capable(&check(json!({"tbo_available": true})), "other"));
    }
}
