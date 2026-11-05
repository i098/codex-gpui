//! Settings ▸ Trusted contact (`ChatGptTrustedContactSettings`): the ChatGPT backend's
//! `/trusted_contact` service. Invite one contact (name, phone, email), see whether the
//! invitation is pending or active, resend it, or remove the contact.
use super::{
    Layer,
    account_page::{Kind, action, checkbox, frame, paragraph, status},
    backend::{self, Call, Response},
    settings_row_content,
    worktrees::card,
};
use crate::{
    composer::Composer,
    confirmation::{Confirmation, Outcome},
    text,
};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, prelude::FluentBuilder as _, px,
};
use serde::Deserialize;
use serde_json::{Value, json};
use theme::{ActiveTheme as _, Theme};

const CONSENT: &str = "I confirm this person and I are 18 or older. If this person agrees to be my trusted contact, I understand and agree that OpenAI may notify them in the future if I discuss suicide with ChatGPT in a way that indicates a serious safety concern, and I agree to my data being used for this purpose. I understand this system can make mistakes and that this feature is not an emergency service or a substitute for professional care.";
const SAVE_ERROR: &str = "Unable to save trusted contact. Please try again.";

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Contact {
    pub trusted_contact_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone_number: Option<String>,
    #[serde(default)]
    pub status: String,
}

impl Contact {
    fn name(&self) -> &str {
        self.name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or("Trusted contact")
    }

    fn pending(&self) -> bool {
        self.status == "pending"
    }

    /// The resend body (`o` in `chatgpt_trusted_contact`); `None` without an email.
    fn invitation(&self) -> Option<Value> {
        let email = self
            .email
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())?;
        Some(json!({
            "name": self.name(),
            "email": email,
            "phone_number": self.phone_number,
            "relationship": null,
        }))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Load {
    Loading,
    Failed,
    /// `GET /trusted_contact/enabled` answered `false`: the description only.
    Disabled,
    Ready(Option<Contact>),
}

struct Form {
    name: Entity<Composer>,
    phone: Entity<Composer>,
    email: Entity<Composer>,
    consent: bool,
    email_invalid: bool,
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("trusted-contact-settings", cx, TrustedContact::new)
        .into_any_element()
}

pub struct TrustedContact {
    pub load: Load,
    form: Option<Form>,
    pub message: Option<(SharedString, bool)>,
    busy: bool,
    confirmation: Option<Entity<Confirmation>>,
    layer: Layer,
}

impl TrustedContact {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let page = cx.entity();
        window.defer(cx, move |_, cx| page.update(cx, Self::reload));
        Self {
            load: Load::Loading,
            form: None,
            message: None,
            busy: false,
            confirmation: None,
            layer: Layer::default(),
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.load, Load::Ready(_)) {
            self.load = Load::Loading;
        }
        let call = Call::new("GET", "/trusted_contact/enabled", None);
        backend::request(call, cx, |this, result, cx| match result {
            Ok(response) if response.ok() => {
                if response.body["enabled"] == true {
                    this.load_list(cx);
                } else {
                    this.load = Load::Disabled;
                }
            }
            _ => this.load = Load::Failed,
        });
    }

    fn load_list(&mut self, cx: &mut Context<Self>) {
        let call = Call::new("GET", "/trusted_contact", None);
        backend::request(call, cx, |this, result, _| {
            this.load = match result {
                Ok(response) if response.ok() => {
                    match serde_json::from_value::<Vec<Contact>>(response.body["contacts"].clone())
                    {
                        Ok(contacts) => Load::Ready(contacts.into_iter().next()),
                        Err(_) => Load::Failed,
                    }
                }
                _ => Load::Failed,
            };
        });
    }

