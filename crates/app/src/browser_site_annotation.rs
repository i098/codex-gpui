#[cfg(target_os = "macos")]
use crate::chatgpt::Gate;
use crate::{Root, SidePanel, Toast, host_settings::Settings};
use gpui::{App, AppContext as _, Context, Entity, Window};
use serde_json::Value;
use ui::browser::{BrowserTab, SiteAnnotationRequest};
use ui::confirmation::{Confirmation, Outcome};

const ANSWERED: &str = "browser-site-annotation-permission-answered-hostnames";
const BLOCKED: &str = "browser-disabled-site-annotation-hostnames";

#[cfg(any(test, target_os = "macos"))]
fn enabled(rollout: bool, cli: Option<bool>, workspace: Option<bool>) -> bool {
    cli != Some(false)
        && workspace != Some(false)
        && (rollout || (cli == Some(true) && workspace == Some(true)))
}

#[cfg(target_os = "macos")]
pub(crate) async fn availability(bridge: &bridge::Bridge, gates: &Value) -> Option<bool> {
    if !Gate::BrowserAnnotationApi.enabled(gates) {
        return None;
    }
    let requirements = bridge.config_requirements().await.ok()?;
    let workspace =
        requirements["requirements"]["featureRequirements"]["browser_annotation_api"].as_bool();
    let mut cursor = None;
    let cli = loop {
        let page: Value = bridge
            .request(
                "experimentalFeature/list",
                serde_json::json!({"cursor": cursor, "limit": 100}),
            )
            .await
            .ok()?;
        if let Some(feature) = page["data"]
            .as_array()?
            .iter()
            .find(|feature| feature["name"] == "browser_annotation_api")
        {
            break feature["enabled"].as_bool();
        }
        let next = page["nextCursor"].as_str().map(str::to_owned);
        if next.is_none() {
            break None;
        }
        if next == cursor {
            return None;
        }
        cursor = next;
    };
    enabled(
        Gate::BrowserAnnotationApiRollout.enabled(gates),
        cli,
        workspace,
    )
    .then(|| Gate::BrowserAnnotationPermission.enabled(gates))
}

fn contains(values: Option<&Value>, host: &str) -> bool {
    values
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().any(|value| value == host))
}

fn permission(
    host: &str,
    answered: Option<&Value>,
    blocked: Option<&Value>,
    required: bool,
) -> Option<bool> {
    if contains(blocked, host) {
        Some(false)
    } else if !required || contains(answered, host) {
        Some(true)
    } else {
        None
    }
}

fn hostname(url: &url::Url) -> Option<&str> {
    let host = url.host_str()?;
    Some(host.strip_suffix('.').unwrap_or(host))
}

pub(crate) fn sync_blocked(tab: &Entity<BrowserTab>, cx: &mut App) {
    let blocked = tab.read(cx).url().and_then(hostname).is_some_and(|host| {
        cx.try_global::<Settings>()
            .is_some_and(|settings| contains(settings.read_all().get(BLOCKED), host))
    });
    tab.update(cx, |tab, _| tab.set_site_annotation_blocked(blocked));
}

/// Pushes the account's annotation gates into an open tab.
pub(crate) fn apply_gates(
    tab: &Entity<BrowserTab>,
    multi_select: bool,
    permission: Option<bool>,
    cx: &mut App,
) {
    sync_blocked(tab, cx);
    tab.update(cx, |tab, _| {
        tab.set_annotation_multi_select(multi_select);
        tab.set_site_annotation_api(permission);
    });
}

/// Applies the blocked-site lists and the gates to every tab that is already open.
pub(crate) fn sync_open_tabs(root: &mut Root, cx: &mut Context<Root>) {
    for (_, tab) in &root.browser_tabs {
        sync_blocked(tab, cx);
        root.refresh_annotation_gates(tab, cx);
    }
}

impl Root {
    /// Re-reads the gates in the background so a tab that is already open follows changes;
    /// runs when the tab opens and on each navigation.
    pub(crate) fn refresh_annotation_gates(
        &self,
        tab: &Entity<BrowserTab>,
        cx: &mut Context<Self>,
    ) {
        let host = self.host.clone();
        #[cfg(target_os = "macos")]
        let bridge = self.bridge.clone();
        let gate = cx.background_spawn(async move {
            let Some(identity) = host.identity() else {
                return (false, None);
            };
            let gates = crate::usage::statsig(host.as_ref(), &identity);
            let multi_select = crate::chatgpt::Gate::BrowserAnnotationMultiSelect.enabled(&gates);
            #[cfg(target_os = "macos")]
            let permission = availability(&bridge, &gates).await;
            #[cfg(not(target_os = "macos"))]
            let permission = None;
            (multi_select, permission)
        });
        let weak = tab.downgrade();
        cx.spawn(async move |_, cx| {
            let (multi_select, permission) = gate.await;
            if let Some(tab) = weak.upgrade() {
                cx.update(|cx| apply_gates(&tab, multi_select, permission, cx));
            }
        })
        .detach();
    }

