//! Settings ▸ Parental controls (`ChatGptParentalControlsSettings`): the ChatGPT backend's
//! `/amphora` family service. A personal account sees its family members, invites a parent or
//! a teen, opens a pending invite, resends or cancels an invitation, and unlinks accounts.
use super::{
    Layer,
    account_page::{Kind, action, frame, paragraph, status},
    backend::{self, Call, Response},
    notifications::{self, Account},
    on_card, settings_row_content,
    subtrees::OpenSection,
    worktrees::card,
};
use crate::{
    composer::Composer,
    confirmation::{Confirmation, Outcome},
    icon::icon,
    text,
};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, FontWeight, HighlightStyle,
    InteractiveElement, InteractiveText, IntoElement, MouseButton, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, StyledText, Window, deferred, div,
    prelude::FluentBuilder as _, px, rgba,
};
use serde::Deserialize;
use serde_json::{Value, json};
use theme::{ActiveTheme as _, Theme};

const FAQ: &str = "https://help.openai.com/articles/12315553-parental-controls-on-chatgpt-faq";
const ACTION_ERROR: &str = "Something went wrong. Try again.";

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
struct Group {
    id: Option<String>,
    role: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
struct Member {
    user_id: Option<String>,
    name: Option<String>,
    email: Option<String>,
    account_identifier: Option<String>,
    role: String,
    status: String,
}

impl Member {
    fn invited(&self) -> bool {
        self.status == "invited"
    }

    fn identifier(&self) -> Option<&str> {
        self.email
            .as_deref()
            .or(self.account_identifier.as_deref())
            .filter(|s| !s.is_empty())
    }

    /// The name, email or account identifier, else the original's fallback label.
    fn label(&self, fallback: &'static str) -> String {
        self.name
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(self.identifier())
            .unwrap_or(if self.invited() {
                "[missing]"
            } else {
                fallback
            })
            .to_owned()
    }
}

/// An unanswered `family-invite` notification.
#[derive(Clone, Debug, PartialEq)]
struct Invite {
    notification_id: String,
    family_group_id: String,
    inviter: String,
    /// `owner`: the inviter is the teen, and accepting makes this account the parent.
    role: String,
}

#[derive(Clone, Debug, PartialEq)]
enum Family {
    Loading,
    Failed,
    Ready {
        group: Group,
        members: Option<Vec<Member>>,
        invite: Option<Invite>,
    },
}

/// The graduation notices (`/amphora/u18_graduation_unlink_setting_notices`).
#[derive(Clone, Debug, Default, PartialEq)]
struct Notices {
    loading: bool,
    failed: bool,
    items: Vec<Value>,
    /// The ids of a dismissal in flight, or of the last one that failed.
    dismissing: Option<Vec<String>>,
    dismiss_failed: bool,
}

struct AddForm {
    /// `teen` or `parent`: the role of the invited person.
    role: Option<&'static str>,
    choose_role: bool,
    owner: bool,
    phone: bool,
    email_field: Entity<Composer>,
    phone_field: Entity<Composer>,
    /// The field text that failed validation; any edit clears it (`k(!1)` on change).
    invalid: Option<String>,
    error: Option<String>,
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("parental-controls-settings", cx, ParentalControls::new)
        .into_any_element()
}

pub struct ParentalControls {
    family: Family,
    /// A reload of loaded data failed: the original keeps the data under a refresh error.
    refresh_failed: bool,
    notices: Notices,
    /// The user the family was loaded for.
    loaded_for: Option<String>,
    generation: u64,
    /// The member whose page is open.
    detail: Option<Member>,
    form: Option<AddForm>,
    busy: bool,
    message: Option<(SharedString, bool)>,
    confirmation: Option<Entity<Confirmation>>,
    layer: Layer,
}

impl ParentalControls {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<notifications::State>(|this, cx| this.sync(cx))
            .detach();
        let page = cx.entity();
        window.defer(cx, move |_, cx| {
            notifications::dispatch(notifications::Action::RefreshAccount, cx);
            page.update(cx, Self::sync);
        });
        Self {
            family: Family::Loading,
            refresh_failed: false,
            notices: Notices::default(),
            loaded_for: None,
            generation: 0,
            detail: None,
            form: None,
            busy: false,
            message: None,
            confirmation: None,
            layer: Layer::default(),
        }
    }

    fn account(cx: &App) -> (Account, Option<String>) {
        cx.try_global::<notifications::State>()
            .map(|s| (s.account, s.user_id.clone()))
            .unwrap_or_default()
    }

