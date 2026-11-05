//! `Settings → Account` (`account-settings`): the ChatGPT account's status, name and email,
//! and the `Delete account` confirmation for personal accounts. The app sends the deletion
//! through [`Dispatch`]; the server enforces the recent-login and organization locks.

use super::{Layer, deco, on_card, probe, settings_row, settings_row_content};
use crate::composer::Composer;
use crate::screens::computer_use::Load;
use crate::{LogOut, focus, text};
use gpui::{
    AnyElement, App, AppContext as _, Context, Div, Entity, Global, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder as _, px,
};
use std::sync::Arc;
use theme::{ActiveTheme as _, Theme};

/// `GET /me` and the account's `structure` from `/accounts/check`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub plan: String,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub personal: bool,
}

impl Profile {
    /// What the user types to confirm: the email, else the phone number.
    pub fn contact_point(&self) -> Option<&str> {
        self.email.as_deref().or(self.phone.as_deref())
    }

    /// The original enables deletion once `DELETE` and the contact point (any case) match.
    pub fn confirms(&self, contact: &str, keyword: &str) -> bool {
        keyword.trim() == "DELETE"
            && self
                .contact_point()
                .is_none_or(|point| contact.to_lowercase() == point.to_lowercase())
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    Refresh,
    Delete,
    RefreshLogin,
}

pub type Dispatch = Arc<dyn Fn(Action, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct State {
    pub profile: Load<Profile>,
    pub deleting: bool,
    /// The server wants a login from the last 10 minutes before it deletes the account.
    pub needs_recent_login: bool,
    /// Bumped when the dialog must close (deleted or sent to refresh the login).
    pub close_revision: u64,
    pub deleted: bool,
    pub dispatch: Option<Dispatch>,
}
impl Global for State {}

fn dispatch(action: Action, cx: &mut App) {
    if let Some(dispatch) = cx.try_global::<State>().and_then(|s| s.dispatch.clone()) {
        dispatch(action, cx);
    }
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("account-settings", cx, |window, cx| {
            cx.observe_global_in::<State>(window, |this: &mut Account, window, cx| {
                let state = cx.global::<State>();
                if state.close_revision != this.seen_close {
                    this.seen_close = state.close_revision;
                    this.dialog_open = false;
                    if state.deleted {
                        window.dispatch_action(Box::new(LogOut), cx);
                    }
                }
                cx.notify();
            })
            .detach();
            dispatch(Action::Refresh, cx);
            let fields = ["", "DELETE"].map(|placeholder| {
                let field = cx.new(|cx| Composer::settings_field(placeholder, "", false, cx));
                cx.observe(&field, |_: &mut Account, _, cx| cx.notify())
                    .detach();
                field
            });
            Account {
                layer: Layer::default(),
                dialog_open: false,
                fields,
                seen_close: cx.default_global::<State>().close_revision,
            }
        })
        .into_any_element()
}

struct Account {
    layer: Layer,
    dialog_open: bool,
    /// The contact-point and `DELETE` confirmations.
    fields: [Entity<Composer>; 2],
    seen_close: u64,
}

impl Account {
    #[allow(clippy::too_many_arguments)]
    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        fill: gpui::Hsla,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let t = cx.theme().clone();
        let handle = focus::handle(id, 0, window, cx);
        let on_click = Arc::new(on_click);
        let on_key = on_click.clone();
        let label_color = if fill == t.text_alpha_5 {
            t.text
        } else {
            t.background_panel
        };
        focus::ring(
            div()
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(SharedString::from(label))
                .relative()
                .h(px(28.))
                .px(px(9.))
                .flex()
                .items_center()
                .justify_center()
                .child(on_card(12.5, shapes::Corner::Squircle, fill, None))
                .child(text(label, 13., 18., 430., label_color))
                .when(disabled, |d| d.opacity(0.6)),
            &handle,
            px(12.5),
            px(2.),
            cx,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if !disabled {
                on_click(this, cx);
                cx.notify();
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            if !disabled && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                on_key(this, cx);
                cx.notify();
                cx.stop_propagation();
            }
        }))
        .into_any_element()
    }

    fn card(&self, rows: Vec<Div>, t: &Theme) -> Div {
        let last = rows.len().saturating_sub(1);
        let divider = crate::over(t.border, t.background_panel);
        div()
            .relative()
            .p(px(1.))
            .child(deco(
                &self.layer,
                20.,
                shapes::Corner::Squircle,
                t.background_panel,
                Some(t.border),
            ))
            .child(crate::rounded_clip(
                px(19.),
                shapes::Corner::Squircle,
                div()
                    .flex()
                    .flex_col()
                    .children(rows.into_iter().enumerate().map(|(i, row)| {
                        row.when(i < last, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left(px(16.))
                                    .right(px(16.))
                                    .bottom_0()
                                    .h(px(1.))
                                    .bg(divider),
                            )
                        })
                    })),
            ))
    }

    fn open_dialog(&mut self, cx: &mut Context<Self>) {
        for field in &self.fields {
            field.update(cx, |field, cx| field.set_text("", cx));
        }
        self.dialog_open = true;
    }

    fn input(&self, index: usize, label: SharedString, cx: &Context<Self>) -> AnyElement {
        div()
            .id(("account-delete-input", index))
            .role(gpui::Role::TextInput)
            .aria_label(label)
            .aria_value(self.fields[index].read(cx).field_text().to_owned())
            .w_full()
            .child(self.fields[index].clone())
            .into_any_element()
    }

    fn delete_dialog(
        &self,
        profile: &Profile,
        state: &State,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        let busy = state.deleting;
        let line = |s: &'static str| {
            text(s, 13., 19., 430., t.text_secondary)
                .whitespace_normal()
                .into_any_element()
        };
        let mut body = div()
            .relative()
            .w(px(480.))
            .p(px(24.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(on_card(
                20.,
                shapes::Corner::Squircle,
                t.background_panel,
                Some(t.border),
            ))
            .child(text("Delete account—are you sure?", 20., 26., 500., t.text))
            .child(line("Deleting your account is permanent and cannot be undone"))
            .child(line(
                "You will lose access to OpenAI services, including ChatGPT and the API",
            ))
            .child(line(
                "You cannot create a new account using the same email address",
            ))
            .child(line("Your data will be deleted within 30 days, except for limited data we may keep longer where required or permitted by law"))
            .child(
                div()
                    .id("account-delete-article")
                    .cursor_pointer()
                    .child(
                        text(
                            "Read our help center article for more information",
                            13.,
                            19.,
                            430.,
                            t.text_accent,
                        )
                        .whitespace_normal(),
                    )
                    .on_click(|_, _, cx| {
                        cx.open_url(
                            "https://help.openai.com/en/articles/6378407-how-to-delete-your-account",
                        )
                    }),
            );
        if state.needs_recent_login {
            body = body.child(
                text(
                    "You may only delete your account if you logged in within the last 10 minutes. Refresh your login, then return here to continue",
                    13.,
                    19.,
                    430.,
                    t.text_error,
                )
                .whitespace_normal(),
            );
        } else {
            let contact = if profile.email.is_some() {
                "Please type your account email"
            } else {
                "Please type your account phone number"
            };
            if let Some(point) = profile.contact_point() {
                body = body
                    .child(text(contact, 13., 19., 500., t.text))
                    .child(self.input(0, format!("{contact}: {point}").into(), cx));
            }
            body = body
                .child(text(
                    "To proceed, type \"DELETE\" below",
                    13.,
                    19.,
                    500.,
                    t.text,
                ))
                .child(self.input(1, "DELETE".into(), cx));
        }
        let cancel = self.button(
            "account-delete-cancel",
            "Cancel",
            t.text_alpha_5,
            busy,
            window,
            cx,
            |this, _| this.dialog_open = false,
        );
        let confirm = if state.needs_recent_login {
            self.button(
                "account-refresh-login",
                "Refresh login",
                t.text,
                busy,
                window,
                cx,
                |_, cx| dispatch(Action::RefreshLogin, cx),
            )
        } else {
            let confirmed = profile.confirms(
                self.fields[0].read(cx).field_text(),
                self.fields[1].read(cx).field_text(),
            );
            self.button(
                "account-delete-confirm",
                "Permanently delete my account",
                t.text_error,
                busy || !confirmed,
                window,
                cx,
                |_, cx| dispatch(Action::Delete, cx),
            )
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::black().opacity(0.4))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && !busy {
                    this.dialog_open = false;
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(
                body.child(
                    div()
                        .pt(px(8.))
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(cancel)
                        .child(confirm),
                ),
            )
    }
}

impl Render for Account {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = cx.try_global::<State>().cloned().unwrap_or_default();
        let t = cx.theme().clone();
        let secondary =
            |value: String| text(value, 13., 18., 430., t.text_secondary).into_any_element();
        let (rows, profile) = match &state.profile {
            Load::Loading => (Vec::new(), None),
            Load::Failed(error) => {
                let retry = self.button(
                    "account-retry",
                    "Retry",
                    t.text_alpha_5,
                    false,
                    window,
                    cx,
                    |_, cx| dispatch(Action::Refresh, cx),
                );
                (
                    vec![settings_row_content(
                        "Could not load account settings",
                        Some(secondary(error.clone())),
                        retry,
                        &t,
                    )],
                    None,
                )
            }
            Load::Ready(profile) => {
                let mut rows = vec![
                    settings_row("Status", None, secondary(profile.plan.clone()), &t),
                    settings_row("Name", None, secondary(profile.name.clone()), &t),
                ];
                if let Some(email) = &profile.email {
                    rows.push(settings_row("Email", None, secondary(email.clone()), &t));
                }
                (rows, Some(profile.clone()))
            }
        };
        let mut body = div()
            .w_full()
            .max_w(px(768.))
            .px(px(20.))
            .pb(px(20.))
            .flex()
            .flex_col()
            .gap(px(16.));
        if !rows.is_empty() {
            body = body.child(self.card(rows, &t));
        }
        if let Some(profile) = profile.as_ref().filter(|profile| profile.personal) {
            let delete = self.button(
                "account-delete",
                "Delete",
                t.text_error,
                state.deleting || profile.contact_point().is_none(),
                window,
                cx,
                |this, cx| this.open_dialog(cx),
            );
            body =
                body.child(self.card(vec![settings_row("Delete account", None, delete, &t)], &t));
        }
        let dialog = profile
            .filter(|profile| self.dialog_open && profile.personal)
            .map(|profile| self.delete_dialog(&profile, &state, window, cx));
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(div().flex_none().h(px(46.)))
            .child(
                div()
                    .id("account-page")
                    .flex_1()
                    .overflow_y_scroll()
                    .layer()
                    .child(probe(&self.layer))
                    .child(
                        div()
                            .pt(px(16.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(768.))
                                    .px(px(20.))
                                    .py(px(12.))
                                    .min_h(px(60.))
                                    .flex()
                                    .items_center()
                                    .child(text("Account", 28., 33.6, 500., t.text)),
                            )
                            .child(body),
                    ),
            )
            .children(dialog)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_needs_the_keyword_and_the_contact_point_in_any_case() {
        let profile = Profile {
            email: Some("Me@Example.com".into()),
            phone: Some("+15550100".into()),
            ..Default::default()
        };
        assert!(profile.confirms("me@example.COM", " DELETE "));
        assert!(!profile.confirms("me@example.com", "delete"));
        assert!(!profile.confirms("+15550100", "DELETE"));
        let phone_only = Profile {
            phone: Some("+15550100".into()),
            ..Default::default()
        };
        assert!(phone_only.confirms("+15550100", "DELETE"));
    }
}
