//! Settings ▸ Agreements (`AgreementsSettings`): for a `quorum` workspace, the OpenAI Services
//! Agreement link and, in a HIPAA-eligible workspace, the Business Associate Agreement. The
//! BAA state comes from `GET /accounts/{account_id}/metadata`; Agree sends
//! `POST /quorum/workspace/{account_id}/sign_baa`.
use super::{
    Layer,
    account::{self, Profile},
    account_page::{Kind, action, frame, status},
    backend::{self, Call},
    parental_controls::linked_styled,
    settings_row_content,
    worktrees::card,
};
use crate::{screens::computer_use::Load as Profiled, text};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone as _};
use gpui::{
    AnyElement, App, Context, Div, HighlightStyle, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, UnderlineStyle, Window, div,
    prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use theme::{ActiveTheme as _, Theme};

const SERVICES_AGREEMENT: &str = "https://openai.com/policies/services-agreement/";
const BAA: &str = "https://cdn.openai.com/osa/baa.pdf";
const HIPAA_HELP: &str = "https://help.openai.com/en/articles/20001069-chatgpt-healthcare-and-regulated-workspace-functionality";

/// The workspace's `workspace_baa_metadata`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Baa {
    pub signed: bool,
    pub timestamp: Option<Value>,
}

impl Baa {
    /// A timestamp alone also means signed, as in the original (`signed ?? !!timestamp`).
    pub fn from_metadata(body: &Value) -> Self {
        let meta = &body["workspace_baa_metadata"];
        let timestamp = Some(meta["timestamp"].clone()).filter(|t| match t {
            Value::String(s) => !s.is_empty(),
            Value::Number(_) => true,
            _ => false,
        });
        Self {
            signed: meta["signed"].as_bool().unwrap_or(timestamp.is_some()),
            timestamp,
        }
    }

