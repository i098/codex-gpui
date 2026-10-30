//! Profile menu ▸ `Your dot` (`/dots/home`): the original's `DotsLandingRoute` eligibility
//! modal, fed by `GET /accounts/check/v4-2023-04-27` and statsig gate `1207782907`.

use crate::screens::settings::button;
use crate::{EmptySidebar, Shell, focus, text};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, px,
};
use serde_json::Value;
use theme::ActiveTheme as _;

/// The `business-member` seat request (`ready`, `pending`, `sent`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Request {
    Ready,
    Pending,
    Sent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum State {
    #[default]
    Loading,
    LoadError,
    /// `tbo_available`: the original redirects to its dot chat; the port has none, so it links
    /// to the same page in ChatGPT.
    Available,
    Upgrade,
    ProRollout,
    ProRegion,
    Account,
    BusinessOwner,
    /// `None`: the member cannot request a seat from here.
    BusinessMember(Option<Request>),
    Workspace,
}

/// The original's `ve()` decision over `accounts[id]` (`{account, features}`).
pub fn eligibility(entry: &Value, region_gate: bool) -> State {
    let account = &entry["account"];
    let (Some(available), Some(structure), Some(plan)) = (
        account["tbo_config"]["tbo_available"].as_bool(),
        account["structure"].as_str(),
        account["plan_type"].as_str(),
    ) else {
        return State::LoadError;
    };
    if available {
        return State::Available;
    }
    if structure == "personal" {
        return match plan {
            "prolite" => State::ProRollout,
            "pro" | "promax" if region_gate => State::ProRegion,
            "pro" | "promax" => State::Account,
            _ => State::Upgrade,
        };
    }
    if plan != "team" || account["tbo_config"]["plan_eligible"] != false {
        return State::Workspace;
    }
    if account["account_user_role"] == "account-owner" {
        return State::BusinessOwner;
    }
    let can_request = account["is_trial"] != true
        && account["is_view_only"] != true
        && account["subscription_seat_pools_enabled"] == true
        && entry["features"]
            .as_array()
            .is_some_and(|features| features.iter().any(|f| f == "self_serve_business_prolite"));
    State::BusinessMember(can_request.then_some(Request::Ready))
}

/// The modal's single submit button.
pub struct Primary;

pub struct Page {
    pub state: State,
    /// Bumped per load so a stale response cannot overwrite a newer one.
    pub generation: u64,
    /// The checked `ChatGPT-Account-ID`, for `Manage seats` and the seat request.
    pub account_id: String,
}

impl EventEmitter<Primary> for Page {}

impl Page {
    pub fn new() -> Self {
        Self {
            state: State::Loading,
            generation: 0,
            account_id: String::new(),
        }
    }

    /// Title, optional description and button label, verbatim `orbit.*` defaults.
    pub fn copy(&self) -> Option<(&'static str, Option<&'static str>, &'static str)> {
        const GOT_IT: &str = "Got it";
        const BUSINESS: &str = "Dots require a\nBusiness Premium seat";
        Some(match self.state {
            State::Loading => return None,
            State::LoadError => ("We couldn’t check your access to dots", None, "Try again"),
            State::Available => ("Continue in the ChatGPT app", None, "Open ChatGPT"),
            State::Upgrade => (
                "Dots require a Pro plan",
                Some("Upgrade to Pro 100 to be eligible as access rolls out"),
                "Upgrade to Pro",
            ),
            State::ProRollout => (
                "Dots aren't available on your plan yet",
                Some("We are rolling out dots gradually. We’ll let you know when you have access."),
                GOT_IT,
            ),
            State::ProRegion => (
                "Dots aren’t available in your region yet",
                Some(
                    "Your Pro plan doesn't have access to dots yet. We’ll let you know when they become available.",
                ),
                GOT_IT,
            ),
            State::Account => (
                "Dots aren’t available for this account",
                Some("Your account doesn’t meet the requirements to use dots"),
                GOT_IT,
            ),
            State::BusinessOwner => (
                BUSINESS,
                Some("You’re on a Standard seat. Upgrade to Business Premium to access dots."),
                "Manage seats",
            ),
            State::BusinessMember(None) => (
                BUSINESS,
                Some(
                    "You’re on a Standard seat. Ask your workspace owner about a Business Premium seat.",
                ),
                GOT_IT,
            ),
            State::BusinessMember(Some(Request::Sent)) => (
                "Request sent",
                Some("Your request for a Premium seat was sent"),
                "Done",
            ),
            State::BusinessMember(Some(_)) => (
                BUSINESS,
                Some(
                    "You’re on a Standard seat. Send a request to your workspace owner to upgrade to Premium.",
                ),
                "Request a Premium seat",
            ),
            State::Workspace => ("Dots aren’t available in your workspace yet", None, "Done"),
        })
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let pending = self.state == State::BusinessMember(Some(Request::Pending));
        let body = self.copy().map(|(title, description, label)| {
            let handle = focus::handle("dots-primary", 0, window, cx);
            let action = focus::ring(
                button(label, t.text_alpha_5, None, &t)
                    .id("dots-primary")
                    .role(gpui::Role::Button)
                    .aria_label(label),
                &handle,
                px(12.5),
                px(2.),
                cx,
            )
            .when(pending, |b| b.opacity(0.4))
            .when(!pending, |b| {
                b.on_click(cx.listener(|_, _, _, cx| cx.emit(Primary)))
                    .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.emit(Primary);
                            cx.stop_propagation();
                        }
                    }))
            });
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(7.))
                .pt(px(44.))
                .child(text(title, 20., 28., 600., t.text))
                .children(description.map(|d| text(d, 13., 18.57, 430., t.text_secondary)))
                .child(div().pt(px(12.)).child(action))
        });
        div()
            .id("dots-page")
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .px(px(30.))
            .bg(t.surface)
            .children(body)
    }
}

