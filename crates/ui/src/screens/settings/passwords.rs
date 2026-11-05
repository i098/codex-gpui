//! Settings ▸ Passwords (`PasswordsSettings`): the 1Password connection
//! (`/password-managers/1password`) and the saved passwords (`/saved-credentials`) that the
//! ChatGPT backend keeps for the cloud browser.
use super::{
    Layer,
    account_page::{Kind, action, frame, paragraph, status},
    ads_controls::{Load, date},
    backend::{self, Call},
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
use serde_json::{Value, json};
use std::time::Duration;
use theme::{ActiveTheme as _, Theme};

const MANAGER: &str = "/password-managers/1password";
const MANAGER_ERROR: &str = "We couldn’t confirm your connection. Refresh to try again";
const CONNECT_ERROR: &str = "We couldn’t start the connection. Try again";
const LIST_ERROR: &str = "We couldn’t confirm the latest saved passwords. Refresh to check.";
/// The original's `refetchInterval` while the connection is `connecting`.
const POLL: Duration = Duration::from_millis(1500);
/// The only `authorization_url` origins the original opens.
const AUTHORIZE_ORIGINS: [&str; 3] = [
    "https://my.b5dev.eu",
    "https://my.b5dev.com",
    "https://my.1password.com",
];

/// One `/saved-credentials` item.
#[derive(Clone, Debug, PartialEq)]
pub struct Password {
    pub id: String,
    pub origin: String,
    pub account: String,
    pub created_at: Value,
}

impl Password {
    fn read(item: &Value) -> Option<Self> {
        Some(Self {
            id: item["credential_id"].as_str()?.to_owned(),
            origin: item["origin"].as_str()?.to_owned(),
            account: item["account_label"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            created_at: item["created_at"].clone(),
        })
    }

    /// `new URL(origin).hostname`.
    fn site(&self) -> String {
        url::Url::parse(&self.origin)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_else(|| self.origin.clone())
    }

    /// The original's search: `${origin} ${account_label}`, lowercased, contains the query.
    fn matches(&self, query: &str) -> bool {
        format!("{} {}", self.origin, self.account)
            .to_lowercase()
            .contains(query)
    }
}

/// The 1Password card's description, in the original's order of checks.
fn manager_status(manager: &Load<Value>, error: Option<&'static str>) -> &'static str {
    let manager = match (error, manager) {
        (Some(error), _) => return error,
        (None, Load::Failed) => return MANAGER_ERROR,
        (None, Load::Loading) => return "Checking connection…",
        (None, Load::Ready(manager)) => manager,
    };
    if manager["state"] == "connected" {
        "Connected"
    } else if manager["available"] != true || manager["enrollment_url"].is_null() {
        "1Password setup isn’t available in this workspace yet"
    } else {
        match manager["state"].as_str() {
            Some("connecting") => "Connecting… Complete authorization in your browser",
            Some("failed" | "reconnect_required") => {
                "Connection needs attention. Connect again to retry"
            }
            _ => "Use selected logins from 1Password when ChatGPT asks you to sign in",
        }
    }
}

/// The enrollment answer's `authorization_url`, only when it is 1Password's `/oauth/authorize`.
fn authorize_url(body: &Value) -> Option<String> {
    let url = url::Url::parse(body["authorization_url"].as_str()?).ok()?;
    let origin = url.origin().ascii_serialization();
    (AUTHORIZE_ORIGINS.contains(&origin.as_str()) && url.path() == "/oauth/authorize")
        .then(|| url.to_string())
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("passwords-settings", cx, Passwords::new)
        .into_any_element()
}

pub struct Passwords {
    pub manager: Load<Value>,
    manager_error: Option<&'static str>,
    manager_busy: bool,
    polling: bool,
    pub passwords: Vec<Password>,
    cursor: Option<String>,
    loading: bool,
    list_error: bool,
    deleting: bool,
    search: Entity<Composer>,
    confirmation: Option<Entity<Confirmation>>,
    layer: Layer,
}

impl Passwords {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| Composer::settings_compact_field("Search passwords", "", cx));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        let page = cx.entity();
        window.defer(cx, move |_, cx| {
            page.update(cx, |this, cx| {
                this.load_manager(cx);
                this.refresh(cx);
            })
        });
        Self {
            manager: Load::Loading,
            manager_error: None,
            manager_busy: false,
            polling: false,
            passwords: Vec::new(),
            cursor: None,
            loading: false,
            list_error: false,
            deleting: false,
            search,
            confirmation: None,
            layer: Layer::default(),
        }
    }

    /// `GET /password-managers/1password`; polls again while the state is `connecting`.
    fn load_manager(&mut self, cx: &mut Context<Self>) {
        backend::request(Call::new("GET", MANAGER, None), cx, |this, result, cx| {
            this.manager = match result {
                Ok(response) if response.ok() => Load::Ready(response.body),
                _ => Load::Failed,
            };
            if matches!(&this.manager, Load::Ready(m) if m["state"] == "connecting") {
                this.poll(cx);
            }
        });
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        if self.polling {
            return;
        }
        self.polling = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(POLL).await;
            this.update(cx, |this, cx| {
                this.polling = false;
                this.load_manager(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The `Refresh` button: clears errors and rereads the connection.
    fn refresh_manager(&mut self, cx: &mut Context<Self>) {
        self.manager_error = None;
        self.manager = Load::Loading;
        self.load_manager(cx);
        cx.notify();
    }

    fn confirm_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| {
            Confirmation::new(
                "Set up 1Password?",
                "Use selected logins from 1Password when ChatGPT asks you to sign in. You choose which login to share.",
                "Set up 1Password",
                window,
                cx,
            )
            .settings_style()
        });
        cx.subscribe(&dialog, |this, _, outcome, cx| {
            if *outcome == Outcome::Confirm {
                this.enroll(cx);
            }
            this.confirmation = None;
            cx.notify();
        })
        .detach();
        self.show(dialog, window, cx);
    }

    /// `POST /password-managers/1password/enrollment` with the account user, then opens the
    /// 1Password authorization page in the browser.
    pub fn enroll(&mut self, cx: &mut Context<Self>) {
        if self.manager_busy {
            return;
        }
        self.manager_busy = true;
        self.manager_error = None;
        let call = Call::new("POST", format!("{MANAGER}/enrollment"), Some(json!({})))
            .with_account_user("expected_account_user_id");
        backend::request(call, cx, |this, result, cx| {
            this.manager_busy = false;
            match result
                .ok()
                .filter(|r| r.ok())
                .and_then(|r| authorize_url(&r.body))
            {
                Some(url) => cx.open_url(&url),
                None => this.manager_error = Some(CONNECT_ERROR),
            }
            this.load_manager(cx);
        });
        cx.notify();
    }

    /// `DELETE /password-managers/1password`.
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        if self.manager_busy {
            return;
        }
        self.manager_busy = true;
        self.manager_error = None;
        backend::request(
            Call::new("DELETE", MANAGER, None),
            cx,
            |this, result, cx| {
                this.manager_busy = false;
                if !result.is_ok_and(|r| r.ok()) {
                    this.manager_error = Some(MANAGER_ERROR);
                }
                this.load_manager(cx);
            },
        );
        cx.notify();
    }

    /// Rereads the saved passwords from the first page.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.passwords.clear();
        self.cursor = None;
        self.loading = false;
        self.list_error = false;
        self.load_page(cx);
    }

    /// `GET /saved-credentials`, from the last page's `next_cursor`.
    fn load_page(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.list_error = false;
        let path = match &self.cursor {
            Some(cursor) => format!(
                "/saved-credentials?cursor={}",
                url::form_urlencoded::byte_serialize(cursor.as_bytes()).collect::<String>()
            ),
            None => "/saved-credentials".to_owned(),
        };
        backend::request(Call::new("GET", path, None), cx, |this, result, _| {
            this.loading = false;
            match result {
                Ok(response) if response.ok() => {
                    let body = response.body;
                    let items = body["items"].as_array().into_iter().flatten();
                    this.passwords.extend(items.filter_map(Password::read));
                    this.cursor = body["next_cursor"].as_str().map(str::to_owned);
                }
                _ => this.list_error = true,
            }
        });
        cx.notify();
    }

    /// The password's details: website, account and saved date, with `Delete password`.
    fn open_details(&mut self, password: Password, window: &mut Window, cx: &mut Context<Self>) {
        let mut details = format!(
            "Website: {}\nAccount: {}",
            password.origin, password.account
        );
        if let Some(saved) = date(&password.created_at) {
            details.push_str(&format!("\nSaved: {saved}"));
        }
        let dialog = cx.new(|cx| {
            Confirmation::new(password.site(), details, "Delete password", window, cx)
                .destructive()
                .settings_style()
                .cancel_label("Close")
        });
        cx.subscribe_in(&dialog, window, move |this, _, outcome, window, cx| {
            this.confirmation = None;
            if *outcome == Outcome::Confirm {
                this.confirm_delete(password.clone(), window, cx);
            }
            cx.notify();
        })
        .detach();
        self.show(dialog, window, cx);
    }

    fn confirm_delete(&mut self, password: Password, window: &mut Window, cx: &mut Context<Self>) {
        let description = format!(
            "{}\n{}\n\nThis won’t sign you out of the website",
            password.origin, password.account
        );
        let dialog = cx.new(|cx| {
            Confirmation::new("Delete this password?", description, "Delete", window, cx)
                .destructive()
                .settings_style()
        });
        cx.subscribe(&dialog, move |this, _, outcome, cx| {
            if *outcome == Outcome::Confirm {
                this.delete(&password.id, cx);
            }
            this.confirmation = None;
            cx.notify();
        })
        .detach();
        self.show(dialog, window, cx);
    }

    fn show(&mut self, dialog: Entity<Confirmation>, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&dialog.focus_handle(cx), cx);
        self.confirmation = Some(dialog);
        cx.notify();
    }

    /// `DELETE /saved-credentials/{credential_id}`, then rereads the list.
    pub fn delete(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.deleting {
            return;
        }
        self.deleting = true;
        let path = format!(
            "/saved-credentials/{}",
            url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>()
        );
        backend::request(Call::new("DELETE", path, None), cx, |this, result, cx| {
            this.deleting = false;
            if result.is_ok_and(|r| r.ok()) {
                this.refresh(cx);
            } else {
                this.list_error = true;
            }
        });
        cx.notify();
    }

    fn manager_view(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let manager = match &self.manager {
            Load::Ready(manager) => Some(manager),
            _ => None,
        };
        let state = manager.and_then(|m| m["state"].as_str());
        let can_set_up = manager.is_some_and(|m| m["available"] == true)
            && manager.is_some_and(|m| !m["enrollment_url"].is_null())
            && state != Some("connected");
        let busy = self.manager_busy;
        let controls = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .when(can_set_up, |d| {
                d.child(
                    action(
                        "password-manager-set-up",
                        "Set up",
                        Kind::Secondary,
                        busy,
                        t,
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.confirm_setup(window, cx))),
                )
            })
            .when(state.is_some_and(|s| s != "disconnected"), |d| {
                d.child(
                    action(
                        "password-manager-disconnect",
                        "Disconnect",
                        Kind::Secondary,
                        busy,
                        t,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.disconnect(cx))),
                )
            })
            .child(
                action(
                    "password-manager-refresh",
                    "Refresh",
                    Kind::Secondary,
                    busy,
                    t,
                )
                .on_click(cx.listener(|this, _, _, cx| this.refresh_manager(cx))),
            );
        let row = settings_row_content(
            "1Password",
            Some(
                div()
                    .debug_selector(|| "password-manager-status".into())
                    .child(status(
                        manager_status(&self.manager, self.manager_error),
                        false,
                        t,
                    ))
                    .into_any_element(),
            ),
            controls.into_any_element(),
            t,
        );
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(text("Password manager", 16., 22., 500., t.text))
            .child(card(vec![row], Some(&self.layer), t))
    }

    fn list_view(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let query = self.search.read(cx).field_text().trim().to_lowercase();
        let shown: Vec<&Password> = self
            .passwords
            .iter()
            .filter(|p| p.matches(&query))
            .collect();
        let title = if self.cursor.is_none() && !self.loading && !self.list_error {
            format!("Saved passwords ({})", self.passwords.len())
        } else {
            "Saved passwords".to_owned()
        };
        let busy = self.loading || self.deleting;
        let header = div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .child(text(title, 16., 22., 500., t.text))
            .child(
                action("passwords-refresh", "Refresh", Kind::Secondary, busy, t)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            );
        let rows: Vec<gpui::Div> = shown
            .iter()
            .enumerate()
            .map(|(index, password)| {
                let remove = action(
                    SharedString::from(format!("password-remove-{index}")),
                    "Remove",
                    Kind::Danger,
                    self.deleting,
                    t,
                )
                .on_click({
                    let password = (*password).clone();
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.confirm_delete(password.clone(), window, cx);
                    })
                });
                let id = SharedString::from(format!("password-row-{index}"));
                let password = (*password).clone();
                div().child(
                    div()
                        .id(id.clone())
                        .debug_selector(move || id.to_string())
                        .cursor_pointer()
                        .child(settings_row_content(
                            password.site(),
                            Some(status(password.account.clone(), false, t).into_any_element()),
                            remove.into_any_element(),
                            t,
                        ))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_details(password.clone(), window, cx)
                        })),
                )
            })
            .collect();
        let empty = !self.loading && !self.list_error && rows.is_empty();
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(12.))
            .child(header)
            .child(
                div()
                    .w_full()
                    .debug_selector(|| "passwords-search".into())
                    .child(self.search.clone()),
            )
            .when(self.list_error, |d| d.child(status(LIST_ERROR, true, t)))
            .when(self.loading, |d| {
                d.child(status("Loading saved passwords…", false, t))
            })
            .when(empty, |d| {
                d.child(paragraph(
                    if query.is_empty() {
                        "No saved passwords"
                    } else {
                        "No matching passwords"
                    },
                    t,
                ))
            })
            .when(!rows.is_empty(), |d| {
                d.child(div().w_full().child(card(rows, Some(&self.layer), t)))
            })
            .when(self.cursor.is_some(), |d| {
                d.when(!query.is_empty(), |d| {
                    d.child(paragraph("Load more passwords to search the rest", t))
                })
                .child(
                    action("passwords-load-more", "Load more", Kind::Secondary, busy, t)
                        .on_click(cx.listener(|this, _, _, cx| this.load_page(cx))),
                )
            })
    }
}

