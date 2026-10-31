//! Settings ▸ Ads controls (`AdsControlsSettings`): the ChatGPT backend's ads settings.
//! Ads history (`/bazaar/history`), topics (`/bazaar/preferences`), delete all ads data
//! (`DELETE /bazaar/profile`), personalized ads and the free ad-free opt-out.
use super::{
    Layer,
    account_page::{Kind, action, frame, paragraph, status},
    backend::{self, Call},
    settings_row_content, switch,
    worktrees::card,
};
use crate::{
    confirmation::{Confirmation, Outcome},
    icon::icon,
    text,
};
use chrono::{DateTime, Local, TimeZone as _};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use theme::{ActiveTheme as _, Theme};

const UPDATE_ERROR: &str = "Couldn’t update ads controls. Please try again";

/// The ads fields of `GET /settings/user`, mapped like the original's settings query.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// `settings.bazaar_personalization_enabled !== false`.
    pub personalized: bool,
    /// `flags.bazaar_personalization_unavailable === true`.
    pub unavailable: bool,
    /// `settings.free_ads_opt_out === true`: the `Ads are off` state.
    pub opted_out: bool,
}

impl Settings {
    fn read(body: &Value) -> Self {
        Self {
            personalized: body["settings"]["bazaar_personalization_enabled"] != false,
            unavailable: body["flags"]["bazaar_personalization_unavailable"] == true,
            opted_out: body["settings"]["free_ads_opt_out"] == true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Load<T> {
    Loading,
    Failed,
    Ready(T),
}

/// The open view: the main page or one of its two lists.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    Main,
    History,
    Topics,
}

/// One cursor-paged list (`{history|preferences: [...], cursor}`).
#[derive(Default)]
pub struct Pages {
    pub items: Vec<Value>,
    cursor: Option<String>,
    loading: bool,
    failed: bool,
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("ads-controls-settings", cx, AdsControls::new)
        .into_any_element()
}

pub struct AdsControls {
    pub settings: Load<Settings>,
    pub view: View,
    pub pages: Pages,
    pub message: Option<(SharedString, bool)>,
    busy: bool,
    confirmation: Option<Entity<Confirmation>>,
    layer: Layer,
}

impl AdsControls {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let page = cx.entity();
        window.defer(cx, move |_, cx| page.update(cx, Self::reload));
        Self {
            settings: Load::Loading,
            view: View::Main,
            pages: Pages::default(),
            message: None,
            busy: false,
            confirmation: None,
            layer: Layer::default(),
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.settings, Load::Ready(_)) {
            self.settings = Load::Loading;
        }
        let call = Call::new("GET", "/settings/user", None);
        backend::request(call, cx, |this, result, _| {
            this.settings = match result {
                Ok(response) if response.ok() => Load::Ready(Settings::read(&response.body)),
                _ => Load::Failed,
            };
        });
    }

    fn open(&mut self, view: View, cx: &mut Context<Self>) {
        self.view = view;
        self.pages = Pages::default();
        self.message = None;
        if view != View::Main {
            self.load_page(cx);
        }
        cx.notify();
    }

    /// `GET /bazaar/history` or `/bazaar/preferences`, from the last page's cursor.
    fn load_page(&mut self, cx: &mut Context<Self>) {
        let (path, key) = match self.view {
            View::Main => return,
            View::History => ("/bazaar/history", "history"),
            View::Topics => ("/bazaar/preferences", "preferences"),
        };
        if self.pages.loading {
            return;
        }
        self.pages.loading = true;
        self.pages.failed = false;
        let path = match &self.pages.cursor {
            Some(cursor) => format!("{path}?cursor={}", query(cursor)),
            None => path.to_owned(),
        };
        let view = self.view;
        backend::request(Call::new("GET", path, None), cx, move |this, result, _| {
            if this.view != view {
                return;
            }
            this.pages.loading = false;
            match result {
                Ok(response) if response.ok() => {
                    let body = response.body;
                    if let Some(items) = body[key].as_array() {
                        this.pages.items.extend(items.iter().cloned());
                    }
                    this.pages.cursor = body["cursor"].as_str().map(str::to_owned);
                }
                _ => this.pages.failed = true,
            }
        });
    }

    /// `PATCH /settings/account_user_setting?feature=…&value=…`, then reread the settings.
    pub fn set(&mut self, feature: &'static str, value: bool, cx: &mut Context<Self>) {
        let Load::Ready(settings) = self.settings else {
            return;
        };
        if self.busy {
            return;
        }
        self.busy = true;
        self.message = None;
        let path = format!("/settings/account_user_setting?feature={feature}&value={value}");
        backend::request(
            Call::new("PATCH", path, None),
            cx,
            move |this, result, cx| {
                this.busy = false;
                if result.is_ok_and(|r| r.ok()) {
                    this.settings = Load::Ready(match feature {
                        "bazaar_personalization_enabled" => Settings {
                            personalized: value,
                            ..settings
                        },
                        _ => Settings {
                            opted_out: value,
                            ..settings
                        },
                    });
                    this.reload(cx);
                } else {
                    this.message = Some((UPDATE_ERROR.into(), true));
                }
            },
        );
        cx.notify();
    }

    fn confirm_delete(&mut self, unavailable: bool, window: &mut Window, cx: &mut Context<Self>) {
        let description = if unavailable {
            "This will permanently delete your ad history. This action cannot be undone."
        } else {
            "This will permanently delete your ads history and ads topics data. This action cannot be undone."
        };
        let dialog = cx.new(|cx| {
            Confirmation::new("Delete all ads data?", description, "Delete", window, cx)
                .destructive()
                .settings_style()
        });
        cx.subscribe(&dialog, |this, _, outcome, cx| {
            if *outcome == Outcome::Confirm {
                this.delete(cx);
            }
            this.confirmation = None;
            cx.notify();
        })
        .detach();
        window.focus(&dialog.focus_handle(cx), cx);
        self.confirmation = Some(dialog);
        cx.notify();
    }

    /// `DELETE /bazaar/profile`: clears the history and the topics.
    pub fn delete(&mut self, cx: &mut Context<Self>) {
        self.busy = true;
        self.message = None;
        let call = Call::new("DELETE", "/bazaar/profile", None);
        backend::request(call, cx, |this, result, _| {
            this.busy = false;
            if result.is_ok_and(|r| r.ok()) {
                this.pages = Pages::default();
            } else {
                this.message = Some(("Couldn’t delete ads data. Please try again".into(), true));
            }
        });
        cx.notify();
    }

    fn main_view(&self, settings: Settings, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        if settings.opted_out {
            let body = div()
                .p(px(16.))
                .flex()
                .flex_col()
                .items_start()
                .gap(px(12.))
                .child(text("Ads are off", 14., 20., 500., t.text))
                .child(paragraph(
                    "You’re using ChatGPT ad-free with reduced usage. Expand your access by upgrading your plan or turning on ads.",
                    t,
                ))
                .child(
                    action("ads-turn-on", "Turn on ads", Kind::Primary, self.busy, t).on_click(
                        cx.listener(|this, _, _, cx| this.set("free_ads_opt_out", false, cx)),
                    ),
                );
            return div().child(card(vec![body], Some(&self.layer), t));
        }
        let link = |id: &'static str, title: &'static str, view: View, cx: &mut Context<Self>| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .cursor_pointer()
                .child(settings_row_content(
                    title,
                    None,
                    icon("chevron-right", 16., t.text_secondary).into_any_element(),
                    t,
                ))
                .on_click(cx.listener(move |this, _, _, cx| this.open(view, cx)))
                .into_any_element()
        };
        let mut rows = vec![link("ads-history-link", "History", View::History, cx)];
        if !settings.unavailable {
            rows.push(link("ads-topics-link", "Topics", View::Topics, cx));
        }
        let unavailable = settings.unavailable;
        rows.push(
            settings_row_content(
                "Delete ads data",
                Some(paragraph(
                    if unavailable {
                        "Delete your ad history associated with you. This won’t delete or change your chats."
                    } else {
                        "Clear all ads history and topics data. This won't affect your chats."
                    },
                    t,
                )),
                action("ads-delete", "Delete", Kind::Danger, self.busy, t)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.confirm_delete(unavailable, window, cx)
                    }))
                    .into_any_element(),
                t,
            )
            .into_any_element(),
        );
        let on = settings.personalized && !unavailable;
        let toggle = switch("ads-personalize", on, self.busy || unavailable, t)
            .debug_selector(|| "ads-personalize".into())
            .when(!unavailable, |s| {
                s.on_click(cx.listener(move |this, _, _, cx| {
                    this.set("bazaar_personalization_enabled", !on, cx)
                }))
            })
            .into_any_element();
        let personalize = settings_row_content(
            "Personalize ads",
            Some(paragraph(
                if unavailable {
                    "Personalized ads currently aren’t available in your region. You’ll still see ads based on limited data like general location, context, and device type."
                } else {
                    "Allow ChatGPT to use your past chats, activity, and preferences to select ads. Ads may still be based on your current chat."
                },
                t,
            )),
            toggle,
            t,
        );
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(card(rows, Some(&self.layer), t))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(text("Ads personalization", 16., 22., 500., t.text))
                    .child(card(
                        vec![personalize.into_any_element()],
                        Some(&self.layer),
                        t,
                    )),
            )
    }

    fn list_view(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let history = self.view == View::History;
        let (title, description) = if history {
            (
                "Ads history",
                "See the ads you’ve viewed or interacted with. You’re always in control — you can clear your history at any time.",
            )
        } else {
            (
                "Topics inspired by your activity",
                "ChatGPT may use these topics to choose ads that are more relevant to you.",
            )
        };
        let rows: Vec<AnyElement> = self
            .pages
            .items
            .iter()
            .map(|item| {
                if history {
                    let (title, detail) = history_row(item);
                    settings_row_content(
                        title,
                        Some(status(detail, false, t).into_any_element()),
                        div().into_any_element(),
                        t,
                    )
                    .into_any_element()
                } else {
                    let topic = item["content"].as_str().unwrap_or_default().to_owned();
                    settings_row_content(topic, None, div().into_any_element(), t)
                        .into_any_element()
                }
            })
            .collect();
        let empty = !self.pages.loading && !self.pages.failed && rows.is_empty();
        let back = div()
            .id("ads-back")
            .debug_selector(|| "ads-back".into())
            .flex()
            .items_center()
            .gap(px(4.))
            .cursor_pointer()
            .child(icon("chevron-left", 16., t.text_secondary))
            .child(text("Back", 13., 18., 500., t.text_secondary))
            .on_click(cx.listener(|this, _, _, cx| this.open(View::Main, cx)));
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(12.))
            .child(back)
            .child(text(title, 16., 22., 500., t.text))
            .child(paragraph(description, t))
            .when(!rows.is_empty(), |d| {
                d.child(div().w_full().child(card(rows, Some(&self.layer), t)))
            })
            .when(empty, |d| {
                let (title, detail) = if history {
                    ("No ad history", "Ads you view in ChatGPT will appear here")
                } else {
                    (
                        "No topics yet",
                        "ChatGPT may use topics to make ads more relevant. Topics may be based on your activity or feedback on ads.",
                    )
                };
                d.child(text(title, 14., 20., 500., t.text)).child(paragraph(detail, t))
            })
            .when(self.pages.loading, |d| {
                d.child(status("Loading ads controls…", false, t))
            })
            .when(self.pages.failed, |d| {
                d.child(status("Couldn’t load ads controls. Please try again", false, t))
                    .child(
                        action("ads-list-retry", "Retry", Kind::Secondary, false, t)
                            .on_click(cx.listener(|this, _, _, cx| this.load_page(cx))),
                    )
            })
            .when(
                self.pages.cursor.is_some() && !self.pages.loading && !self.pages.failed,
                |d| {
                    d.child(
                        action("ads-load-more", "Load more", Kind::Secondary, false, t)
                            .on_click(cx.listener(|this, _, _, cx| this.load_page(cx))),
                    )
                },
            )
    }
}