    fn open_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = |placeholder, label, cx: &mut Context<Self>| {
            cx.new(|cx| Composer::settings_compact_field(placeholder, "", cx).field_label(label))
        };
        let name = field("Enter your trusted contact's full name", "Name", cx);
        let phone = field("Phone number", "Phone", cx);
        let email = field("contact@example.com", "Email", cx);
        for editor in [&name, &phone, &email] {
            cx.observe(editor, |_, _, cx| cx.notify()).detach();
        }
        window.focus(&name.focus_handle(cx), cx);
        self.form = Some(Form {
            name,
            phone,
            email,
            consent: false,
            email_invalid: false,
        });
        self.message = None;
        cx.notify();
    }

    /// `POST /trusted_contact`; a blocked email or phone (400) is the original's
    /// `Couldn’t add trusted contact` state.
    pub fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else {
            return;
        };
        let name = form.name.read(cx).field_text().trim().to_owned();
        let email = form.email.read(cx).field_text().trim().to_owned();
        let phone = form.phone.read(cx).field_text().trim().to_owned();
        form.email_invalid = !valid_email(&email);
        if self.busy || name.is_empty() || !form.consent || form.email_invalid {
            cx.notify();
            return;
        }
        self.busy = true;
        self.message = None;
        let body = json!({
            "name": name,
            "email": email,
            "phone_number": (!phone.is_empty()).then_some(phone),
            "relationship": null,
        });
        let call = Call::new("POST", "/trusted_contact", Some(body));
        backend::request(call, cx, |this, result, cx| {
            this.busy = false;
            match result {
                Ok(response) if response.ok() => {
                    if let Ok(contact) = serde_json::from_value(response.body) {
                        this.load = Load::Ready(Some(contact));
                    }
                    this.form = None;
                    this.message = Some(("Invitation sent".into(), false));
                    this.load_list(cx);
                }
                Ok(response) if blocked(&response) => {
                    this.message = Some((
                        "Couldn’t add trusted contact. This can happen for different reasons — for example, they may not be eligible.".into(),
                        true,
                    ));
                }
                _ => this.message = Some((SAVE_ERROR.into(), true)),
            }
        });
        cx.notify();
    }

    /// `PUT /trusted_contact/{id}` with the contact's invitation.
    fn resend(&mut self, contact: Contact, cx: &mut Context<Self>) {
        let Some(body) = contact.invitation() else {
            return;
        };
        self.busy = true;
        let path = format!("/trusted_contact/{}", contact.trusted_contact_id);
        backend::request(
            Call::new("PUT", path, Some(body)),
            cx,
            |this, result, cx| {
                this.busy = false;
                this.message = Some(match result {
                    Ok(response) if response.ok() => {
                        this.load_list(cx);
                        ("Invitation resent.".into(), false)
                    }
                    _ => (SAVE_ERROR.into(), true),
                });
            },
        );
        cx.notify();
    }

    fn confirm_remove(&mut self, contact: Contact, window: &mut Window, cx: &mut Context<Self>) {
        let description = format!(
            "You’re about to remove {} as your trusted contact.",
            contact.name()
        );
        let dialog = cx.new(|cx| {
            Confirmation::new("Are you sure?", description, "Remove contact", window, cx)
                .destructive()
                .settings_style()
                .cancel_label("Go back")
        });
        cx.subscribe(&dialog, move |this, _, outcome, cx| {
            if *outcome == Outcome::Confirm {
                this.remove(&contact, cx);
            }
            this.confirmation = None;
            cx.notify();
        })
        .detach();
        window.focus(&dialog.focus_handle(cx), cx);
        self.confirmation = Some(dialog);
        cx.notify();
    }

    /// `DELETE /trusted_contact/{id}`.
    pub fn remove(&mut self, contact: &Contact, cx: &mut Context<Self>) {
        self.busy = true;
        let path = format!("/trusted_contact/{}", contact.trusted_contact_id);
        backend::request(Call::new("DELETE", path, None), cx, |this, result, cx| {
            this.busy = false;
            this.message = Some(match result {
                Ok(response) if response.ok() => {
                    this.load = Load::Ready(None);
                    this.load_list(cx);
                    ("Trusted contact removed".into(), false)
                }
                _ => (
                    "Unable to remove trusted contact. Please try again.".into(),
                    true,
                ),
            });
        });
        cx.notify();
    }

    fn form_view(&self, form: &Form, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let ready = !self.busy
            && form.consent
            && !form.name.read(cx).field_text().trim().is_empty()
            && !form.email.read(cx).field_text().trim().is_empty();
        let field = |label: &'static str, editor: &Entity<Composer>| {
            div()
                .debug_selector(move || format!("trusted-contact-field-{label}"))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(text(label, 13., 18., 500., t.text))
                .child(editor.clone())
        };
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(text("Invite a trusted contact", 16., 22., 500., t.text))
            .child(paragraph("Choose a friend, family member, or another trusted adult who is readily available to offer support. We’ll invite them to be your trusted contact, and they can choose whether to participate.", t))
            .child(field("Name", &form.name))
            .child(field("Phone", &form.phone))
            .child(field("Email", &form.email))
            .when(form.email_invalid, |d| {
                d.child(status("Please enter a valid email address.", true, t))
            })
            .child(
                checkbox("trusted-contact-consent", form.consent, CONSENT, t).on_click(
                    cx.listener(|this, _, _, cx| {
                        if let Some(form) = &mut this.form {
                            form.consent = !form.consent;
                            cx.notify();
                        }
                    }),
                ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        action("trusted-contact-cancel-form", "Cancel", Kind::Secondary, false, t).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.form = None;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        action("trusted-contact-send", "Send invitation", Kind::Primary, !ready, t)
                            .when(ready, |b| {
                                b.on_click(cx.listener(|this, _, _, cx| this.submit(cx)))
                            }),
                    ),
            )
    }

    fn contact_view(&self, contact: &Contact, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let detail = div()
            .flex()
            .flex_col()
            .children(
                [contact.email.clone(), contact.phone_number.clone()]
                    .into_iter()
                    .flatten()
                    .map(|line| text(line, 12., 16., 430., t.text_secondary)),
            )
            .into_any_element();
        let badge = if contact.pending() {
            ("Pending", t.text_secondary)
        } else {
            ("Active", t.text_success)
        };
        let row = settings_row_content(
            contact.name().to_owned(),
            Some(detail),
            text(badge.0, 12., 16., 500., badge.1).into_any_element(),
            t,
        );
        let resend = contact.pending() && contact.invitation().is_some();
        let remove_label = if contact.pending() {
            "Cancel invitation"
        } else {
            "Remove contact"
        };
        let (resend_contact, remove_contact) = (contact.clone(), contact.clone());
        div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(card(vec![row], Some(&self.layer), t))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .when(resend, |d| {
                        d.child(
                            action(
                                "trusted-contact-resend",
                                "Resend invitation",
                                Kind::Secondary,
                                self.busy,
                                t,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if !this.busy {
                                        this.resend(resend_contact.clone(), cx);
                                    }
                                },
                            )),
                        )
                    })
                    .child(
                        action(
                            "trusted-contact-remove",
                            remove_label,
                            Kind::Danger,
                            self.busy,
                            t,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                if !this.busy {
                                    this.confirm_remove(remove_contact.clone(), window, cx);
                                }
                            },
                        )),
                    ),
            )
    }
}