    /// Loads the family once the personal-account gate opens for a new user.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let (account, user) = Self::account(cx);
        if account == Account::Personal && user.is_some() && user != self.loaded_for {
            self.loaded_for = user;
            self.family = Family::Loading;
            self.detail = None;
            self.load_notices(cx);
            self.reload(cx);
        }
        cx.notify();
    }

    fn me(&self) -> Option<&str> {
        self.loaded_for.as_deref()
    }

    /// `GET /amphora`, then the members of the family, or the pending invite without one.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        backend::request(
            Call::new("GET", "/amphora", None),
            cx,
            move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                let group = match result {
                    Ok(response) if response.ok() => {
                        serde_json::from_value::<Group>(response.body).unwrap_or_default()
                    }
                    _ => return this.fail(),
                };
                match group.id.clone() {
                    Some(id) => this.load_members(generation, group, id, cx),
                    None => this.find_invite(generation, group, None, Vec::new(), cx),
                }
            },
        );
    }

    fn fail(&mut self) {
        match self.family {
            Family::Ready { .. } => self.refresh_failed = true,
            _ => self.family = Family::Failed,
        }
    }

    fn ready(&mut self, group: Group, members: Option<Vec<Member>>, invite: Option<Invite>) {
        self.refresh_failed = false;
        self.family = Family::Ready {
            group,
            members,
            invite,
        };
    }

    fn load_members(&mut self, generation: u64, group: Group, id: String, cx: &mut Context<Self>) {
        let path = format!("/amphora/{}/members", encode(&id));
        backend::request(Call::new("GET", path, None), cx, move |this, result, _| {
            if this.generation != generation {
                return;
            }
            match result.ok().filter(Response::ok).map(|r| members(&r.body)) {
                Some(Some(members)) => this.ready(group, Some(members), None),
                _ => this.fail(),
            }
        });
    }

    /// Pages `/amphora/notifications` for the first unanswered `family-invite` whose family
    /// still exists; a family that answers 404 is skipped (`ze`).
    fn find_invite(
        &mut self,
        generation: u64,
        group: Group,
        cursor: Option<String>,
        unavailable: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let mut path = "/amphora/notifications?limit=20".to_owned();
        if let Some(cursor) = &cursor {
            path.push_str(&format!("&cursor={}", encode(cursor)));
        }
        backend::request(Call::new("GET", path, None), cx, move |this, result, cx| {
            if this.generation != generation {
                return;
            }
            let Some(body) = result.ok().filter(Response::ok).map(|r| r.body) else {
                return this.fail();
            };
            let found = body["items"].as_array().and_then(|items| {
                items
                    .iter()
                    .find(|item| {
                        item["notification_type"] == "family-invite"
                            && item["reacted_to_at"].is_null()
                            && item["payload"]["family_group_id"]
                                .as_str()
                                .is_none_or(|id| !unavailable.iter().any(|u| u == id))
                    })
                    .cloned()
            });
            let Some(item) = found else {
                return match body["cursor"].as_str() {
                    Some(next) => {
                        this.find_invite(generation, group, Some(next.into()), unavailable, cx)
                    }
                    None => this.ready(group, None, None),
                };
            };
            let payload = &item["payload"];
            let Some(family) = payload["family_group_id"]
                .as_str()
                .filter(|s| !s.is_empty())
            else {
                return this.fail();
            };
            let invite = Invite {
                notification_id: item["id"].as_str().unwrap_or_default().to_owned(),
                family_group_id: family.to_owned(),
                inviter: payload["inviter_name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or(payload["inviter_identifier"].as_str())
                    .unwrap_or_default()
                    .to_owned(),
                role: payload["role"].as_str().unwrap_or_default().to_owned(),
            };
            let path = format!("/amphora/{}/members", encode(family));
            backend::request(Call::new("GET", path, None), cx, move |this, result, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(response) if response.ok() => this.ready(group, None, Some(invite)),
                    Ok(response) if response.status == 404 => {
                        let mut unavailable = unavailable;
                        unavailable.push(invite.family_group_id);
                        this.find_invite(generation, group, cursor, unavailable, cx);
                    }
                    _ => this.fail(),
                }
            });
        });
    }

    fn load_notices(&mut self, cx: &mut Context<Self>) {
        self.notices.loading = true;
        let call = Call::new(
            "GET",
            "/amphora/u18_graduation_unlink_setting_notices",
            None,
        );
        backend::request(call, cx, |this, result, _| {
            this.notices.loading = false;
            match result {
                Ok(response) if response.ok() => {
                    this.notices.failed = false;
                    this.notices.items = response.body["notices"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                }
                _ => this.notices.failed = true,
            }
        });
    }

    fn dismiss_notices(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        if self.notices.dismissing.is_some() && !self.notices.dismiss_failed {
            return;
        }
        self.notices.dismissing = Some(ids.clone());
        self.notices.dismiss_failed = false;
        let call = Call::new(
            "POST",
            "/amphora/u18_graduation_unlink_setting_notices/dismiss",
            Some(json!({ "notice_ids": ids })),
        );
        backend::request(call, cx, move |this, result, _| match result {
            Ok(response) if response.ok() => {
                this.notices.dismissing = None;
                this.notices
                    .items
                    .retain(|n| !ids.iter().any(|id| n["id"] == id.as_str()));
            }
            _ => this.notices.dismiss_failed = true,
        });
        cx.notify();
    }

    fn group(&self) -> Option<&Group> {
        match &self.family {
            Family::Ready { group, .. } => Some(group),
            _ => None,
        }
    }

    fn role(&self) -> &str {
        self.group().and_then(|g| g.role.as_deref()).unwrap_or("")
    }

    fn members(&self) -> Option<&Vec<Member>> {
        match &self.family {
            Family::Ready { members, .. } => members.as_ref(),
            _ => None,
        }
    }

    fn open_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let owner = self.role() == "owner";
        let member = matches!(self.role(), "member_role" | "child");
        // `L`: a teen alone in its family can only add a parent.
        let only_parent = self.role() == "member_role"
            && self
                .members()
                .is_some_and(|m| m.iter().all(|m| m.user_id.as_deref() == self.me()));
        let field = |placeholder, cx: &mut Context<Self>| {
            let field = cx.new(|cx| Composer::settings_compact_field(placeholder, "", cx));
            cx.observe(&field, |this, field, cx| {
                let text = field.read(cx).field_text();
                if let Some(form) = &mut this.form
                    && form
                        .invalid
                        .as_deref()
                        .is_some_and(|rejected| rejected != text)
                {
                    form.invalid = None;
                }
                cx.notify();
            })
            .detach();
            field
        };
        let email_field = field("name@email.com", cx);
        let phone_field = field("Phone number", cx);
        window.focus(&email_field.focus_handle(cx), cx);
        self.form = Some(AddForm {
            role: if owner {
                Some("teen")
            } else if only_parent {
                Some("parent")
            } else {
                None
            },
            choose_role: !owner && !(member && only_parent),
            owner,
            phone: false,
            email_field,
            phone_field,
            invalid: None,
            error: None,
        });
        cx.notify();
    }

    /// The invite form's submit: validates the identifier, creates the family when there is
    /// none, then `POST /amphora/{id}/members`.
    fn submit_invite(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else {
            return;
        };
        let field = if form.phone {
            &form.phone_field
        } else {
            &form.email_field
        };
        let raw = field.read(cx).field_text().to_owned();
        let identifier = raw.trim().to_owned();
        let identifier = if form.phone {
            e164(&identifier)
        } else {
            super::trusted_contact::valid_email(&identifier).then_some(identifier)
        };
        let valid = identifier.is_some();
        let identifier = identifier.unwrap_or_default();
        form.invalid = (!valid).then_some(raw);
        let Some(role) = form.role.filter(|_| valid && !self.busy) else {
            cx.notify();
            return;
        };
        form.error = None;
        self.busy = true;
        let invite = move |_: &mut Self, id: String, cx: &mut Context<Self>| {
            let body = json!({"members": [{"account_identifier": identifier, "role": role}]});
            let path = format!("/amphora/{}/members", encode(&id));
            backend::request(
                Call::new("POST", path, Some(body)),
                cx,
                |this, result, cx| this.invited(result, cx),
            );
        };
        match self.group().and_then(|g| g.id.clone()) {
            Some(id) => invite(self, id, cx),
            None => {
                let my_role = if role == "parent" { "teen" } else { "parent" };
                let call = Call::new("POST", "/amphora", Some(json!({ "role": my_role })));
                backend::request(call, cx, move |this, result, cx| {
                    match result
                        .ok()
                        .filter(Response::ok)
                        .and_then(|r| r.body["id"].as_str().map(str::to_owned))
                    {
                        Some(id) => {
                            // Keep the new family, as the original does, so a failed invite
                            // retries the member POST instead of creating the family again.
                            if let Family::Ready { group, .. } = &mut this.family {
                                group.id = Some(id.clone());
                            }
                            invite(this, id, cx)
                        }
                        None => this.invited(
                            Err(anyhow::anyhow!("Family creation returned no group")),
                            cx,
                        ),
                    }
                });
            }
        }
        cx.notify();
    }

    fn invited(&mut self, result: anyhow::Result<Response>, cx: &mut Context<Self>) {
        self.busy = false;
        match result {
            Ok(response) if response.ok() => {
                self.form = None;
                self.reload(cx);
            }
            other => {
                if let Some(form) = &mut self.form {
                    form.error = Some(error_text(&other));
                }
            }
        }
    }

    /// The pending invite's dialog: Accept or Decline, then the notification is marked
    /// `reacted` (`POST /amphora/{id}/invite`).
    fn open_invite(&mut self, invite: Invite, window: &mut Window, cx: &mut Context<Self>) {
        let parent = invite.role == "owner";
        let title = if parent {
            format!("Are you {}'s parent or guardian?", invite.inviter)
        } else {
            format!("Add {} as your parent or guardian?", invite.inviter)
        };
        let body = if parent {
            "If you accept, you’ll be able to manage some of your teen’s settings, set time limits, and add safeguards that work for your family. You won’t have access to details about your teen’s conversations with ChatGPT, except in certain rare safety circumstances. You can unlink accounts anytime.".to_owned()
        } else {
            format!(
                "If you accept, {} will be able to manage some of your settings, set time limits, and add safeguards that work for your family. We won’t share details of your conversations with ChatGPT, except in certain rare safety circumstances. You can unlink accounts anytime.",
                invite.inviter
            )
        };
        let dialog = cx.new(|cx| {
            Confirmation::new(title, body, "Accept", window, cx)
                .settings_style()
                .alternate("Decline", window, cx)
                .with_documentation("Learn more", FAQ, cx)
        });
        cx.subscribe(&dialog, move |this, _, outcome, cx| {
            this.confirmation = None;
            match outcome {
                Outcome::Confirm => this.answer_invite(invite.clone(), "accept", cx),
                Outcome::Alternate => this.answer_invite(invite.clone(), "decline", cx),
                _ => {}
            }
            cx.notify();
        })
        .detach();
        self.confirmation = Some(dialog);
        cx.notify();
    }

    fn answer_invite(&mut self, invite: Invite, choice: &'static str, cx: &mut Context<Self>) {
        self.busy = true;
        self.message = None;
        let path = format!("/amphora/{}/invite", encode(&invite.family_group_id));
        let call = Call::new("POST", path, Some(json!({ "choice": choice })));
        backend::request(call, cx, move |this, result, cx| match result {
            Ok(response) if response.ok() => this.react(invite, Some(choice), cx),
            // A used or revoked invite: drop its notification so the card does not return.
            Ok(response) if response.status == 404 => this.react(invite, None, cx),
            _ => {
                this.busy = false;
                this.message = Some((ACTION_ERROR.into(), true));
            }
        });
        cx.notify();
    }

    /// `POST /amphora/notifications/{id}/reacted` after an answer (`choice`), or after the
    /// invite turned out to be gone (`None`), then the original's result note.
    fn react(&mut self, invite: Invite, choice: Option<&'static str>, cx: &mut Context<Self>) {
        let path = format!(
            "/amphora/notifications/{}/reacted",
            encode(&invite.notification_id)
        );
        backend::request(
            Call::new("POST", path, None),
            cx,
            move |this, result, cx| {
                this.busy = false;
                let dismissed = result.is_ok_and(|r| r.ok());
                this.message = Some(match (choice, dismissed) {
                    (None, _) => ("This invitation is no longer available".into(), true),
                    (Some("accept"), true) => ("Request accepted".into(), false),
                    (Some("accept"), false) => ("Invitation accepted. The notification couldn’t be dismissed. Retry, or close and return later.".into(), true),
                    (_, true) => ("Invitation update".into(), false),
                    (_, false) => ("Invitation declined. The notification couldn’t be dismissed. Retry, or close and return later.".into(), true),
                });
                this.reload(cx);
            },
        );
    }

    /// Resend: a confirmation, then `POST /amphora/{id}/members/resend` and the `Sent!` note.
    fn confirm_resend(&mut self, member: Member, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| {
            Confirmation::new(
                "Resend invitation",
                "We’ll send another invite to your family member. If they’re new to ChatGPT, they’ll be asked to create an account.",
                "Send",
                window,
                cx,
            )
            .settings_style()
        });
        cx.subscribe(&dialog, move |this, _, outcome, cx| {
            this.confirmation = None;
            if *outcome == Outcome::Confirm {
                this.invitation_call(member.clone(), true, cx);
            }
            cx.notify();
        })
        .detach();
        window.focus(&dialog.focus_handle(cx), cx);
        self.confirmation = Some(dialog);
        cx.notify();
    }

    /// `members/resend` or `remove` for an invited member (`se`).
    fn invitation_call(&mut self, member: Member, resend: bool, cx: &mut Context<Self>) {
        let Some(id) = self.group().and_then(|g| g.id.clone()) else {
            return;
        };
        let Some(identifier) = member.identifier().map(str::to_owned) else {
            return;
        };
        let (path, body) = if resend {
            (
                format!("/amphora/{}/members/resend", encode(&id)),
                json!({ "account_identifier": identifier }),
            )
        } else {
            (
                format!("/amphora/{}/remove", encode(&id)),
                match member.email.as_deref().filter(|e| !e.is_empty()) {
                    Some(email) => json!({ "email": email }),
                    None => json!({ "account_identifier": identifier }),
                },
            )
        };
        self.busy = true;
        self.message = None;
        backend::request(
            Call::new("POST", path, Some(body)),
            cx,
            move |this, result, cx| {
                this.busy = false;
                match result {
                    Ok(response) if response.ok() => {
                        if resend {
                            this.message = Some((
                                format!("Sent! We sent {identifier} a new invite. If they’re new to ChatGPT, they’ll be asked to create a new account.").into(),
                                false,
                            ));
                        } else {
                            this.detail = None;
                        }
                        this.reload(cx);
                    }
                    other => {
                        let verb = if resend { "resend" } else { "cancel" };
                        let error = error_text(&other);
                        this.message =
                            Some((format!("Unable to {verb} invite: {error}").into(), true));
                    }
                }
            },
        );
        cx.notify();
    }

    /// Unlink (`J`): a confirmation, then `POST /amphora/{id}/remove` with the user to remove.
    fn confirm_unlink(
        &mut self,
        member: Member,
        unlink_self: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = self.role() == "owner";
        let leaving = owner && member.role == "owner";
        let label = if leaving {
            "Remove from family"
        } else {
            "Unlink accounts"
        };
        let question = if leaving {
            "Are you sure you want to leave this family group?".to_owned()
        } else if unlink_self {
            "Are you sure you want to unlink your accounts?".to_owned()
        } else {
            match member.label("") {
                name if name.is_empty() || name == "[missing]" => {
                    "Remove this member from this family group?".to_owned()
                }
                name => format!("Remove {name} from this family group?"),
            }
        };
        let dialog = cx.new(|cx| {
            Confirmation::new(label, question, label, window, cx)
                .settings_style()
                .destructive()
        });
        let user = if unlink_self || leaving {
            self.me().map(str::to_owned)
        } else {
            member.user_id.clone()
        };
        cx.subscribe(&dialog, move |this, _, outcome, cx| {
            this.confirmation = None;
            if *outcome == Outcome::Confirm
                && let Some(user) = user.clone()
            {
                this.unlink(user, cx);
            }
            cx.notify();
        })
        .detach();
        window.focus(&dialog.focus_handle(cx), cx);
        self.confirmation = Some(dialog);
        cx.notify();
    }

    fn unlink(&mut self, user_id: String, cx: &mut Context<Self>) {
        let Some(id) = self.group().and_then(|g| g.id.clone()) else {
            return;
        };
        self.busy = true;
        self.message = None;
        let path = format!("/amphora/{}/remove", encode(&id));
        let call = Call::new("POST", path, Some(json!({ "user_id": user_id })));
        backend::request(call, cx, |this, result, cx| {
            this.busy = false;
            match result {
                Ok(response) if response.ok() => {
                    this.detail = None;
                    this.reload(cx);
                }
                _ => this.message = Some((ACTION_ERROR.into(), true)),
            }
        });
        cx.notify();
    }

    fn open_member(&mut self, member: Member, cx: &mut Context<Self>) {
        self.detail = Some(member);
        self.message = None;
        cx.notify();
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        self.detail = None;
        self.message = None;
        cx.notify();
    }
}

