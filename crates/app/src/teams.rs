//! Drives `Settings → Teams` over the ChatGPT backend like the original's workspace team
//! queries: `GET|POST /accounts/{account_id}/teams` and `PATCH …/teams/{team_id}`.

use crate::{
    Root,
    host::{Host, Identity},
    usage,
};
use anyhow::{Context as _, Result, ensure};
use gpui::{App, AppContext as _, BorrowAppContext as _, Context, WeakEntity};
use serde_json::{Value, json};
use std::sync::Arc;
use ui::screens::{
    computer_use::Load,
    settings::teams::{Action, State, Team},
};

/// The original's page size (`Zgi`).
const PAGE: usize = 100;
/// The original's Statsig gate for workspace teams (`z8`).
const TEAMS_GATE: &str = "853110088";

pub fn install(root: WeakEntity<Root>, cx: &mut App) {
    cx.set_global(State {
        // Deferred: the page refreshes while it is built, inside the root's own update.
        dispatch: Some(Arc::new(move |action, cx| {
            let root = root.clone();
            cx.defer(move |cx| {
                root.update(cx, |root, cx| root.teams_action(action, cx))
                    .ok();
            });
        })),
        ..Default::default()
    });
}

fn path(identity: &Identity, rest: &[&str]) -> String {
    let mut url = url::Url::parse("https://chatgpt.com/").expect("static URL");
    url.path_segments_mut()
        .expect("HTTP URL")
        .extend(["accounts", identity.account_id.as_str(), "teams"])
        .extend(rest);
    url.path().to_owned()
}

/// The workspace account with the teams gate on (`z8`): a workspace `structure` and the gate.
fn available(host: &dyn Host, identity: &Identity) -> Result<bool> {
    let accounts = usage::http(host, identity, "GET", "/accounts/check/v4-2023-04-27", None)?;
    let workspace =
        accounts["accounts"][&identity.account_id]["account"]["structure"] == "workspace";
    Ok(workspace && usage::statsig(host, identity)["feature_gates"][TEAMS_GATE]["value"] == true)
}

/// Every page of the team list, as the original's `Jgi` reads it.
fn list(host: &dyn Host, identity: &Identity) -> Result<Vec<Team>> {
    let mut teams = Vec::new();
    for offset in (0..).step_by(PAGE) {
        let url = format!("{}?offset={offset}&limit={PAGE}", path(identity, &[]));
        let page = usage::http(host, identity, "GET", &url, None)?;
        teams.extend(
            page["items"]
                .as_array()
                .context("teams page has no items")?
                .iter()
                .filter_map(Team::from_value),
        );
        if page["has_more"] != true {
            return Ok(teams);
        }
    }
    unreachable!("the offset range is unbounded")
}

/// The original's save error text by HTTP status (`edit.nameConflict`, `forbidden`, `error`).
fn save_error(status: u64) -> &'static str {
    match status {
        409 => "This team name already exists",
        403 => "You don't have permission to save this team",
        _ => "Unable to save this team",
    }
}

/// The body of a save: the name, and the description when it has one.
fn fields(name: &str, description: &str, editing: bool) -> Value {
    if description.is_empty() && !editing {
        json!({"name": name})
    } else {
        json!({"name": name, "description": description})
    }
}

fn save(
    host: &dyn Host,
    identity: &Identity,
    id: Option<&str>,
    body: Value,
) -> Result<std::result::Result<(), &'static str>> {
    let (method, url) = match id {
        Some(id) => ("PATCH", path(identity, &[id])),
        None => ("POST", path(identity, &[])),
    };
    let (status, _) = usage::send(host, identity, method, &url, Some(body))?;
    Ok(if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(save_error(status))
    })
}

fn snapshot(host: &dyn Host) -> Result<(bool, Vec<Team>)> {
    let identity = host.identity().context("Teams require a ChatGPT account")?;
    ensure!(
        available(host, &identity)?,
        "Teams are unavailable for this account"
    );
    Ok((true, list(host, &identity)?))
}

impl Root {
    fn teams_action(&mut self, action: Action, cx: &mut Context<Self>) {
        let host = self.host.clone();
        match action {
            Action::Refresh => {
                let task = cx.background_spawn(async move { snapshot(host.as_ref()) });
                cx.spawn(async move |_, cx| {
                    let result = task.await;
                    cx.update(|cx| {
                        cx.update_global::<State, _>(|state, _| match result {
                            Ok((available, teams)) => {
                                state.available = available;
                                state.teams = Load::Ready(teams);
                            }
                            Err(error) => {
                                eprintln!("teams: {error:#}");
                                state.available = false;
                                state.teams = Load::Failed(error.to_string());
                            }
                        })
                    });
                })
                .detach();
            }
            Action::Create { name, description } => {
                let body = fields(&name, &description, false);
                self.save_team(None, body, cx);
            }
            Action::Edit {
                id,
                name,
                description,
            } => {
                let body = fields(&name, &description, true);
                self.save_team(Some(id), body, cx);
            }
        }
    }

    fn save_team(&mut self, id: Option<String>, body: Value, cx: &mut Context<Self>) {
        if cx.global::<State>().saving {
            return;
        }
        cx.update_global::<State, _>(|state, _| {
            state.saving = true;
            state.error = None;
        });
        let host = self.host.clone();
        let task = cx.background_spawn(async move {
            let identity = host.identity().context("Teams require a ChatGPT account")?;
            let saved = save(host.as_ref(), &identity, id.as_deref(), body)?;
            // Reread after a save so the list shows the server's copy.
            let teams = saved
                .is_ok()
                .then(|| list(host.as_ref(), &identity))
                .transpose()?;
            anyhow::Ok((saved, teams))
        });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            cx.update(|cx| {
                cx.update_global::<State, _>(|state, _| {
                    state.saving = false;
                    match result {
                        Ok((saved, teams)) => {
                            state.error = saved.err().map(str::to_owned);
                            if let Some(teams) = teams {
                                state.teams = Load::Ready(teams);
                            }
                        }
                        Err(error) => {
                            eprintln!("teams save: {error:#}");
                            state.error = Some(save_error(0).into());
                        }
                    }
                })
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_encode_the_account_and_team() {
        let identity = Identity {
            account_id: "ws a/b".into(),
            user_id: "u".into(),
            device_id: "d".into(),
        };
        assert_eq!(path(&identity, &[]), "/accounts/ws%20a%2Fb/teams");
        assert_eq!(
            path(&identity, &["t 1"]),
            "/accounts/ws%20a%2Fb/teams/t%201"
        );
    }

    #[test]
    fn save_errors_follow_the_original_statuses() {
        assert_eq!(save_error(409), "This team name already exists");
        assert_eq!(
            save_error(403),
            "You don't have permission to save this team"
        );
        assert_eq!(save_error(500), "Unable to save this team");
    }

    #[test]
    fn create_omits_an_empty_description_but_edit_clears_it() {
        assert_eq!(fields("A", "", false), json!({"name": "A"}));
        assert_eq!(
            fields("A", "", true),
            json!({"name": "A", "description": ""})
        );
    }
}
