//! Settings ▸ Security (`ChatGptSecuritySettings`, reached only through
//! `codex://settings/security`): the account PIN that guards locked chats
//! (`/locked_chats/...`) and lockdown mode (`lockdown_mode_enabled`).
use super::{
    Layer,
    account_page::{Kind, action, frame, paragraph, status},
    backend::{self, Call},
    settings_row_content, switch,
    worktrees::card,
};
use crate::{composer::Composer, text};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, prelude::FluentBuilder as _, px,
};
use serde_json::json;
use theme::{ActiveTheme as _, Theme};

/// Where the original's desktop build sends a PIN reset: the reset needs a browser sign-in.
pub const RESET_URL: &str = "https://chatgpt.com/settings/security?view=account-pin";
const FAILED: &str = "Couldn’t update your PIN. Please try again.";

#[derive(Clone, Debug, PartialEq)]
pub enum Pin {
    Loading,
    Failed,
    Ready {
        has_pin: bool,
        /// `pin_lockout_retry_after_seconds`.
        retry_after: u64,
    },
}

/// The open PIN dialog step (`create`, `change-current`, `change-new`, native reset notice).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    Create,
    Current,
    New,
    Reset,
}

struct Dialog {
    step: Step,
    first: Entity<Composer>,
    second: Entity<Composer>,
    /// The verified current PIN, for `POST /locked_chats/pin/change`.
    current: String,
    error: Option<&'static str>,
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("security-settings", cx, Security::new)
        .into_any_element()
}

pub struct Security {
    pub pin: Pin,
    /// `settings.lockdown_mode_enabled` from `GET /settings/user`; `None` until read.
    pub lockdown: Option<bool>,
    dialog: Option<Dialog>,
    pub message: Option<(SharedString, bool)>,
    busy: bool,
    layer: Layer,
}