/// `encodeURIComponent` for a path segment or query value.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn members(body: &Value) -> Option<Vec<Member>> {
    serde_json::from_value(body["items"].clone()).ok()
}

/// The backend's error message for the original's `{error}` slot.
fn error_text(result: &anyhow::Result<Response>) -> String {
    match result {
        Ok(response) => response.body["detail"]
            .as_str()
            .or(response.body["detail"]["message"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("HTTP {}", response.status)),
        Err(error) => error.to_string(),
    }
}

/// The number in E.164 form (`+14155550100`), as the original's phone input sends it.
/// shortcut: digits-only shape check, not libphonenumber's per-region `isValid`;
/// upgrade if the backend starts rejecting numbers this accepts.
fn e164(s: &str) -> Option<String> {
    let digits: String = s
        .strip_prefix('+')?
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '(' | ')' | '.'))
        .collect();
    ((8..=15).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("+{digits}"))
}

/// A wrapping paragraph whose `<a>…</a>` span opens `url`.
fn linked(
    id: &'static str,
    s: impl Into<String>,
    url: SharedString,
    size: f32,
    color: gpui::Hsla,
    t: &Theme,
) -> gpui::Div {
    let s = s.into();
    let (plain, link) = match s.split_once("<a>") {
        Some((before, rest)) => {
            let (label, after) = rest.split_once("</a>").unwrap_or((rest, ""));
            let start = before.len();
            (
                format!("{before}{label}{after}"),
                Some(start..start + label.len()),
            )
        }
        None => (s.clone(), None),
    };
    let styled = StyledText::new(plain).with_highlights(link.clone().map(|r| {
        (
            r,
            HighlightStyle {
                color: Some(t.text_accent),
                ..Default::default()
            },
        )
    }));
    div()
        .text_size(px(size))
        .line_height(px(size * 1.5))
        .font_weight(FontWeight(430.))
        .text_color(color)
        .child(
            InteractiveText::new(id, styled)
                .on_click(link.into_iter().collect(), move |_, _, cx| {
                    cx.open_url(&url)
                }),
        )
}