impl Render for Passwords {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let content = div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(self.manager_view(&t, cx))
            .child(self.list_view(&t, cx));
        frame(
            "passwords-page",
            "Passwords",
            vec![paragraph("Manage saved website logins", &t)],
            content,
            self.confirmation.as_ref(),
            &self.layer,
            window,
            &t,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_status_follows_the_original_order() {
        let ready = |v: Value| Load::Ready(v);
        let available = |state: &str| {
            ready(json!({"available": true, "enrollment_url": "https://x", "state": state}))
        };
        assert_eq!(manager_status(&Load::Loading, None), "Checking connection…");
        assert_eq!(manager_status(&Load::Failed, None), MANAGER_ERROR);
        assert_eq!(
            manager_status(&available("connected"), Some(CONNECT_ERROR)),
            CONNECT_ERROR
        );
        // `connected` wins over an unavailable workspace.
        assert_eq!(
            manager_status(&ready(json!({"state": "connected"})), None),
            "Connected"
        );
        assert_eq!(
            manager_status(
                &ready(json!({"available": true, "state": "connecting"})),
                None
            ),
            "1Password setup isn’t available in this workspace yet"
        );
        assert_eq!(
            manager_status(&available("reconnect_required"), None),
            "Connection needs attention. Connect again to retry"
        );
        assert_eq!(
            manager_status(&available("disconnected"), None),
            "Use selected logins from 1Password when ChatGPT asks you to sign in"
        );
    }

    #[test]
    fn authorize_url_accepts_only_1password_authorize() {
        let url = |u: &str| authorize_url(&json!({"authorization_url": u}));
        assert_eq!(
            url("https://my.1password.com/oauth/authorize?x=1").as_deref(),
            Some("https://my.1password.com/oauth/authorize?x=1")
        );
        assert_eq!(url("https://my.1password.com/oauth/token"), None);
        assert_eq!(url("https://evil.example/oauth/authorize"), None);
        assert_eq!(url("http://my.1password.com/oauth/authorize"), None);
        assert_eq!(authorize_url(&json!({})), None);
    }

    #[test]
    fn search_matches_origin_or_account() {
        let p = Password::read(&json!({
            "credential_id": "c1", "origin": "https://Example.com",
            "account_label": "Ada@Example.com", "created_at": 1
        }))
        .unwrap();
        assert_eq!(p.site(), "example.com");
        assert!(p.matches("ada@"));
        assert!(p.matches("example.com ada"));
        assert!(!p.matches("globex"));
    }
}
