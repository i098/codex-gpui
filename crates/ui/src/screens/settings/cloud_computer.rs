//! Settings ▸ Cloud computer (`ChatGptCloudBrowserSettings`): the cloud browser's saved
//! cookies per site (`/flora/browser/cookies`, Clear all through `/agent/browser_context`) and
//! its passkeys (`/flora/browser/passkeys`, shown while `/flora/browser/config` has
//! `webauthn_enabled`).
use super::{
    Layer,
    account_page::{Kind, action, frame, paragraph, status},
    ads_controls::Load,
    backend::{self, Call},
    settings_row_content,
    worktrees::card,
};
use crate::{
    confirmation::{Confirmation, Outcome},
    icon::icon,
    text,
};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use theme::{ActiveTheme as _, Theme};

const COOKIES_LOAD_ERROR: &str = "Saved cookies couldn't be loaded";
const COOKIES_DELETE_ERROR: &str = "Could not delete browser data";
const PASSKEYS_LOAD_ERROR: &str = "Passkeys couldn't be loaded";
const PASSKEY_DELETE_ERROR: &str = "Failed to delete passkey";

/// One `/flora/browser/cookies` host.
#[derive(Clone, Debug, PartialEq)]
struct Host {
    hostname: String,
    cookies: u64,
}

/// The hosts that share one `registrable_domain`.
#[derive(Clone, Debug, PartialEq)]
struct Site {
    domain: String,
    hosts: Vec<Host>,
}

impl Site {
    /// The original's `groupBy(hosts, "registrable_domain")`, in first-seen order.
    fn group(body: &Value) -> Vec<Site> {
        let mut sites: Vec<Site> = Vec::new();
        for host in body["hosts"].as_array().into_iter().flatten() {
            let (Some(hostname), Some(domain)) = (
                host["hostname"].as_str(),
                host["registrable_domain"].as_str(),
            ) else {
                continue;
            };
            let host = Host {
                hostname: hostname.to_owned(),
                cookies: host["cookie_count"].as_u64().unwrap_or_default(),
            };
            match sites.iter_mut().find(|s| s.domain == domain) {
                Some(site) => site.hosts.push(host),
                None => sites.push(Site {
                    domain: domain.to_owned(),
                    hosts: vec![host],
                }),
            }
        }
        sites
    }

    /// The site lists its hosts under a toggle unless its only host is the domain itself.
    fn has_hosts(&self) -> bool {
        self.hosts.len() > 1 || self.hosts[0].hostname != self.domain
    }

    fn cookies(&self) -> u64 {
        self.hosts.iter().map(|h| h.cookies).sum()
    }
}

fn cookie_count(count: u64) -> String {
    match count {
        1 => "1 cookie".to_owned(),
        n => format!("{n} cookies"),
    }
}

/// What a cookie deletion removes.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    All,
    /// A hostname, with its subdomains for a site row.
    Host {
        hostname: String,
        subdomains: bool,
    },
}

impl Target {
    fn path(&self) -> String {
        match self {
            Target::All => "/agent/browser_context".to_owned(),
            Target::Host {
                hostname,
                subdomains,
            } => format!(
                "/flora/browser/cookies/{}?include_subdomains={subdomains}",
                encode(hostname)
            ),
        }
    }

    fn confirmation(&self) -> (String, &'static str) {
        match self {
            Target::All => (
                "Clear all cookies?".to_owned(),
                "Cookies saved by the cloud browser will be permanently deleted. Cookies in the browser on your device won't be affected.",
            ),
            Target::Host { hostname, .. } => (
                format!("Delete cookies for {hostname}?"),
                "Site data will be permanently deleted. You will be immediately logged out of all sessions, including those used by active conversations.",
            ),
        }
    }
}

/// One `/flora/browser/passkeys` item.
#[derive(Clone, Debug, PartialEq)]
struct Passkey {
    id: String,
    website: String,
    account: String,
}

impl Passkey {
    fn read(item: &Value) -> Option<Self> {
        let name = |key: &str| item[key].as_str().filter(|s| !s.is_empty());
        Some(Self {
            id: item["credential_id"].as_str()?.to_owned(),
            website: item["relying_party_id"].as_str()?.to_owned(),
            account: name("user_display_name")
                .or(name("user_name"))
                .unwrap_or_default()
                .to_owned(),
        })
    }
}

fn encode(segment: &str) -> String {
    url::form_urlencoded::byte_serialize(segment.as_bytes()).collect()
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("cloud-computer-settings", cx, CloudComputer::new)
        .into_any_element()
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    Main,
    Cookies,
}