/// `/dots/home`: sidebar closed, no rail item current.
pub fn shell(page: Entity<Page>, cx: &mut App) -> Entity<Shell> {
    let sidebar = cx.new(|_| EmptySidebar).into();
    cx.new(|_| {
        let mut shell = Shell::new(sidebar, page.into());
        shell.sidebar_open = false;
        shell.chat_chrome = false;
        shell.new_tab = false;
        shell.main_backgrounds = 1;
        shell
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(structure: &str, plan: &str, extra: Value) -> Value {
        let mut account = json!({
            "tbo_config": {"tbo_available": false, "plan_eligible": false},
            "structure": structure,
            "plan_type": plan,
        });
        for (key, value) in extra.as_object().unwrap() {
            account[key] = value.clone();
        }
        json!({"account": account, "features": ["self_serve_business_prolite"]})
    }

    #[test]
    fn eligibility_covers_every_state() {
        let none = json!({});
        assert_eq!(eligibility(&json!({}), false), State::LoadError);
        assert_eq!(
            eligibility(
                &json!({"account": {"structure": "personal", "plan_type": "pro"}}),
                false
            ),
            State::LoadError
        );
        let available = json!({"tbo_config": {"tbo_available": true}});
        assert_eq!(
            eligibility(&entry("personal", "pro", available), false),
            State::Available
        );
        assert_eq!(
            eligibility(&entry("personal", "prolite", none.clone()), true),
            State::ProRollout
        );
        assert_eq!(
            eligibility(&entry("personal", "promax", none.clone()), true),
            State::ProRegion
        );
        assert_eq!(
            eligibility(&entry("personal", "pro", none.clone()), false),
            State::Account
        );
        assert_eq!(
            eligibility(&entry("personal", "plus", none.clone()), true),
            State::Upgrade
        );
        let owner = json!({"account_user_role": "account-owner"});
        assert_eq!(
            eligibility(&entry("workspace", "team", owner), false),
            State::BusinessOwner
        );
        assert_eq!(
            eligibility(&entry("workspace", "team", none.clone()), false),
            State::BusinessMember(None)
        );
        let pools = json!({"subscription_seat_pools_enabled": true});
        assert_eq!(
            eligibility(&entry("workspace", "team", pools.clone()), false),
            State::BusinessMember(Some(Request::Ready))
        );
        let trial = json!({"subscription_seat_pools_enabled": true, "is_trial": true});
        assert_eq!(
            eligibility(&entry("workspace", "team", trial), false),
            State::BusinessMember(None)
        );
        let eligible = json!({"tbo_config": {"tbo_available": false, "plan_eligible": true}});
        assert_eq!(
            eligibility(&entry("workspace", "team", eligible), false),
            State::Workspace
        );
        assert_eq!(
            eligibility(&entry("workspace", "enterprise", pools), false),
            State::Workspace
        );
    }
}