impl Render for TrustedContact {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let content = match (&self.load, &self.form) {
            (Load::Loading, _) => div().child(status("Loading trusted contact...", false, &t)),
            (Load::Failed, _) => div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(12.))
                .child(status(
                    "Couldn't load trusted contact. Please try again.",
                    false,
                    &t,
                ))
                .child(
                    action("trusted-contact-retry", "Retry", Kind::Secondary, false, &t)
                        .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                ),
            (Load::Disabled, _) => div(),
            (Load::Ready(_), Some(form)) => self.form_view(form, &t, cx),
            (Load::Ready(Some(contact)), None) => self.contact_view(&contact.clone(), &t, cx),
            (Load::Ready(None), None) => div().flex().child(
                action(
                    "add-trusted-contact-button",
                    "Add contact",
                    Kind::Secondary,
                    false,
                    &t,
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_form(window, cx))),
            ),
        };
        let message = self
            .message
            .clone()
            .map(|(message, error)| status(message, error, &t));
        frame(
            "trusted-contact-page",
            "Trusted contact",
            vec![
                paragraph(
                    "Having a trusted contact can make it easier to get support from someone who knows you well.",
                    &t,
                ),
                paragraph(
                    "In the future, if you discuss suicide with ChatGPT in a way that indicates a serious safety concern, we may automatically notify your trusted contact so they can check in with you. They must be 18+ to participate.",
                    &t,
                ),
            ],
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .child(content)
                .children(message),
            self.confirmation.as_ref(),
            &self.layer,
            window,
            &t,
        )
    }
}

/// A `POST /trusted_contact` refusal for a blocked email or phone number (`st`).
fn blocked(response: &Response) -> bool {
    response.status == 400
        && matches!(
            response.error_code(),
            Some("trusted-contacts-blocked-email" | "trusted-contacts-blocked-phone-number")
        )
}

/// The original's `et` email pattern: a dot-atom-like local part, `@`, then two or more
/// hostname labels of 1–63 letters, digits or inner hyphens.
pub(super) fn valid_email(email: &str) -> bool {
    const LOCAL: &str = ".!#$%&'*+/=?^_`{|}~-";
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    let label = |l: &str| {
        (1..=63).contains(&l.len())
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !l.starts_with('-')
            && !l.ends_with('-')
    };
    !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || LOCAL.contains(c))
        && domain.split('.').count() >= 2
        && domain.split('.').all(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_follows_the_original_pattern() {
        for good in ["a@b.co", "first.last+tag@mail.example.com", "x_y@a-b.io"] {
            assert!(valid_email(good), "{good}");
        }
        for bad in [
            "", "a@b", "@b.co", "a@@b.co", "a@-b.co", "a@b-.co", "a b@c.co", "a@b..co", "a@b.co.",
        ] {
            assert!(!valid_email(bad), "{bad}");
        }
    }

    #[test]
    fn only_blocked_codes_are_unavailable() {
        let response = |status, code: &str| Response {
            status,
            body: json!({"detail": {"error_code": code}}),
        };
        assert!(blocked(&response(400, "trusted-contacts-blocked-email")));
        assert!(blocked(&response(
            400,
            "trusted-contacts-blocked-phone-number"
        )));
        assert!(!blocked(&response(400, "other")));
        assert!(!blocked(&response(500, "trusted-contacts-blocked-email")));
    }
}
