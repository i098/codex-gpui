use crate::{Root, host, hotkey::policy, signin::ORIGINAL_APP_VERSION};
use anyhow::{Context as _, Result, ensure};
use bridge::types::Account;
use gpui::{AppContext as _, BorrowAppContext as _, Context, Task};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Runtime {
    pub account: Option<Account>,
    identity: Option<host::Identity>,
    pub task: Option<Task<()>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Status {
    Loading,
    Allowed,
    Denied,
    Error,
}

fn retryable(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    let status = message
        .strip_prefix("host HTTP status ")
        .or_else(|| message.strip_prefix("host error "))
        .and_then(|value| value.split([' ', ':']).next()?.parse::<u16>().ok());
    status.is_none_or(|status| matches!(status, 408 | 425 | 429 | 500..=599))
}

// Match the original query client's three transient retries.
async fn retry_read<T: Send + 'static>(
    executor: &gpui::BackgroundExecutor,
    mut read: impl Fn() -> Result<T> + Send + 'static,
) -> Result<T> {
    let mut attempt = 0;
    loop {
        let (result, next) = executor
            .spawn(async move {
                let result = read();
                (result, read)
            })
            .await;
        read = next;
        if attempt == 3 || !result.as_ref().is_err_and(retryable) {
            return result;
        }
        executor
            .timer(std::time::Duration::from_secs(1 << attempt))
            .await;
        attempt += 1;
    }
}

fn feature(host: &dyn host::Host, identity: Option<&host::Identity>) -> Result<(bool, Value)> {
    let response = if let Some(identity) = identity {
        policy::request(
            host,
            identity,
            "POST",
            "/wham/statsig/bootstrap",
            Some(json!({
                "app_session_id": host::device_id(), "stable_id": identity.device_id,
                "app_version": ORIGINAL_APP_VERSION, "brand_name": "chatgpt", "build_flavor": "prod",
                "locale": "en-US", "system_name": if cfg!(target_os = "macos") { "macOS" } else { std::env::consts::OS },
                "window_type": "electron"
            })),
        )?
    } else {
        let response = host.fetch(&host::Request::Http(json!({
            "url": "https://ab.chatgpt.com/v1/initialize?k=client-sYWqzCYMRkUg4DqqiZcR5DGTNl2iD7zNJY0HoeDLzxR&st=js-mono&sv=3.34.0",
            "method": "POST", "headers": {"Content-Type": "application/json"},
            "body": json!({"user":{"customIDs":{"stableID":host::device_id()}},"hash":"djb2","deltasResponseRequested":false,"full_checksum":null}).to_string(),
            "returnErrorResponse": true
        })))?;
        ensure!(
            host.identity().is_none(),
            "Activity account changed during the request"
        );
        host::json_body(&response)?
    };
    let payload = match response["statsigPayload"].as_str() {
        Some(payload) => serde_json::from_str(payload)?,
        None => response,
    };
    ensure!(
        payload["feature_gates"].is_object(),
        "Missing activity feature gates"
    );
    Ok((
        payload["feature_gates"]["4039078146"]["value"] == true,
        payload,
    ))
}

fn access(
    host: &dyn host::Host,
    identity: Option<&host::Identity>,
    account: &Account,
) -> Result<Status> {
    let Account::Chatgpt { plan_type, .. } = account else {
        return Ok(Status::Allowed);
    };
    ensure!(
        host.identity().as_ref() == identity,
        "Activity account changed before the access check"
    );
    ensure!(!plan_type.is_empty(), "Missing activity account plan");
    let identity = identity.context("Activity account identity is unavailable")?;
    if policy::consumer_plan(plan_type) {
        return Ok(Status::Allowed);
    }
    let mut url = url::Url::parse("https://chatgpt.com/").expect("static URL");
    url.path_segments_mut().expect("HTTP URL").extend([
        "accounts",
        identity.account_id.as_str(),
        "settings",
    ]);
    let settings = policy::request(host, identity, "GET", url.path(), None)?;
    Ok(workspace_access(&settings))
}

fn workspace_access(settings: &Value) -> Status {
    // The original combines local, work-local, and work-cloud access with allowed-first precedence.
    if settings["beta_settings"]["wham_local_access"] == true
        || settings["admin_work_local_enabled"] == true
        || settings["admin_work_mode_enabled"] == true
    {
        Status::Allowed
    } else {
        Status::Denied
    }
}