pub struct CloudComputer {
    view: View,
    /// `webauthn_enabled` from `/flora/browser/config`; passkeys stay hidden until it is true.
    webauthn: bool,
    cookies: Load<Vec<Site>>,
    passkeys: Load<Vec<Passkey>>,
    /// Sites whose hosts are listed.
    expanded: Vec<String>,
    deleting: bool,
    confirmation: Option<Entity<Confirmation>>,
    layer: Layer,
}

impl CloudComputer {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let page = cx.entity();
        window.defer(cx, move |_, cx| page.update(cx, Self::load_config));
        Self {
            view: View::Main,
            webauthn: false,
            cookies: Load::Loading,
            passkeys: Load::Loading,
            expanded: Vec::new(),
            deleting: false,
            confirmation: None,
            layer: Layer::default(),
        }
    }

    fn load_config(&mut self, cx: &mut Context<Self>) {
        let call = Call::new("GET", "/flora/browser/config", None);
        backend::request(call, cx, |this, result, cx| {
            this.webauthn =
                matches!(&result, Ok(r) if r.ok() && r.body["webauthn_enabled"] == true);
            if this.webauthn {
                this.load_passkeys(cx);
            }
        });
    }

    fn load_passkeys(&mut self, cx: &mut Context<Self>) {
        self.passkeys = Load::Loading;
        let call = Call::new("GET", "/flora/browser/passkeys", None);
        backend::request(call, cx, |this, result, _| {
            this.passkeys = match result {
                Ok(r) if r.ok() => Load::Ready(
                    r.body["passkeys"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Passkey::read)
                        .collect(),
                ),
                _ => Load::Failed,
            };
        });
        cx.notify();
    }

    fn load_cookies(&mut self, cx: &mut Context<Self>) {
        self.cookies = Load::Loading;
        let call = Call::new("GET", "/flora/browser/cookies", None);
        backend::request(call, cx, |this, result, _| {
            this.cookies = match result {
                Ok(r) if r.ok() => Load::Ready(Site::group(&r.body)),
                _ => Load::Failed,
            };
        });
        cx.notify();
    }

    fn open(&mut self, view: View, cx: &mut Context<Self>) {
        self.view = view;
        if view == View::Cookies {
            self.load_cookies(cx);
        }
        cx.notify();
    }

    fn toggle(&mut self, domain: String, cx: &mut Context<Self>) {
        match self.expanded.iter().position(|d| *d == domain) {
            Some(i) => _ = self.expanded.remove(i),
            None => self.expanded.push(domain),
        }
        cx.notify();
    }

    /// Asks, then sends `DELETE`; the dialog stays open with `error` when the call fails.
    fn confirm(
        &mut self,
        title: String,
        description: String,
        call: Call,
        error: &'static str,
        done: fn(&mut Self, &mut Context<Self>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = cx.new(|cx| {
            Confirmation::new(title, description, "Delete", window, cx)
                .destructive()
                .settings_style()
        });
        cx.subscribe_in(&dialog, window, move |this, dialog, outcome, window, cx| {
            if *outcome != Outcome::Confirm {
                this.confirmation = None;
                cx.notify();
                return;
            }
            this.deleting = true;
            let dialog = dialog.downgrade();
            let window = window.window_handle();
            backend::request(call.clone(), cx, move |this, result, cx| {
                this.deleting = false;
                if result.is_ok_and(|r| r.ok()) {
                    this.confirmation = None;
                    done(this, cx);
                } else if let Some(dialog) = dialog.upgrade() {
                    window
                        .update(cx, |_, window, cx| {
                            dialog.update(cx, |d, cx| d.failed(error, window, cx))
                        })
                        .ok();
                }
            });
            cx.notify();
        })
        .detach();
        window.focus(&dialog.focus_handle(cx), cx);
        self.confirmation = Some(dialog);
        cx.notify();
    }

    fn delete_cookies(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        let (title, description) = target.confirmation();
        let call = Call::new("DELETE", target.path(), None);
        let done: fn(&mut Self, &mut Context<Self>) = Self::load_cookies;
        self.confirm(
            title,
            description.to_owned(),
            call,
            COOKIES_DELETE_ERROR,
            done,
            window,
            cx,
        );
    }

    fn delete_passkey(&mut self, passkey: Passkey, window: &mut Window, cx: &mut Context<Self>) {
        let description = format!(
            "Delete the passkey for {} on {}? You may lose access to this account.",
            passkey.account, passkey.website
        );
        let call = Call::new(
            "DELETE",
            format!("/flora/browser/passkeys/{}", encode(&passkey.id)),
            None,
        );
        let done: fn(&mut Self, &mut Context<Self>) = Self::load_passkeys;
        self.confirm(
            "Delete passkey?".to_owned(),
            description,
            call,
            PASSKEY_DELETE_ERROR,
            done,
            window,
            cx,
        );
    }

    /// A load error row with `Retry`.
    fn retry_row(
        id: &'static str,
        error: &'static str,
        retry: fn(&mut Self, &mut Context<Self>),
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        settings_row_content(
            error,
            None,
            action(id, "Retry", Kind::Secondary, false, t)
                .on_click(cx.listener(move |this, _, _, cx| retry(this, cx)))
                .into_any_element(),
            t,
        )
    }

    fn section(title: &'static str, body: impl IntoElement, t: &Theme) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(text(title, 16., 22., 500., t.text))
            .child(body)
    }

    fn main_view(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let cookies = div().child(
            div()
                .id("cloud-cookies-link")
                .debug_selector(|| "cloud-cookies-link".into())
                .cursor_pointer()
                .child(settings_row_content(
                    "ChatGPT Work cookies",
                    Some(paragraph("Manage cookies used by ChatGPT Work", t)),
                    icon("chevron-right", 16., t.text_tertiary).into_any_element(),
                    t,
                ))
                .on_click(cx.listener(|this, _, _, cx| this.open(View::Cookies, cx))),
        );
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(Self::section(
                "General",
                card(vec![cookies], Some(&self.layer), t),
                t,
            ))
            .when(self.webauthn, |d| {
                d.child(Self::section("Passkeys", self.passkeys_view(t, cx), t))
            })
    }

    fn passkeys_view(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let rows = match &self.passkeys {
            Load::Loading => return status("Loading…", false, t).into_any_element(),
            Load::Failed => vec![Self::retry_row(
                "cloud-passkeys-retry",
                PASSKEYS_LOAD_ERROR,
                Self::load_passkeys,
                t,
                cx,
            )],
            Load::Ready(passkeys) if passkeys.is_empty() => vec![settings_row_content(
                "No passkeys saved",
                None,
                div().into_any_element(),
                t,
            )],
            Load::Ready(passkeys) => passkeys
                .iter()
                .enumerate()
                .map(|(i, passkey)| {
                    let delete = action(
                        SharedString::from(format!("cloud-passkey-delete-{i}")),
                        "Delete passkey",
                        Kind::Danger,
                        self.deleting,
                        t,
                    )
                    .on_click({
                        let passkey = passkey.clone();
                        cx.listener(move |this, _, window, cx| {
                            this.delete_passkey(passkey.clone(), window, cx)
                        })
                    });
                    settings_row_content(
                        passkey.website.clone(),
                        Some(status(passkey.account.clone(), false, t).into_any_element()),
                        delete.into_any_element(),
                        t,
                    )
                })
                .collect(),
        };
        card(rows, Some(&self.layer), t).into_any_element()
    }

    fn site_row(
        &self,
        index: usize,
        site: &Site,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::Div> {
        let expanded = self.expanded.contains(&site.domain);
        let delete = |id: String, target: Target, cx: &mut Context<Self>| {
            div()
                .id(SharedString::from(id.clone()))
                .debug_selector(move || id.clone())
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .cursor_pointer()
                .hover(|d| d.bg(t.ghost_hover))
                .when(self.deleting, |d| d.opacity(0.5))
                .child(icon("shortcut-trash", 16., t.text_secondary))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if !this.deleting {
                        this.delete_cookies(target.clone(), window, cx);
                    }
                }))
        };
        let toggle = site.has_hosts().then(|| {
            let id = format!("cloud-site-hosts-{index}");
            let domain = site.domain.clone();
            div()
                .id(SharedString::from(id.clone()))
                .debug_selector(move || id.clone())
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .cursor_pointer()
                .hover(|d| d.bg(t.ghost_hover))
                .child(icon(
                    if expanded {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
                    16.,
                    t.text_secondary,
                ))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle(domain.clone(), cx)))
        });
        let site_target = Target::Host {
            hostname: site.domain.clone(),
            subdomains: true,
        };
        let controls = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .children(toggle)
            .child(delete(
                format!("cloud-site-delete-{index}"),
                site_target,
                cx,
            ));
        let mut rows = vec![settings_row_content(
            site.domain.clone(),
            Some(status(cookie_count(site.cookies()), false, t).into_any_element()),
            controls.into_any_element(),
            t,
        )];
        if expanded && site.has_hosts() {
            rows.extend(site.hosts.iter().enumerate().map(|(h, host)| {
                let target = Target::Host {
                    hostname: host.hostname.clone(),
                    subdomains: false,
                };
                div().pl(px(16.)).child(settings_row_content(
                    host.hostname.clone(),
                    Some(status(cookie_count(host.cookies), false, t).into_any_element()),
                    delete(format!("cloud-host-delete-{index}-{h}"), target, cx).into_any_element(),
                    t,
                ))
            }));
        }
        rows
    }

    fn cookies_view(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let back = div()
            .id("cloud-cookies-back")
            .debug_selector(|| "cloud-cookies-back".into())
            .flex()
            .items_center()
            .gap(px(4.))
            .cursor_pointer()
            .child(icon("chevron-left", 16., t.text_secondary))
            .child(text("Back", 13., 18., 500., t.text_secondary))
            .on_click(cx.listener(|this, _, _, cx| this.open(View::Main, cx)));
        let clear = settings_row_content(
            "Cookies",
            Some(paragraph("Remove cookies saved by the cloud browser", t)),
            action(
                "cloud-cookies-clear-all",
                "Clear all",
                Kind::Danger,
                self.deleting,
                t,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                if !this.deleting {
                    this.delete_cookies(Target::All, window, cx);
                }
            }))
            .into_any_element(),
            t,
        );
        let sites = match &self.cookies {
            Load::Loading => status("Loading…", false, t).into_any_element(),
            Load::Failed => card(
                vec![Self::retry_row(
                    "cloud-cookies-retry",
                    COOKIES_LOAD_ERROR,
                    Self::load_cookies,
                    t,
                    cx,
                )],
                Some(&self.layer),
                t,
            )
            .into_any_element(),
            Load::Ready(sites) if sites.is_empty() => card(
                vec![settings_row_content(
                    "No cookies saved",
                    None,
                    div().into_any_element(),
                    t,
                )],
                Some(&self.layer),
                t,
            )
            .into_any_element(),
            Load::Ready(sites) => {
                let rows = sites
                    .iter()
                    .enumerate()
                    .flat_map(|(i, site)| self.site_row(i, site, t, cx))
                    .collect();
                card(rows, Some(&self.layer), t).into_any_element()
            }
        };
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(back)
            .child(card(vec![clear], Some(&self.layer), t))
            .child(Self::section("Sites", sites, t))
    }
}