impl Security {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let page = cx.entity();
        window.defer(cx, move |_, cx| {
            page.update(cx, |this, cx| {
                this.load_pin(cx);
                this.load_lockdown(cx);
            })
        });
        Self {
            pin: Pin::Loading,
            lockdown: None,
            dialog: None,
            message: None,
            busy: false,
            layer: Layer::default(),
        }
    }

    pub fn load_pin(&mut self, cx: &mut Context<Self>) {
        let call = Call::new("GET", "/locked_chats/status", None);
        backend::request(call, cx, |this, result, _| {
            this.pin = match result {
                Ok(response) if response.ok() => Pin::Ready {
                    has_pin: response.body["has_pin"] == true,
                    retry_after: response.body["pin_lockout_retry_after_seconds"]
                        .as_u64()
                        .unwrap_or(0),
                },
                _ => Pin::Failed,
            };
        });
    }

    fn load_lockdown(&mut self, cx: &mut Context<Self>) {
        let call = Call::new("GET", "/settings/user", None);
        backend::request(call, cx, |this, result, _| {
            if let Ok(response) = result
                && response.ok()
            {
                this.lockdown = Some(response.body["settings"]["lockdown_mode_enabled"] == true);
            }
        });
    }

    /// `PATCH /settings/account_user_setting?feature=lockdown_mode_enabled&value=…`; the switch
    /// moves at once and springs back if the write fails.
    pub fn set_lockdown(&mut self, on: bool, cx: &mut Context<Self>) {
        let previous = self.lockdown;
        self.lockdown = Some(on);
        let path =
            format!("/settings/account_user_setting?feature=lockdown_mode_enabled&value={on}");
        backend::request(
            Call::new("PATCH", path, None),
            cx,
            move |this, result, _| {
                this.message = Some(if result.is_ok_and(|r| r.ok()) {
                    let message = if on {
                        "Lockdown mode turned on"
                    } else {
                        "Lockdown mode turned off"
                    };
                    (message.into(), false)
                } else {
                    this.lockdown = previous;
                    ("We couldn't update lockdown mode. Try again.".into(), true)
                });
            },
        );
        cx.notify();
    }

    fn open(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        let field = |label: &'static str, cx: &mut Context<Self>| {
            let editor = cx.new(|cx| {
                let mut editor = Composer::settings_compact_field("", "", cx).field_label(label);
                editor.set_secret(true, cx);
                editor
            });
            cx.observe(&editor, |_, _, cx| cx.notify()).detach();
            editor
        };
        let (first, second) = match step {
            Step::Current => ("Current PIN", "Confirm new PIN"),
            _ => ("New PIN", "Confirm new PIN"),
        };
        let first = field(first, cx);
        let second = field(second, cx);
        window.focus(&first.focus_handle(cx), cx);
        self.dialog = Some(Dialog {
            step,
            first,
            second,
            current: String::new(),
            error: None,
        });
        self.message = None;
        cx.notify();
    }

    /// The dialog's primary button: verify the current PIN, or send the new one.
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        if self.busy {
            return;
        }
        let first = dialog.first.read(cx).field_text().to_owned();
        let second = dialog.second.read(cx).field_text().to_owned();
        let (call, step) = match dialog.step {
            Step::Reset => return,
            Step::Current => {
                if !four_digits(&first) {
                    dialog.error = Some("Enter a four-digit PIN");
                    return cx.notify();
                }
                dialog.current = first.clone();
                let body = json!({"pin": first});
                (
                    Call::new("POST", "/locked_chats/unlock", Some(body)),
                    Step::Current,
                )
            }
            step => {
                if let Err(error) = new_pin(&first, &second) {
                    dialog.error = Some(error);
                    return cx.notify();
                }
                let call = if step == Step::Create {
                    let body = json!({"pin": first, "pin_confirmation": second});
                    Call::new("POST", "/locked_chats/unlock", Some(body))
                } else {
                    let body = json!({"current_pin": dialog.current, "new_pin": first});
                    Call::new("POST", "/locked_chats/pin/change", Some(body))
                };
                (call, step)
            }
        };
        self.busy = true;
        dialog.error = None;
        backend::request(call, cx, move |this, result, cx| {
            this.busy = false;
            let Some(dialog) = &mut this.dialog else {
                return;
            };
            match result {
                Ok(response) if response.ok() => {
                    if step == Step::Current {
                        for editor in [&dialog.first, &dialog.second] {
                            editor.update(cx, |editor, cx| editor.set_text("", cx));
                        }
                        dialog
                            .first
                            .update(cx, |editor, cx| editor.set_field_label("New PIN", cx));
                        dialog.step = Step::New;
                    } else {
                        this.dialog = None;
                        this.message = Some((
                            if step == Step::Create {
                                "Account PIN created".into()
                            } else {
                                "Account PIN changed".into()
                            },
                            false,
                        ));
                        this.load_pin(cx);
                    }
                }
                Ok(response) => {
                    dialog.error = Some(match response.error_code() {
                        Some("invalid_pin") => "Incorrect PIN. Try again.",
                        _ => FAILED,
                    });
                    if response.error_code() == Some("pin_locked") {
                        this.dialog = None;
                    }
                    if step == Step::New {
                        dialog_reset(this, cx);
                    }
                    if matches!(response.error_code(), Some("invalid_pin" | "pin_locked")) {
                        this.load_pin(cx);
                    }
                }
                Err(_) => dialog.error = Some(FAILED),
            }
        });
        cx.notify();
    }

    fn pin_rows(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let rows = match &self.pin {
            Pin::Loading => vec![row_status("Loading...", t)],
            Pin::Failed => vec![settings_row_content(
                "Account PIN",
                Some(status("Couldn’t load your PIN settings.", true, t).into_any_element()),
                action(
                    "account-pin-retry",
                    "Check again",
                    Kind::Secondary,
                    false,
                    t,
                )
                .on_click(cx.listener(|this, _, _, cx| this.load_pin(cx)))
                .into_any_element(),
                t,
            )],
            Pin::Ready { retry_after, .. } if *retry_after > 0 => vec![settings_row_content(
                "Account PIN",
                Some(status(locked(*retry_after), true, t).into_any_element()),
                action(
                    "account-pin-check",
                    "Check again",
                    Kind::Secondary,
                    false,
                    t,
                )
                .on_click(cx.listener(|this, _, _, cx| this.load_pin(cx)))
                .into_any_element(),
                t,
            )],
            Pin::Ready { has_pin: false, .. } => vec![settings_row_content(
                "Create account PIN",
                Some(paragraph(
                    "Create a PIN to lock selected chats and hide them from your sidebar and search. Once locked, new messages in that chat won’t be used to personalize regular chats.",
                    t,
                )),
                action(
                    "account-pin-create",
                    "Create a PIN",
                    Kind::Secondary,
                    false,
                    t,
                )
                .on_click(cx.listener(|this, _, window, cx| this.open(Step::Create, window, cx)))
                .into_any_element(),
                t,
            )],
            Pin::Ready { has_pin: true, .. } => vec![
                settings_row_content(
                    "Change PIN",
                    None,
                    action("account-pin-change", "Change", Kind::Secondary, false, t)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open(Step::Current, window, cx)),
                        )
                        .into_any_element(),
                    t,
                ),
                settings_row_content(
                    "Reset PIN",
                    Some(paragraph(
                        "Resetting your PIN deletes all locked chats protected by your current PIN",
                        t,
                    )),
                    action("account-pin-reset", "Reset", Kind::Danger, false, t)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open(Step::Reset, window, cx)),
                        )
                        .into_any_element(),
                    t,
                ),
            ],
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(text("Account PIN", 16., 22., 500., t.text))
            .child(paragraph("Control how you access your locked chats", t))
            .child(card(rows, Some(&self.layer), t))
    }

    fn dialog_view(&self, dialog: &Dialog, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let cancel = action("account-pin-cancel", "Cancel", Kind::Secondary, false, t).on_click(
            cx.listener(|this, _, _, cx| {
                this.dialog = None;
                cx.notify();
            }),
        );
        let field = |label: &'static str, editor: &Entity<Composer>| {
            div()
                .debug_selector(move || format!("account-pin-field-{label}"))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(text(label, 13., 18., 500., t.text))
                .child(editor.clone())
        };
        let (title, submit) = match dialog.step {
            Step::Create => ("Create account PIN", "Create PIN"),
            Step::Current => ("Current PIN", "Continue"),
            Step::New => ("Create new PIN", "Change PIN"),
            Step::Reset => ("Reset PIN", ""),
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(text(title, 16., 22., 500., t.text));
        let body = if dialog.step == Step::Reset {
            body.child(paragraph("Before resetting, check that the selected ChatGPT account in your browser is the account whose locked chats you want to delete. The browser may be signed in to a different account.", t))
                .child(
                    div().flex().gap(px(8.)).child(cancel).child(
                        action("account-pin-open-browser", "Open Account PIN in browser", Kind::Primary, false, t)
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.open_url(RESET_URL);
                                this.dialog = None;
                                cx.notify();
                            })),
                    ),
                )
        } else {
            let first = if dialog.step == Step::Current {
                "Current PIN"
            } else {
                "New PIN"
            };
            body.child(field(first, &dialog.first))
                .when(dialog.step != Step::Current, |d| {
                    d.child(field("Confirm new PIN", &dialog.second))
                })
                .children(dialog.error.map(|error| status(error, true, t)))
                .child(
                    div().flex().gap(px(8.)).child(cancel).child(
                        action("account-pin-submit", submit, Kind::Primary, self.busy, t)
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
                )
        };
        div().child(card(vec![body.p(px(16.))], Some(&self.layer), t))
    }
}