/// The original's settings row (`x`) holding only a label or a spinner, full pane width.
fn bare(row: gpui::Div) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(row)
        .into_any_element()
}

fn loading_row(t: &Theme) -> gpui::Div {
    div().child(
        div()
            .id("parental-controls-loading")
            .role(gpui::Role::Status)
            .px(px(16.))
            .py(px(12.))
            .child(icon("sidebar-cmd-spinner", 12., t.text_secondary)),
    )
}

fn retry_row<V: 'static>(
    message: &'static str,
    id: &'static str,
    t: &Theme,
    cx: &mut Context<V>,
    on_retry: impl Fn(&mut V, &mut Context<V>) + 'static,
) -> gpui::Div {
    // The original's load-error row: 13 px text and a borderless toolbar-size Retry.
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(24.))
        .px(px(16.))
        .py(px(12.))
        .child(text(message, 13., 18., 400., t.text))
        .child(
            div()
                .id(id)
                .debug_selector(move || id.into())
                .h(px(28.))
                .px(px(9.))
                .flex()
                .flex_none()
                .items_center()
                .relative()
                .child(on_card(
                    8.,
                    shapes::Corner::Squircle,
                    t.button_secondary,
                    None,
                ))
                .child(text("Retry", 13., 18., 400., t.text))
                .on_click(cx.listener(move |this, _, _, cx| on_retry(this, cx))),
        )
}