    /// `BAA was agreed on`: the time in local time like `toLocaleString()`, the raw value when
    /// it is not a date, or `Not available`.
    pub fn agreed_on(&self) -> String {
        let format =
            |time: DateTime<Local>| time.format("%-m/%-d/%Y, %-I:%M:%S\u{202f}%p").to_string();
        match &self.timestamp {
            Some(Value::String(s)) => DateTime::parse_from_rfc3339(s)
                .map(|t| t.with_timezone(&Local))
                .ok()
                .or_else(|| {
                    // A date without an offset is local time, as `new Date(...)` reads it.
                    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
                        .ok()
                        .and_then(|t| Local.from_local_datetime(&t).single())
                })
                .map_or_else(|| s.clone(), format),
            // A number is epoch milliseconds, as `new Date(number)` reads it.
            Some(Value::Number(n)) => n
                .as_f64()
                .and_then(|ms| Local.timestamp_millis_opt(ms as i64).single())
                .map_or_else(|| n.to_string(), format),
            _ => "Not available".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Load {
    Loading,
    Failed,
    Ready(Baa),
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("agreements-settings", cx, |window, cx| {
            cx.observe_global_in::<account::State>(window, |this: &mut Agreements, _, cx| {
                this.sync(cx);
                cx.notify();
            })
            .detach();
            let page = cx.entity();
            window.defer(cx, move |_, cx| page.update(cx, Agreements::sync));
            Agreements {
                account: None,
                load: Load::Loading,
                signing: false,
                sign_failed: false,
                refetch_failed: false,
                layer: Layer::default(),
            }
        })
        .into_any_element()
}

pub struct Agreements {
    /// The workspace whose metadata `load` holds.
    account: Option<String>,
    load: Load,
    signing: bool,
    sign_failed: bool,
    /// A reread after the BAA was shown failed: the error and Retry show below it.
    refetch_failed: bool,
    layer: Layer,
}

/// The signed-in profile when the account has Agreements access (`status: allowed`).
fn allowed(cx: &App) -> Option<Profile> {
    match &cx.try_global::<account::State>()?.profile {
        Profiled::Ready(profile) if profile.agreements() => Some(profile.clone()),
        _ => None,
    }
}

impl Agreements {
    /// Loads the metadata of the allowed workspace when it changed.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let account = allowed(cx).map(|p| p.account_id);
        if account.is_some() && account != self.account {
            self.account = account;
            self.load = Load::Loading;
            self.reload(cx);
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else {
            return;
        };
        self.refetch_failed = false;
        let call = Call::new("GET", format!("/accounts/{account}/metadata"), None);
        backend::request(call, cx, move |this, result, _| {
            if this.account.as_deref() != Some(account.as_str()) {
                return;
            }
            match result {
                Ok(response) if response.ok() => {
                    this.load = Load::Ready(Baa::from_metadata(&response.body));
                }
                _ if matches!(this.load, Load::Ready(_)) => this.refetch_failed = true,
                _ => this.load = Load::Failed,
            }
        });
        cx.notify();
    }

    /// `POST /quorum/workspace/{account_id}/sign_baa`, then the metadata again.
    fn agree(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else {
            return;
        };
        if self.signing {
            return;
        }
        self.signing = true;
        self.sign_failed = false;
        let call = Call::new(
            "POST",
            format!("/quorum/workspace/{account}/sign_baa"),
            None,
        );
        backend::request(call, cx, |this, result, cx| {
            this.signing = false;
            match result {
                Ok(response) if response.ok() => {
                    this.load = Load::Ready(Baa {
                        signed: true,
                        timestamp: Some(response.body["timestamp"].clone())
                            .filter(|t| !t.is_null()),
                    });
                    this.reload(cx);
                }
                _ => this.sign_failed = true,
            }
        });
        cx.notify();
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        if self.account.is_some() {
            self.reload(cx);
        } else {
            account::dispatch(account::Action::Refresh, cx);
        }
    }

    /// The original's load error (`workspaceAgreements.loadError`) with Retry.
    fn load_error(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        div()
            .py(px(16.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.))
            .child(status("Couldn't load agreements", false, t))
            .child(
                action("agreements-retry", "Retry", Kind::Secondary, false, t)
                    .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
            )
    }

    fn view(&self, baa: &Baa, hipaa: bool, t: &Theme, cx: &mut Context<Self>) -> Div {
        let services = section(
            "OpenAI Services Agreement",
            None,
            card(
                vec![settings_row_content(
                    "Review the current Services Agreement.",
                    None,
                    view_link("agreements-services-view", SERVICES_AGREEMENT, t),
                    t,
                )],
                Some(&self.layer),
                t,
            ),
            t,
        );
        let baa_section = hipaa.then(|| {
            let disclaimer = if baa.signed {
                "You agreed to the <a>Business Associate and Healthcare Addendum</a>. You represent that you are a Covered Entity or Business Associate under HIPAA and have authority to enter into this agreement."
            } else {
                "By clicking Agree, you agree to the <a>Business Associate and Healthcare Addendum</a>. You represent that you are a Covered Entity or Business Associate under HIPAA and have authority to enter into this agreement."
            };
            let control = if baa.signed {
                view_link("agreements-baa-view", BAA, t)
            } else {
                action("agreements-agree", "Agree", Kind::Secondary, self.signing, t)
                    .when(!self.signing, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| this.agree(cx)))
                    })
                    .into_any_element()
            };
            let mut rows = vec![
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(24.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .max_w(px(510.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(linked_styled(
                                "agreements-baa-disclaimer",
                                disclaimer,
                                BAA.into(),
                                13.,
                                t.text,
                                UNDERLINED,
                            ))
                            .child(linked_styled(
                                "agreements-baa-help",
                                "For a list of HIPAA eligible products and functionality, refer to <a>this help center article</a>.",
                                HIPAA_HELP.into(),
                                12.,
                                t.text_secondary,
                                UNDERLINED,
                            )),
                    )
                    .child(div().flex_none().child(control)),
            ];
            if baa.signed {
                rows.push(settings_row_content(
                    "BAA was agreed on",
                    None,
                    text(baa.agreed_on(), 13., 18., 430., t.text_secondary)
                        .debug_selector(|| "agreements-baa-agreed-on".into())
                        .into_any_element(),
                    t,
                ));
            }
            section(
                "Business Associate Agreement (BAA)",
                Some("Available for HIPAA-eligible workspaces."),
                card(rows, None, t),
                t,
            )
            .when(self.sign_failed, |d| {
                d.child(
                    status("Couldn't agree to the BAA. Please try again.", true, t)
                        .debug_selector(|| "agreements-sign-error".into()),
                )
            })
        });
        div()
            .pt(px(8.))
            .flex()
            .flex_col()
            .gap(px(40.))
            .child(services)
            .children(baa_section)
            .when(self.refetch_failed, |d| d.child(self.load_error(t, cx)))
    }
}

/// The original's inline links (`underline: "always"`): the text color, underlined.
const UNDERLINED: HighlightStyle = HighlightStyle {
    color: None,
    font_weight: None,
    font_style: None,
    background_color: None,
    underline: Some(UnderlineStyle {
        thickness: px(1.),
        color: None,
        wavy: false,
    }),
    strikethrough: None,
    fade_out: None,
};

/// A section: its title, an optional subtitle and the card.
fn section(title: &'static str, subtitle: Option<&'static str>, body: Div, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(text(title, 16., 25., 500., t.text))
                .children(subtitle.map(|s| text(s, 13., 18., 430., t.text_secondary))),
        )
        .child(body)
}

/// The underlined `View` link that opens `url` in the browser.
fn view_link(id: &'static str, url: &'static str, t: &Theme) -> AnyElement {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .cursor_pointer()
        .child(text("View", 16., 24., 430., t.text).underline())
        .on_click(move |_, _, cx| cx.open_url(url))
        .into_any_element()
}

impl Render for Agreements {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let profile = cx
            .try_global::<account::State>()
            .map(|s| s.profile.clone())
            .unwrap_or_default();
        let content = match (&profile, &self.load) {
            (Profiled::Loading, _) => status("Loading…", false, &t),
            (Profiled::Failed(_), _) => self.load_error(&t, cx),
            // The original's `denied`: the title only.
            (Profiled::Ready(p), _) if !p.agreements() => div(),
            (Profiled::Ready(_), Load::Loading) => status("Loading…", false, &t),
            (Profiled::Ready(_), Load::Failed) => self.load_error(&t, cx),
            (Profiled::Ready(p), Load::Ready(baa)) => self.view(&baa.clone(), p.hipaa, &t, cx),
        };
        frame(
            "agreements-page",
            SharedString::from("Agreements"),
            Vec::new(),
            content,
            None,
            &self.layer,
            window,
            &t,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn signed_follows_the_flag_else_the_timestamp() {
        let baa = |meta: Value| Baa::from_metadata(&json!({"workspace_baa_metadata": meta}));
        assert!(!baa(json!(null)).signed);
        assert!(!baa(json!({"timestamp": ""})).signed);
        assert!(baa(json!({"timestamp": "2026-11-25T10:00:00Z"})).signed);
        assert!(!baa(json!({"signed": false, "timestamp": "2026-11-25T10:00:00Z"})).signed);
        assert!(baa(json!({"signed": true})).signed);
    }

    #[test]
    fn agreed_on_falls_back_to_the_raw_value() {
        let at = |t: Value| {
            Baa {
                signed: true,
                timestamp: Some(t),
            }
            .agreed_on()
        };
        assert_eq!(at(json!("soon")), "soon");
        assert_eq!(
            Baa {
                signed: true,
                timestamp: None
            }
            .agreed_on(),
            "Not available"
        );
        let local = at(json!("2026-11-25T10:00:00"));
        assert!(local.starts_with("11/25/2026, 10:00:00"), "{local}");
        assert!(at(json!("2026-11-25T10:00:00Z")).contains("/2026, "));
        assert!(at(json!(1_764_064_800_000_i64)).contains("/2025, "));
    }
}