impl Render for CloudComputer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let (title, intro, content) = match self.view {
            View::Main => (
                "Cloud computer",
                vec![paragraph(
                    "Manage cloud browsing settings and saved website cookies",
                    &t,
                )],
                self.main_view(&t, cx),
            ),
            View::Cookies => ("Cookies", Vec::new(), self.cookies_view(&t, cx)),
        };
        frame(
            "cloud-computer-page",
            title,
            intro,
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
    use serde_json::json;

    #[test]
    fn hosts_group_by_registrable_domain_in_first_seen_order() {
        let host = |h: &str, d: &str, n: u64| json!({"hostname": h, "registrable_domain": d, "cookie_count": n});
        let sites = Site::group(&json!({"hosts": [
            host("example.com", "example.com", 2),
            host("acme.org", "acme.org", 1),
            host("www.example.com", "example.com", 3),
            {"hostname": "broken"},
        ]}));
        assert_eq!(
            sites.iter().map(|s| s.domain.as_str()).collect::<Vec<_>>(),
            ["example.com", "acme.org"]
        );
        assert_eq!(sites[0].cookies(), 5);
        assert!(sites[0].has_hosts());
        assert!(!sites[1].has_hosts());
        let sub = Site::group(&json!({"hosts": [host("app.acme.org", "acme.org", 1)]}));
        assert!(sub[0].has_hosts());
        assert_eq!(cookie_count(1), "1 cookie");
        assert_eq!(cookie_count(0), "0 cookies");
    }

    #[test]
    fn delete_paths_match_the_original() {
        assert_eq!(Target::All.path(), "/agent/browser_context");
        let host = |subdomains| Target::Host {
            hostname: "www.example.com".into(),
            subdomains,
        };
        assert_eq!(
            host(true).path(),
            "/flora/browser/cookies/www.example.com?include_subdomains=true"
        );
        assert_eq!(
            host(false).path(),
            "/flora/browser/cookies/www.example.com?include_subdomains=false"
        );
    }

    #[test]
    fn passkey_account_falls_back_to_user_name() {
        let p = |v| Passkey::read(&v).map(|p| p.account);
        assert_eq!(
            p(json!({"credential_id": "c", "relying_party_id": "x.com",
                "user_display_name": "", "user_name": "ada"})),
            Some("ada".into())
        );
        assert_eq!(
            p(json!({"credential_id": "c", "relying_party_id": "x.com",
                "user_display_name": "Ada", "user_name": "ada"})),
            Some("Ada".into())
        );
        assert_eq!(p(json!({"relying_party_id": "x.com"})), None);
    }
}
