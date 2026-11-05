//! Drives `Settings → Account` over the ChatGPT backend: the profile (`/me`,
//! `/accounts/check`) and account deletion (`POST /accounts/deactivate`).

use crate::{Root, host::Host, usage};
use anyhow::{Context as _, Result};
use gpui::{App, AppContext as _, BorrowAppContext as _, Context, WeakEntity};
use serde_json::Value;
use std::sync::Arc;
use ui::screens::{
    computer_use::Load,
    settings::account::{Action, Profile, State},
};
use ui::toast::Toast;

pub fn install(root: WeakEntity<Root>, cx: &mut App) {
    cx.set_global(State {
        dispatch: Some(Arc::new(move |action, cx| {
            root.update(cx, |root, cx| root.account_action(action, cx))
                .ok();
        })),
        ..Default::default()
    });
}

#[derive(Debug, PartialEq)]
enum Deletion {
    Deleted,
    NeedsRecentLogin,
    Refused(String),
}

/// The original's `/accounts/deactivate` outcomes (`At` and the reauth check in `Dt`).
fn deletion(status: u64, body: &Value) -> Deletion {
    let code = usage::error_code(body);
    let stale = body["detail"]
        .as_str()
        .or(body["detail"]["message"].as_str())
        == Some("Stale login not recent enough for deactivation");
    match status {
        200..=299 => Deletion::Deleted,
        401 if code == Some("reauth_required") => Deletion::NeedsRecentLogin,
        403 if stale => Deletion::NeedsRecentLogin,
        403 if code == Some("scim_managed_account_deletion_forbidden") => Deletion::Refused(
            "Your account is managed by your organization. Contact your IT team to delete it"
                .into(),
        ),
        403 if code == Some("business_workspace_owner_account_deletion_forbidden") => {
            Deletion::Refused(match body["detail"]["workspace_name"].as_str() {
                Some(name) if !name.is_empty() => format!(
                    "To delete your account, add another owner to your ChatGPT Business workspace \"{name}\""
                ),
                _ => "To delete your account, add another owner to your ChatGPT Business workspace"
                    .into(),
            })
        }
        _ => Deletion::Refused("Failed to delete account. Please try again later".into()),
    }
}

fn profile(host: &dyn Host) -> Result<Profile> {
    let identity = host
        .identity()
        .context("Account settings require a ChatGPT account")?;
    let me = usage::http(host, &identity, "GET", "/me", None)?;
    let accounts = usage::http(
        host,
        &identity,
        "GET",
        "/accounts/check/v4-2023-04-27",
        None,
    )?;
    let account = &accounts["accounts"][&identity.account_id]["account"];
    let text = |value: &Value| value.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    Ok(Profile {
        plan: text(&account["plan_type"]).unwrap_or_default(),
        name: text(&me["name"]).unwrap_or_default(),
        email: text(&me["email"]),
        phone: text(&me["phone_number"]),
        personal: account["structure"] == "personal",
    })
}

impl Root {
    fn account_action(&mut self, action: Action, cx: &mut Context<Self>) {
        let host = self.host.clone();
        match action {
            Action::Refresh => {
                let task = cx.background_spawn(async move { profile(host.as_ref()) });
                cx.spawn(async move |_, cx| {
                    let result = task.await;
                    cx.update(|cx| {
                        cx.update_global::<State, _>(|state, _| {
                            state.profile = match result {
                                Ok(profile) => Load::Ready(profile),
                                Err(error) => Load::Failed(error.to_string()),
                            }
                        })
                    });
                })
                .detach();
            }
            Action::RefreshLogin => {
                // The desktop original sends the user to ChatGPT to sign in again.
                cx.open_url("https://chatgpt.com/#settings/Account");
                cx.update_global::<State, _>(|state, _| {
                    state.needs_recent_login = false;
                    state.close_revision += 1;
                });
            }
            Action::Delete => {
                let state = cx.global::<State>();
                if state.deleting || !matches!(&state.profile, Load::Ready(p) if p.personal) {
                    return;
                }
                cx.update_global::<State, _>(|state, _| state.deleting = true);
                let task = cx.background_spawn(async move {
                    let identity = host
                        .identity()
                        .context("Account deletion requires a ChatGPT account")?;
                    let (status, body) = usage::send(
                        host.as_ref(),
                        &identity,
                        "POST",
                        "/accounts/deactivate",
                        None,
                    )?;
                    Ok::<_, anyhow::Error>(deletion(status, &body))
                });
                cx.spawn(async move |this, cx| {
                    let result = task.await.unwrap_or_else(|error| {
                        eprintln!("accounts/deactivate: {error:#}");
                        Deletion::Refused("Failed to delete account. Please try again later".into())
                    });
                    this.update(cx, |this, cx| {
                        cx.update_global::<State, _>(|state, _| {
                            state.deleting = false;
                            match &result {
                                Deletion::Deleted => {
                                    state.deleted = true;
                                    state.close_revision += 1;
                                }
                                Deletion::NeedsRecentLogin => state.needs_recent_login = true,
                                Deletion::Refused(_) => {}
                            }
                        });
                        if let Deletion::Refused(message) = result {
                            this.toast(Toast::danger(message), cx);
                        }
                    })
                    .ok();
                })
                .detach();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deletion_outcomes_follow_the_original() {
        let code = |code: &str| json!({"detail": {"code": code, "workspace_name": "Acme"}});
        assert_eq!(deletion(204, &Value::Null), Deletion::Deleted);
        assert_eq!(
            deletion(401, &code("reauth_required")),
            Deletion::NeedsRecentLogin
        );
        assert_eq!(
            deletion(
                403,
                &json!({"detail": "Stale login not recent enough for deactivation"})
            ),
            Deletion::NeedsRecentLogin
        );
        assert_eq!(
            deletion(403, &code("business_workspace_owner_account_deletion_forbidden")),
            Deletion::Refused(
                "To delete your account, add another owner to your ChatGPT Business workspace \"Acme\""
                    .into()
            )
        );
        assert!(matches!(
            deletion(403, &code("scim_managed_account_deletion_forbidden")),
            Deletion::Refused(message) if message.starts_with("Your account is managed")
        ));
        assert!(matches!(
            deletion(401, &Value::Null),
            Deletion::Refused(message) if message.starts_with("Failed to delete")
        ));
    }
}