impl Render for Security {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let pin = match &self.dialog {
            Some(dialog) => self.dialog_view(dialog, &t, cx),
            None => self.pin_rows(&t, cx),
        };
        let lockdown = self.lockdown.unwrap_or(false);
        let toggle = switch("lockdown-mode", lockdown, self.lockdown.is_none(), &t)
            .debug_selector(|| "lockdown-mode".into())
            .when(self.lockdown.is_some(), |s| {
                s.on_click(cx.listener(move |this, _, _, cx| this.set_lockdown(!lockdown, cx)))
            })
            .into_any_element();
        let lockdown_row = settings_row_content(
            "Lockdown mode",
            Some(paragraph(
                "Helps protect sensitive data from prompt-injection attacks by limiting features that can connect to the web or external services.",
                &t,
            )),
            toggle,
            &t,
        );
        let message = self
            .message
            .clone()
            .map(|(message, error)| status(message, error, &t));
        frame(
            "security-settings-page",
            "Security",
            Vec::new(),
            div()
                .flex()
                .flex_col()
                .gap(px(24.))
                .child(pin)
                .child(card(vec![lockdown_row], Some(&self.layer), &t))
                .children(message),
            None,
            &self.layer,
            window,
            &t,
        )
    }
}

/// Clears both new-PIN fields after a refused change, like the original's `change-new` reset.
fn dialog_reset(this: &mut Security, cx: &mut Context<Security>) {
    if let Some(dialog) = &this.dialog {
        for editor in [&dialog.first, &dialog.second] {
            editor.update(cx, |editor, cx| editor.set_text("", cx));
        }
    }
}

fn row_status(s: &'static str, t: &Theme) -> gpui::Div {
    div().px(px(16.)).py(px(12.)).child(status(s, false, t))
}

fn four_digits(pin: &str) -> bool {
    pin.len() == 4 && pin.bytes().all(|b| b.is_ascii_digit())
}

/// The new PIN and its confirmation, checked like the original's dialog.
fn new_pin(pin: &str, confirmation: &str) -> Result<(), &'static str> {
    if !four_digits(pin) {
        Err("Enter a four-digit PIN")
    } else if pin != confirmation {
        Err("PINs don’t match. Try again.")
    } else {
        Ok(())
    }
}

/// `Too many attempts. Try again in N minutes.`, rounding the wait up to whole minutes.
fn locked(seconds: u64) -> String {
    let minutes = seconds.div_ceil(60).max(1);
    let unit = if minutes == 1 { "minute" } else { "minutes" };
    format!("Too many attempts. Try again in {minutes} {unit}.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_pin_needs_four_matching_digits() {
        assert_eq!(new_pin("1234", "1234"), Ok(()));
        assert_eq!(new_pin("123", "123"), Err("Enter a four-digit PIN"));
        assert_eq!(new_pin("12a4", "12a4"), Err("Enter a four-digit PIN"));
        assert_eq!(new_pin("1234", "4321"), Err("PINs don’t match. Try again."));
    }

    #[test]
    fn lockout_rounds_up_to_minutes() {
        assert_eq!(locked(1), "Too many attempts. Try again in 1 minute.");
        assert_eq!(locked(60), "Too many attempts. Try again in 1 minute.");
        assert_eq!(locked(61), "Too many attempts. Try again in 2 minutes.");
    }
}
