use crate::{Root, host};
use gpui::{AppContext as _, Context};
use serde_json::{Value, json};
use std::path::PathBuf;

impl Root {
    pub fn first_app_connected(
        &self,
        previous: Option<&[Value]>,
        next: &[Value],
        cx: &mut Context<Self>,
    ) {
        if !first_accessible_app(previous, next) {
            return;
        }
        let host = self.host.clone();
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let home = PathBuf::from(home);
        cx.background_spawn(async move {
            for domain in [None, Some("science")] {
                let request = host::Request::post(
                    "ambient-suggestions-refresh",
                    Some(json!({
                        "domain": domain,
                        "hostId": "local",
                        "projectRoot": home,
                        "mode": "first-plugin-connect"
                    })),
                );
                if let Err(error) = host
                    .fetch(&request)
                    .and_then(|response| host::json_body(&response))
                {
                    eprintln!("Failed to refresh first-app suggestions: {error}");
                }
            }
        })
        .detach();
    }
}

fn first_accessible_app(previous: Option<&[Value]>, next: &[Value]) -> bool {
    let active = |app: &Value| app["isAccessible"] == true && app["isEnabled"] == true;
    previous.is_some_and(|apps| !apps.iter().any(active)) && next.iter().any(active)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_app_refresh_requires_a_known_zero_to_one_transition() {
        let enabled = [json!({"isAccessible":true,"isEnabled":true})];
        let disabled = [json!({"isAccessible":true,"isEnabled":false})];
        let inaccessible = [json!({"isAccessible":false,"isEnabled":true})];
        assert!(!first_accessible_app(None, &enabled));
        assert!(first_accessible_app(Some(&[]), &enabled));
        assert!(first_accessible_app(Some(&disabled), &enabled));
        assert!(first_accessible_app(Some(&inaccessible), &enabled));
        assert!(!first_accessible_app(Some(&enabled), &enabled));
        assert!(!first_accessible_app(Some(&disabled), &disabled));
        assert!(!first_accessible_app(Some(&[]), &inaccessible));
    }
}