fn avatar(member: &Member, t: &Theme) -> gpui::Div {
    let initial = member
        .label("Family member")
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    div()
        .size(px(32.))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(t.list_hover)
        .child(text(initial, 13., 18., 500., t.text))
}

/// The member's name, avatar and email, or `Invitation sent` (`G`).
fn profile(member: &Member, t: &Theme) -> gpui::Div {
    let detail = if member.invited() {
        "Invitation sent".to_owned()
    } else {
        member.identifier().unwrap_or_default().to_owned()
    };
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(16.))
        .py(px(12.))
        .child(avatar(member, t))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(text(member.label("Member"), 13., 18.571_428, 500., t.text))
                .when(!detail.is_empty(), |d| {
                    d.child(text(detail, 12., 16., 430., t.text_secondary))
                }),
        )
        .when(member.role == "owner", |d| {
            d.child(text("Parent", 13., 18., 430., t.text_secondary))
        })
}

impl ParentalControls {
    fn notices_view(&self, t: &Theme, cx: &mut Context<Self>) -> Vec<gpui::Div> {
        let mut rows = Vec::new();
        let pending = self.notices.dismissing.is_some() && !self.notices.dismiss_failed;
        if self.notices.loading || pending {
            let label = if pending {
                "Dismissing account-linking updates…"
            } else {
                "Loading account-linking updates…"
            };
            rows.push(status(label, false, t));
        }
        if self.notices.failed {
            rows.push(retry_row(
                "Updates about parental controls couldn’t be loaded",
                "parental-controls-notices-retry",
                t,
                cx,
                |this, cx| this.load_notices(cx),
            ));
        }
        for recipient in ["teen", "parent"] {
            // Hide a dismissal only while it is in flight; a failed one shows its notices again.
            let dismissing = match &self.notices.dismissing {
                Some(ids) if pending => ids.clone(),
                _ => Vec::new(),
            };
            let group: Vec<&Value> = self
                .notices
                .items
                .iter()
                .filter(|n| {
                    n["recipient_type"] == recipient
                        && !dismissing.iter().any(|id| n["id"] == id.as_str())
                })
                .collect();
            let Some(first) = group.first() else {
                continue;
            };
            let names: Vec<&str> = group
                .iter()
                .filter_map(|n| n["teen_display_name"].as_str())
                .collect();
            let names = format_list(&names);
            let mut body = if recipient == "teen" {
                "Parental controls have been removed because you’re no longer using ChatGPT’s teen experience".to_owned()
            } else if group.len() == 1 {
                format!(
                    "You’re no longer listed as {names}’s parent or guardian. Their account has been updated based on their age."
                )
            } else {
                format!(
                    "You’re no longer listed as the parent or guardian for {names}. Their accounts have been updated based on their ages."
                )
            };
            let url = first["learn_more_url"].as_str().map(str::to_owned);
            if url.is_some() {
                body.push_str(if recipient == "teen" {
                    ". <a>Learn more</a>"
                } else {
                    " <a>Learn more</a>"
                });
            }
            let ids: Vec<String> = group
                .iter()
                .filter_map(|n| n["id"].as_str().map(str::to_owned))
                .collect();
            let id = if recipient == "teen" {
                "parental-controls-notice-teen"
            } else {
                "parental-controls-notice-parent"
            };
            rows.push(
                div()
                    .relative()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .p(px(12.))
                    .child(on_card(
                        12.,
                        shapes::Corner::Squircle,
                        t.background_panel,
                        Some(t.border),
                    ))
                    .child(div().flex_1().child(linked(
                        id,
                        body,
                        url.unwrap_or_default().into(),
                        13.,
                        t.text,
                        t,
                    )))
                    .child(
                        div()
                            .id(id)
                            .debug_selector(move || format!("{id}-close"))
                            .aria_label("Close")
                            .flex_none()
                            .child(icon("close", 16., t.text_secondary))
                            .when(!pending, |d| {
                                d.on_click(cx.listener(move |this, _, _, cx| {
                                    this.dismiss_notices(ids.clone(), cx)
                                }))
                            }),
                    ),
            );
        }
        if self.notices.dismiss_failed
            && let Some(ids) = self.notices.dismissing.clone()
        {
            rows.push(retry_row(
                "These updates couldn’t be dismissed. Please try again.",
                "parental-controls-notices-dismiss-retry",
                t,
                cx,
                move |this, cx| {
                    this.notices.dismiss_failed = false;
                    this.notices.dismissing = None;
                    this.dismiss_notices(ids.clone(), cx)
                },
            ));
        }
        rows
    }