impl Render for AdsControls {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let content = match (&self.settings, self.view) {
            (Load::Loading, _) => div().child(status("Loading ads controls…", false, &t)),
            (Load::Failed, _) => div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(12.))
                .child(status(
                    "Couldn’t load ads controls. Please try again",
                    false,
                    &t,
                ))
                .child(
                    action("ads-retry", "Retry", Kind::Secondary, false, &t)
                        .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                ),
            (Load::Ready(settings), View::Main) => self.main_view(*settings, &t, cx),
            (Load::Ready(_), _) => self.list_view(&t, cx),
        };
        let message = self
            .message
            .clone()
            .map(|(message, error)| status(message, error, &t));
        frame(
            "ads-controls-page",
            "Ads controls",
            Vec::new(),
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

/// `{brand} — {title}` and `{date} · Viewed|Opened` for one `/bazaar/history` entry.
fn history_row(item: &Value) -> (String, String) {
    let ad = &item["ad_data"];
    let brand = ad["advertiser_brand"]["name"].as_str().unwrap_or_default();
    let title = ad["title"].as_str().unwrap_or_default();
    let mut detail = date(&item["occurred_at"]).unwrap_or_default();
    let action = match item["action"].as_str() {
        Some("viewed") => "Viewed",
        Some("opened") => "Opened",
        _ => "",
    };
    if !action.is_empty() {
        if !detail.is_empty() {
            detail.push_str(" · ");
        }
        detail.push_str(action);
    }
    (format!("{brand} — {title}"), detail)
}

/// `occurred_at` (RFC 3339, or Unix seconds or milliseconds) as a local `Oct 5, 2026`.
pub(super) fn date(value: &Value) -> Option<String> {
    let time = match value {
        Value::String(s) => DateTime::parse_from_rfc3339(s).ok()?.with_timezone(&Local),
        Value::Number(n) => {
            let n = n.as_f64()?;
            let millis = if n > 1e12 { n } else { n * 1000. };
            Local.timestamp_millis_opt(millis as i64).single()?
        }
        _ => return None,
    };
    Some(time.format("%b %-d, %Y").to_string())
}

/// Percent-encodes a cursor for the query string.
fn query(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_default_to_personalized() {
        assert_eq!(
            Settings::read(&json!({})),
            Settings {
                personalized: true,
                unavailable: false,
                opted_out: false
            }
        );
        let body = json!({
            "settings": {"bazaar_personalization_enabled": false, "free_ads_opt_out": true},
            "flags": {"bazaar_personalization_unavailable": true},
        });
        assert_eq!(
            Settings::read(&body),
            Settings {
                personalized: false,
                unavailable: true,
                opted_out: true
            }
        );
    }

    #[test]
    fn history_row_names_the_ad_and_action() {
        let item = json!({
            "ad_data": {"advertiser_brand": {"name": "Acme"}, "title": "Rockets"},
            "action": "opened",
            "occurred_at": "not a date",
        });
        assert_eq!(
            history_row(&item),
            ("Acme — Rockets".into(), "Opened".into())
        );
        assert!(date(&json!(1_760_000_000)).is_some_and(|d| d.ends_with(", 2025")));
        assert_eq!(
            date(&json!(1_760_000_000)),
            date(&json!(1_760_000_000_000u64))
        );
    }
}