    pub(crate) fn request_site_annotation(
        &mut self,
        key: usize,
        tab: &Entity<BrowserTab>,
        request: SiteAnnotationRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.side != Some(SidePanel::Browser(key))
            || self.has_dialog()
            || !tab.read(cx).accepts_site_annotation(request)
            || tab.read(cx).annotating() == request.active
        {
            return;
        }
        let page = tab.read(cx);
        let Some(host) = page.url().and_then(hostname) else {
            return;
        };
        let Some(settings) = cx.try_global::<Settings>() else {
            return;
        };
        let choice = {
            let values = settings.read_all();
            permission(
                host,
                values.get(ANSWERED),
                values.get(BLOCKED),
                page.site_annotation_permission_required(),
            )
        };
        if let Some(allow) = choice {
            if allow {
                tab.update(cx, |tab, cx| tab.apply_site_annotation(request, cx));
            }
            return;
        }
        let host = host.to_owned();
        let title = if request.active {
            "Let this site start annotation mode?"
        } else {
            "Let this site toggle annotation mode?"
        };
        let description = format!(
            "Annotation mode lets you leave comments for ChatGPT on the page. {host} can suggest comments and add context but can’t read or submit your prompts."
        );
        let dialog = cx.new(|cx| {
            Confirmation::new(title, description, "Allow", window, cx).site_annotation(window, cx)
        });
        cx.subscribe(&dialog, move |root, _, outcome, cx| {
            root.diff_confirmation = None;
            root.sync_browser_pages(cx);
            cx.notify();
            let allow = match outcome {
                Outcome::Confirm => true,
                Outcome::Alternate => false,
                Outcome::Cancel => return,
            };
            let Some(tab) = root.browser_tab(key).cloned() else {
                return;
            };
            if root.side != Some(SidePanel::Browser(key))
                || !tab.read(cx).accepts_site_annotation(request)
                || tab.read(cx).url().and_then(hostname) != Some(host.as_str())
            {
                return;
            }
            let written = cx.global_mut::<Settings>().update(
                if allow { ANSWERED } else { BLOCKED },
                |old| {
                    let mut hosts = old.and_then(Value::as_array).cloned().unwrap_or_default();
                    if !hosts.iter().any(|value| value == host.as_str()) {
                        hosts.push(Value::String(host.clone()));
                    }
                    Ok(Value::Array(hosts))
                },
            );
            if written.is_err() {
                root.toast(Toast::danger("Could not save your choice. Please try again."), cx);
            } else if allow {
                tab.update(cx, |tab, cx| tab.apply_site_annotation(request, cx));
            } else {
                for (_, tab) in &root.browser_tabs {
                    sync_blocked(tab, cx);
                }
                root.toast(Toast::info("Annotation features blocked for this site. You can still annotate the page yourself."), cx);
            }
        })
        .detach();
        self.diff_confirmation = Some(dialog);
        self.sync_browser_pages(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workspace_and_cli_denials_override_rollout() {
        assert!(!enabled(true, Some(false), Some(true)));
        assert!(!enabled(true, Some(true), Some(false)));
        assert!(!enabled(false, Some(true), None));
        assert!(enabled(false, Some(true), Some(true)));
        assert!(enabled(true, None, None));
    }

    #[test]
    fn block_wins_even_when_the_gate_skips_the_prompt() {
        let allowed = json!(["example.com", "other.test"]);
        let blocked = json!(["example.com"]);
        assert_eq!(
            permission("example.com", Some(&allowed), Some(&blocked), false),
            Some(false)
        );
        assert_eq!(
            permission("other.test", Some(&allowed), Some(&blocked), true),
            Some(true)
        );
        assert_eq!(
            permission("new.test", Some(&allowed), Some(&blocked), true),
            None
        );
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    #[gpui::test]
    fn open_tab_follows_a_changed_gate(cx: &mut gpui::TestAppContext) {
        let window = cx.add_empty_window();
        let tab = window.update(|window, cx| cx.new(|cx| BrowserTab::new(None, window, cx)));
        window.update(|_, cx| {
            assert!(tab.read(cx).site_annotation_permission_required());
            apply_gates(&tab, true, Some(false), cx);
            assert!(!tab.read(cx).site_annotation_permission_required());
            apply_gates(&tab, true, Some(true), cx);
            assert!(tab.read(cx).site_annotation_permission_required());
        });
    }
}