    fn main_view(&self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let owner = self.role() == "owner";
        let member = matches!(self.role(), "member_role" | "child");
        let only_parent = self.role() == "member_role"
            && self
                .members()
                .is_some_and(|m| m.iter().all(|m| m.user_id.as_deref() == self.me()));
        let mut out: Vec<AnyElement> = self
            .notices_view(t, cx)
            .into_iter()
            .map(IntoElement::into_any_element)
            .collect();
        out.push(
            linked(
                "parental-controls-intro",
                "Parents and teens can link accounts, giving parents tools to adjust certain features, set limits, and add safeguards that work for their family. <a>Learn more</a>",
                FAQ.into(),
                14.,
                t.text,
                t,
            )
            .into_any_element(),
        );
        if let Family::Ready {
            invite: Some(invite),
            ..
        } = &self.family
        {
            let invite = invite.clone();
            out.push(
                card(
                    vec![settings_row_content(
                        "You have a pending family invite",
                        None,
                        action(
                            "parental-controls-open-invite",
                            "Invite",
                            Kind::Secondary,
                            self.busy,
                            t,
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if !this.busy {
                                this.open_invite(invite.clone(), window, cx)
                            }
                        }))
                        .into_any_element(),
                        t,
                    )],
                    Some(&self.layer),
                    t,
                )
                .into_any_element(),
            );
        }
        if let Some(members) = self.members().filter(|m| !m.is_empty()) {
            let mut sorted = members.clone();
            sorted.sort_by_key(Member::invited);
            let rows = sorted
                .into_iter()
                .enumerate()
                .map(|(index, member)| {
                    let row = member.clone();
                    div().child(
                        div()
                            .id(("parental-controls-member", index))
                            .debug_selector(move || format!("parental-controls-member-{index}"))
                            .cursor_pointer()
                            .child(profile(&row, t).child(icon(
                                "chevron-right",
                                16.,
                                t.text_tertiary,
                            )))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_member(member.clone(), cx)
                            })),
                    )
                })
                .collect();
            out.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(text("Family members", 14., 21., 500., t.text))
                    .child(card(rows, Some(&self.layer), t))
                    .into_any_element(),
            );
        }
        if !member || only_parent {
            let label = if owner {
                "Add child"
            } else {
                "Add family member"
            };
            out.push(
                div()
                    .flex()
                    .child(
                        action("parental-controls-add", label, Kind::Secondary, false, t).on_click(
                            cx.listener(|this, _, window, cx| this.open_form(window, cx)),
                        ),
                    )
                    .into_any_element(),
            );
        }
        if owner {
            out.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(text("Safety notifications", 14., 21., 500., t.text))
                    .child(paragraph("We care about your teen's safety. If we detect certain serious safety concerns, we will notify you—though no system is perfect. You can choose how you'd like to be contacted.", t))
                    .child(
                        div().flex().child(
                            action("parental-controls-manage-notifications", "Manage notifications", Kind::Secondary, false, t)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(
                                        Box::new(OpenSection { label: "Notifications".into() }),
                                        cx,
                                    )
                                }),
                        ),
                    )
                    .into_any_element(),
            );
        }
        out
    }

    fn detail_view(&self, member: &Member, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let owner = self.role() == "owner";
        let teen = matches!(self.role(), "member_role" | "child");
        let back = div()
            .flex()
            .child(
                action("parental-controls-back", "Back", Kind::Secondary, false, t)
                    .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
            )
            .into_any_element();
        let mut out = vec![back];
        let accepted = self.members().is_some_and(|members| {
            members.iter().any(|m| {
                if member.invited() {
                    m.invited() && m.identifier() == member.identifier()
                } else {
                    !m.invited() && m.user_id.is_some() && m.user_id == member.user_id
                }
            })
        });
        if !accepted {
            out.push(
                settings_row_content(
                    "This family member is no longer available",
                    None,
                    div().into_any_element(),
                    t,
                )
                .into_any_element(),
            );
            return out;
        }
        out.push(card(vec![profile(member, t)], Some(&self.layer), t).into_any_element());
        if member.invited() {
            let (resend, cancel) = (member.clone(), member.clone());
            out.push(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        action(
                            "parental-controls-resend",
                            "Resend invitation",
                            Kind::Secondary,
                            self.busy,
                            t,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                if !this.busy {
                                    this.confirm_resend(resend.clone(), window, cx)
                                }
                            },
                        )),
                    )
                    .child(
                        action(
                            "parental-controls-cancel-invite",
                            "Cancel invitation",
                            Kind::Danger,
                            self.busy,
                            t,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.busy {
                                this.invitation_call(cancel.clone(), false, cx)
                            }
                        })),
                    )
                    .into_any_element(),
            );
            if !owner {
                out.push(paragraph("You can link to one parent at a time", t));
            }
            return out;
        }
        let me = member.user_id.as_deref() == self.me();
        // `ge` for a teen opened by its parent or by itself; else the parent's own page.
        let teen_page = member.role != "owner" && (owner || me);
        let show_unlink = teen_page || !(teen && member.role != "owner");
        if show_unlink {
            // The original says `Remove from family` only on the parent's own page.
            let label = if owner && member.role == "owner" {
                "Remove from family"
            } else {
                "Unlink accounts"
            };
            let target = member.clone();
            let unlink_self = me || !teen_page;
            out.push(
                div()
                    .flex()
                    .child(
                        action(
                            "parental-controls-unlink",
                            label,
                            Kind::Danger,
                            self.busy,
                            t,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                if !this.busy {
                                    this.confirm_unlink(target.clone(), unlink_self, window, cx)
                                }
                            },
                        )),
                    )
                    .into_any_element(),
            );
        }
        if !teen_page {
            out.push(if owner {
                linked(
                    "parental-controls-help",
                    "<a>Learn more</a> about parental controls",
                    FAQ.into(),
                    13.,
                    t.text_secondary,
                    t,
                )
                .into_any_element()
            } else {
                paragraph("You can link one parent at a time", t)
            });
        }
        out
    }

    fn form_view(
        &self,
        form: &AddForm,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let field = if form.phone {
            &form.phone_field
        } else {
            &form.email_field
        };
        let ready =
            !self.busy && form.role.is_some() && !field.read(cx).field_text().trim().is_empty();
        let radio = |value: &'static str, label: &'static str| {
            let checked = form.role == Some(value);
            div()
                .id(value)
                .debug_selector(move || format!("parental-controls-role-{value}"))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .child(
                    div()
                        .size(px(14.))
                        .rounded_full()
                        .border_1()
                        .border_color(if checked { t.text } else { t.text_tertiary })
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(checked, |d| {
                            d.child(div().size(px(6.)).rounded_full().bg(t.text))
                        }),
                )
                .child(text(label, 13., 18., 430., t.text))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(form) = &mut this.form {
                        form.role = Some(value);
                        cx.notify();
                    }
                }))
        };
        let title = if form.owner {
            "Add a child"
        } else {
            "Invite family member"
        };
        let privacy = form.role.map(|role| {
            let s = if role == "teen" {
                "If your child accepts, you'll be able to manage some of their settings, set time limits, and add safeguards that work for your family. You won't have access to details about your teen's conversations with ChatGPT, except in certain rare safety circumstances. You can unlink accounts anytime. <a>Learn more</a>"
            } else {
                "Your parent or guardian will be able to adjust certain features, set time limits, and add safeguards to help guide your experience. We won't share details of your conversations with ChatGPT, except in certain rare safety circumstances. You can unlink accounts anytime. <a>Learn more</a>"
            };
            linked("parental-controls-privacy", s, FAQ.into(), 12., t.text_secondary, t)
        });
        div()
            .id("parental-controls-form")
            .w(px(400.))
            .relative()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(on_card(25., shapes::Corner::Squircle, t.menu_background, Some(t.border)))
            .child(
                div()
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(text(title, 16., 22., 600., t.text))
                    .when(form.choose_role, |d| {
                        d.child(text("This person is:", 13., 18., 500., t.text))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(8.))
                                    .child(radio("parent", "My parent or guardian"))
                                    .child(radio("teen", "My child")),
                            )
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(text(
                                if form.phone { "Phone number" } else { "Email address" },
                                13.,
                                18.,
                                500.,
                                t.text,
                            ))
                            .child(
                                div()
                                    .id("parental-controls-identifier-kind")
                                    .debug_selector(|| "parental-controls-identifier-kind".into())
                                    .cursor_pointer()
                                    .child(text(
                                        if form.phone { "Use email" } else { "Use phone" },
                                        13.,
                                        18.,
                                        430.,
                                        t.text_secondary,
                                    ))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(form) = &mut this.form {
                                            form.phone = !form.phone;
                                            form.invalid = None;
                                            let field = if form.phone {
                                                &form.phone_field
                                            } else {
                                                &form.email_field
                                            };
                                            window.focus(&field.focus_handle(cx), cx);
                                            cx.notify();
                                        }
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "parental-controls-identifier".into())
                            .child(field.clone()),
                    )
                    .when(form.invalid.is_some(), |d| {
                        d.child(status(
                            if form.phone {
                                "Enter a valid phone number."
                            } else {
                                "Enter a valid email address."
                            },
                            true,
                            t,
                        ))
                    })
                    .child(status(
                        "If your family member is new to ChatGPT, they'll be asked to create an account.",
                        false,
                        t,
                    ))
                    .children(privacy)
                    .children(form.error.clone().map(|error| {
                        status(format!("Unable to send invite: {error}"), true, t)
                    }))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                action("parental-controls-form-cancel", "Cancel", Kind::Secondary, self.busy, t)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if !this.busy {
                                            this.form = None;
                                            cx.notify();
                                        }
                                    })),
                            )
                            .child(
                                action("parental-controls-send", "Send", Kind::Primary, !ready, t)
                                    .when(ready, |b| {
                                        b.on_click(cx.listener(|this, _, _, cx| this.submit_invite(cx)))
                                    }),
                            ),
                    ),
            )
    }
}