impl Root {
    fn apply_activity_access(
        &mut self,
        feature_enabled: bool,
        status: Status,
        cx: &mut Context<Self>,
    ) {
        let available = feature_enabled && matches!(status, Status::Loading | Status::Allowed);
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_activity_available(available, cx)
        });
        if let Some(menu) = &self.command_menu {
            menu.update(cx, |menu, cx| menu.set_activity_available(available, cx));
        }
        if cx.has_global::<ui::screens::settings::keyboard_shortcuts::KeyboardState>() {
            cx.update_global::<ui::screens::settings::keyboard_shortcuts::KeyboardState, _>(
                |state, _| {
                    if available {
                        state.active_commands.insert("togglePriorityFilter");
                    } else {
                        state.active_commands.remove("togglePriorityFilter");
                    }
                },
            );
        }
        cx.notify();
    }

    pub(super) fn clear_activity_access(&mut self, cx: &mut Context<Self>) {
        self.activity_access.task = None;
        self.activity_access.identity = None;
        self.apply_activity_access(false, Status::Denied, cx);
    }

    pub(super) fn refresh_activity_access(&mut self, cx: &mut Context<Self>) {
        self.activity_access.task = None;
        if self.popout.is_some() {
            return;
        }
        if self.bridge.is_acp() {
            self.apply_activity_access(true, Status::Allowed, cx);
            return;
        }
        let Some(account) = self.activity_access.account.clone() else {
            self.clear_activity_access(cx);
            return;
        };
        let host = self.host.clone();
        self.activity_access.task = Some(cx.spawn(async move |this, cx| {
            let source = host.clone();
            let identity = cx.background_spawn(async move { source.identity() }).await;
            if this
                .update(cx, |root, cx| {
                    if !Arc::ptr_eq(&root.host, &host) {
                        return false;
                    }
                    if root.activity_access.identity != identity {
                        root.apply_activity_access(false, Status::Denied, cx);
                    }
                    root.activity_access.identity = identity.clone();
                    true
                })
                .ok()
                != Some(true)
            {
                return;
            }
            let source = host.clone();
            let expected = identity.clone();
            let result = retry_read(cx.background_executor(), move || {
                feature(source.as_ref(), expected.as_ref())
            })
            .await;
            let (enabled, evaluations) = match result {
                Ok(result) => result,
                Err(error) => {
                    eprintln!("codex-gpui: cannot read activity feature: {error:#}");
                    this.update(cx, |root, cx| {
                        root.apply_activity_access(false, Status::Error, cx)
                    })
                    .ok();
                    return;
                }
            };
            if this
                .update(cx, |root, cx| {
                    if !Arc::ptr_eq(&root.host, &host) {
                        return false;
                    }
                    crate::security::load(root, &evaluations, cx);
                    true
                })
                .ok()
                != Some(true)
            {
                return;
            }
            if !enabled {
                this.update(cx, |root, cx| {
                    root.apply_activity_access(false, Status::Denied, cx)
                })
                .ok();
                return;
            }
            if this
                .update(cx, |root, cx| {
                    if !Arc::ptr_eq(&root.host, &host) {
                        return false;
                    }
                    root.apply_activity_access(true, Status::Loading, cx);
                    true
                })
                .ok()
                != Some(true)
            {
                return;
            }
            let result = retry_read(cx.background_executor(), move || {
                access(host.as_ref(), identity.as_ref(), &account)
            })
            .await;
            let status = result.unwrap_or_else(|error| {
                eprintln!("codex-gpui: cannot read activity access: {error:#}");
                Status::Error
            });
            this.update(cx, |root, cx| root.apply_activity_access(true, status, cx))
                .ok();
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_allowed_workspace_surface_enables_activity() {
        for settings in [
            json!({"beta_settings":{"wham_local_access":true}}),
            json!({"admin_work_local_enabled":true}),
            json!({"admin_work_mode_enabled":true}),
        ] {
            assert!(workspace_access(&settings) == Status::Allowed);
        }
        assert!(workspace_access(&json!({"admin_work_mode_enabled":false})) == Status::Denied);
    }

    struct PolicyHost {
        identity: host::Identity,
        body: Value,
        requests: std::sync::atomic::AtomicUsize,
    }

    impl host::Host for PolicyHost {
        fn identity(&self) -> Option<host::Identity> {
            Some(self.identity.clone())
        }

        fn fetch(&self, _: &host::Request) -> Result<Value> {
            self.requests
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(json!({"status":200,"body":self.body.to_string()}))
        }
    }

    fn policy_host(body: Value) -> PolicyHost {
        PolicyHost {
            identity: host::Identity {
                account_id: "workspace".into(),
                user_id: "user".into(),
                device_id: "device".into(),
            },
            body,
            requests: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    #[test]
    fn workspace_denial_and_account_changes_fail_closed() {
        let host = policy_host(json!({}));
        let mut account = Account::Chatgpt {
            email: None,
            plan_type: "business".into(),
        };
        assert_eq!(
            access(&host, Some(&host.identity), &account).unwrap(),
            Status::Denied
        );
        assert_eq!(host.requests.load(std::sync::atomic::Ordering::Relaxed), 1);
        account = Account::Chatgpt {
            email: None,
            plan_type: "plus".into(),
        };
        assert_eq!(
            access(&host, Some(&host.identity), &account).unwrap(),
            Status::Allowed
        );
        assert_eq!(host.requests.load(std::sync::atomic::Ordering::Relaxed), 1);
        let mut previous = host.identity.clone();
        previous.account_id = "previous-workspace".into();
        assert!(access(&host, Some(&previous), &account).is_err());
        assert_eq!(host.requests.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn feature_accepts_recorded_payloads_and_rejects_missing_gates() {
        let gates = json!({"feature_gates":{"4039078146":{"value":true}}});
        for body in [gates.clone(), json!({"statsigPayload":gates.to_string()})] {
            let host = policy_host(body);
            assert!(feature(&host, Some(&host.identity)).unwrap().0);
        }
        let host = policy_host(json!({}));
        assert!(feature(&host, Some(&host.identity)).is_err());
    }

    #[test]
    fn only_transient_http_failures_are_retried() {
        for status in [408, 425, 429, 500, 599] {
            assert!(retryable(&anyhow::anyhow!("host HTTP status {status}")));
        }
        for status in [400, 401, 403, 404] {
            assert!(!retryable(&anyhow::anyhow!("host HTTP status {status}")));
            assert!(!retryable(&anyhow::anyhow!("host error {status}: denied")));
        }
        assert!(retryable(&anyhow::anyhow!("connection failed")));
    }
}
