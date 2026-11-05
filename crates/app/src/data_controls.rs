//! Drives `Settings → Data controls` over the ChatGPT backend: the account data export
//! (`POST /accounts/data_export`) and Marketing privacy (`/user_granular_consent`).

use crate::{
    Root,
    host::{Host, Identity},
    usage,
};
use anyhow::{Context as _, Result};
use gpui::{App, AppContext as _, BorrowAppContext as _, Context, WeakEntity};
use serde_json::{Value, json};
use std::sync::Arc;
use ui::screens::{
    computer_use::Load,
    settings::data_controls::{Action, Consent, Consents, State},
};
use ui::toast::Toast;

pub fn install(root: WeakEntity<Root>, cx: &mut App) {
    cx.set_global(State {
        dispatch: Some(Arc::new(move |action, cx| {
            root.update(cx, |root, cx| root.data_controls_action(action, cx))
                .ok();
        })),
        ..Default::default()
    });
}

/// The original's export error toasts by HTTP status and backend error code.
fn export_error(status: u64, body: &Value) -> &'static str {
    match (status, usage::error_code(body)) {
        (409, _) => {
            "Your existing export is still being processed. You’ll receive an email when it’s ready."
        }
        (401, _) | (_, Some("reauth_required" | "phone_verification_required")) => {
            "Please sign in again, then retry your data export."
        }
        (_, Some("email_required")) => {
            "Add and verify an email address in ChatGPT before requesting an export."
        }
        _ => "Couldn’t request your data export. Please try again later.",
    }
}

fn snapshot(host: &dyn Host) -> Result<(bool, bool, Load<Consents>)> {
    let identity = host
        .identity()
        .context("Data controls require a ChatGPT account")?;
    let accounts = usage::http(
        host,
        &identity,
        "GET",
        "/accounts/check/v4-2023-04-27",
        None,
    )?;
    let structure = &accounts["accounts"][&identity.account_id]["account"]["structure"];
    let export = structure.is_string() && structure != "workspace";
    let privacy = usage::statsig(host, &identity)["feature_gates"]["3157804138"]["value"] == true;
    let consents = if privacy {
        match usage::http(host, &identity, "GET", "/user_granular_consent", None) {
            Ok(value) => Consents::from_response(&value).map_or_else(
                || Load::Failed("Invalid marketing privacy settings".into()),
                Load::Ready,
            ),
            Err(error) => Load::Failed(error.to_string()),
        }
    } else {
        Load::Loading
    };
    Ok((export, privacy, consents))
}

fn export(host: &dyn Host, identity: &Identity) -> Result<std::result::Result<(), &'static str>> {
    let (status, body) = usage::send(host, identity, "POST", "/accounts/data_export", None)?;
    Ok(if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(export_error(status, &body))
    })
}

impl Root {
    fn data_controls_action(&mut self, action: Action, cx: &mut Context<Self>) {
        let host = self.host.clone();
        match action {
            Action::Refresh => {
                let task = cx.background_spawn(async move { snapshot(host.as_ref()) });
                cx.spawn(async move |_, cx| {
                    let result = task.await;
                    cx.update(|cx| {
                        cx.update_global::<State, _>(|state, _| match result {
                            Ok((export, privacy, consents)) => {
                                state.export_available = export;
                                state.privacy_available = privacy;
                                state.consents = consents;
                            }
                            Err(error) => {
                                state.export_available = false;
                                state.privacy_available = false;
                                state.consents = Load::Failed(error.to_string());
                            }
                        })
                    });
                })
                .detach();
            }
            Action::Export => {
                if cx.global::<State>().exporting {
                    return;
                }
                cx.update_global::<State, _>(|state, _| state.exporting = true);
                let task = cx.background_spawn(async move {
                    let identity = host
                        .identity()
                        .context("Data export requires a ChatGPT account")?;
                    export(host.as_ref(), &identity)
                });
                cx.spawn(async move |this, cx| {
                    let result = task.await;
                    this.update(cx, |this, cx| {
                        let requested = matches!(result, Ok(Ok(())));
                        cx.update_global::<State, _>(|state, _| {
                            state.exporting = false;
                            state.export_requested = requested;
                            state.export_revision += 1;
                        });
                        let toast = match result {
                            Ok(Ok(())) => Toast::success(
                                "Data export requested. You’ll receive an email when it’s ready.",
                            ),
                            Ok(Err(message)) => Toast::danger(message),
                            Err(error) => {
                                eprintln!("data export: {error:#}");
                                Toast::danger(
                                    "Couldn’t request your data export. Please try again later.",
                                )
                            }
                        };
                        this.toast(toast, cx);
                    })
                    .ok();
                })
                .detach();
            }
            Action::SetConsent(consent, on) => self.set_consent(consent, on, cx),
        }
    }

    fn set_consent(&mut self, consent: Consent, on: bool, cx: &mut Context<Self>) {
        let state = cx.global::<State>();
        if state.saving || !matches!(state.consents, Load::Ready(_)) {
            return;
        }
        cx.update_global::<State, _>(|state, _| state.saving = true);
        let host = self.host.clone();
        let task = cx.background_spawn(async move {
            let identity = host
                .identity()
                .context("Marketing privacy requires a ChatGPT account")?;
            usage::http(
                host.as_ref(),
                &identity,
                "POST",
                "/user_granular_consent",
                Some(json!({"granular_consent": {consent.key(): on}})),
            )
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                cx.update_global::<State, _>(|state, _| {
                    state.saving = false;
                    if result.is_ok()
                        && let Load::Ready(consents) = &mut state.consents
                    {
                        consents.set(consent, on);
                    }
                });
                if let Err(error) = result {
                    eprintln!("user_granular_consent: {error:#}");
                    this.toast(
                        Toast::danger("Couldn’t update marketing privacy settings"),
                        cx,
                    );
                }
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_errors_follow_the_original_precedence() {
        let code = |code: &str| json!({"detail": {"code": code}});
        assert!(export_error(409, &code("email_required")).starts_with("Your existing export"));
        assert!(export_error(401, &Value::Null).starts_with("Please sign in again"));
        assert!(export_error(403, &code("phone_verification_required")).starts_with("Please sign"));
        assert!(export_error(400, &code("email_required")).starts_with("Add and verify"));
        assert!(export_error(500, &Value::Null).starts_with("Couldn’t request"));
    }
}