impl Render for ParentalControls {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let (account, _) = Self::account(cx);
        match account {
            Account::Loading => return bare(loading_row(&t)),
            Account::Error => {
                return bare(retry_row(
                    "Your family information couldn’t be loaded",
                    "parental-controls-retry",
                    &t,
                    cx,
                    |_, cx| notifications::dispatch(notifications::Action::RefreshAccount, cx),
                ));
            }
            Account::Hidden => {
                return bare(settings_row_content(
                    "Parental controls are available in your personal ChatGPT account",
                    None,
                    div().into_any_element(),
                    &t,
                ));
            }
            Account::Personal => {}
        }
        match &self.family {
            Family::Loading => return bare(loading_row(&t)),
            Family::Failed => {
                return bare(retry_row(
                    "Your family information couldn’t be loaded",
                    "parental-controls-retry",
                    &t,
                    cx,
                    |this, cx| {
                        this.family = Family::Loading;
                        this.reload(cx)
                    },
                ));
            }
            Family::Ready { .. } => {}
        }
        let mut content: Vec<AnyElement> = Vec::new();
        if self.refresh_failed {
            content.push(
                retry_row(
                    "Your family information couldn’t be refreshed",
                    "parental-controls-refresh-retry",
                    &t,
                    cx,
                    |this, cx| this.reload(cx),
                )
                .into_any_element(),
            );
        }
        match self.detail.clone() {
            Some(member) => content.extend(self.detail_view(&member, &t, cx)),
            None => content.extend(self.main_view(&t, cx)),
        }
        if let Some((message, error)) = self.message.clone() {
            content.push(status(message, error, &t).into_any_element());
        }
        let page = frame(
            "parental-controls-page",
            "Parental controls",
            Vec::new(),
            div().flex().flex_col().gap(px(16.)).children(content),
            self.confirmation.as_ref(),
            &self.layer,
            window,
            &t,
        );
        let help = crate::tooltip::tooltip(
            div()
                .id("parental-controls-help-icon")
                .debug_selector(|| "parental-controls-help-icon".into())
                .aria_label("Learn more about parental controls")
                .cursor_pointer()
                .child(icon("import-info", 16., t.text_secondary))
                .on_click(|_, _, cx| cx.open_url(FAQ)),
            "Learn more about parental controls",
            None,
            crate::tooltip::Side::Bottom,
        );
        let viewport = window.viewport_size();
        div()
            .size_full()
            .relative()
            .child(page)
            .when(self.detail.is_none(), |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(46. + 28.))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .w_full()
                                .max_w(px(768.))
                                .px(px(20.))
                                .h(px(33.6))
                                .flex()
                                .items_center()
                                .justify_end()
                                .child(help),
                        ),
                )
            })
            .children(self.form.as_ref().map(|form| {
                let form = self.form_view(form, &t, cx);
                deferred(
                    gpui::anchored()
                        .position(gpui::point(px(0.), px(0.)))
                        .child(
                            div()
                                .w(viewport.width)
                                .h(viewport.height)
                                .occlude()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(rgba(0x00000022))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        if !this.busy {
                                            this.form = None;
                                            cx.notify();
                                        }
                                    }),
                                )
                                .child(form),
                        ),
                )
            }))
            .into_any_element()
    }
}

fn format_list(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_matches_encode_uri_component() {
        assert_eq!(encode("fam 1/ä"), "fam%201%2F%C3%A4");
        assert_eq!(encode("a-b_c.d!~*'()"), "a-b_c.d!~*'()");
    }

    #[test]
    fn phone_numbers_become_e164_with_8_to_15_digits() {
        assert_eq!(e164("+1 (415) 555-0100").as_deref(), Some("+14155550100"));
        assert_eq!(e164("4155550100"), None);
        assert_eq!(e164("+12345"), None);
        assert_eq!(e164("+1415555abcd"), None);
    }

    #[test]
    fn member_label_falls_back_like_the_original() {
        let invited = Member {
            status: "invited".into(),
            ..Default::default()
        };
        assert_eq!(invited.label("Family member"), "[missing]");
        let accepted = Member {
            status: "accepted".into(),
            email: Some("kid@example.com".into()),
            ..Default::default()
        };
        assert_eq!(accepted.label("Member"), "kid@example.com");
        assert_eq!(Member::default().label("Member"), "Member");
    }
}
