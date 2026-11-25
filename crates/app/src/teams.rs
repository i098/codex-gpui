//! Drives `Settings → Teams` over the ChatGPT backend like the original's workspace team
//! queries: `GET|POST /accounts/{account_id}/teams`, `PATCH …/teams/{team_id}`, and a team's
//! `GET …/members` and `GET …/activities`.

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
    settings::teams::{Action, Activity, Detail, Member, State, Team},
};

/// The original's activity page size (`W8`).
const ACTIVITY_PAGE: usize = 20;

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

/// Every page of an offset-paged team list, as the original's `Jgi` reads it.
fn pages<T>(
    host: &dyn Host,
    identity: &Identity,
    rest: &[&str],
    item: fn(&Value) -> Option<T>,
) -> Result<Vec<T>> {
    let mut items = Vec::new();
    for offset in (0..).step_by(PAGE) {
        let url = format!("{}?offset={offset}&limit={PAGE}", path(identity, rest));
        let page = usage::http(host, identity, "GET", &url, None)?;
        items.extend(
            page["items"]
                .as_array()
                .context("teams page has no items")?
                .iter()
                .filter_map(item),
        );
        if page["has_more"] != true {
            return Ok(items);
        }
    }
    unreachable!("the offset range is unbounded")
}

fn list(host: &dyn Host, identity: &Identity) -> Result<Vec<Team>> {
    pages(host, identity, &[], Team::from_value)
}

/// One activity page after `cursor`, and the next page's cursor.
fn activities(
    host: &dyn Host,
    identity: &Identity,
    team: &str,
    cursor: Option<&str>,
) -> Result<(Vec<Activity>, Option<String>)> {
    let mut url = format!(
        "{}?limit={ACTIVITY_PAGE}",
        path(identity, &[team, "activities"])
    );
    if let Some(cursor) = cursor {
        url.push_str("&cursor=");
        url.extend(url::form_urlencoded::byte_serialize(cursor.as_bytes()));
    }
    let page = usage::http(host, identity, "GET", &url, None)?;
    let items = page["items"]
        .as_array()
        .context("activity page has no items")?
        .iter()
        .filter_map(Activity::from_value)
        .collect();
    Ok((items, page["cursor"].as_str().map(str::to_owned)))
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
            Action::Open { id } => {
                cx.update_global::<State, _>(|state, _| {
                    state.detail = Some(Detail {
                        team_id: id.clone(),
                        ..Default::default()
                    })
                });
                let team = id.clone();
                let members = cx.background_spawn(async move {
                    let identity = host.identity().context("Teams require a ChatGPT account")?;
                    pages(
                        host.as_ref(),
                        &identity,
                        &[team.as_str(), "members"],
                        Member::from_value,
                    )
                });
                cx.spawn({
                    let id = id.clone();
                    async move |_, cx| {
                        let result = members.await;
                        cx.update(|cx| {
                            update_detail(&id, cx, |detail| {
                                detail.members = match result {
                                    Ok(members) => Load::Ready(members),
                                    Err(error) => {
                                        eprintln!("team members: {error:#}");
                                        Load::Failed(error.to_string())
                                    }
                                }
                            })
                        });
                    }
                })
                .detach();
                self.team_activities(id, None, cx);
            }
            Action::MoreActivity => {
                let Some((id, Some(cursor))) = cx
                    .global::<State>()
                    .detail
                    .as_ref()
                    .filter(|d| !d.loading_more)
                    .map(|d| (d.team_id.clone(), d.cursor.clone()))
                else {
                    return;
                };
                update_detail(&id, cx, |detail| detail.loading_more = true);
                self.team_activities(id, Some(cursor), cx);
            }
        }
    }

    /// Reads an activity page and appends it to the open team's log.
    fn team_activities(&mut self, id: String, cursor: Option<String>, cx: &mut Context<Self>) {
        let host = self.host.clone();
        let task = cx.background_spawn({
            let id = id.clone();
            async move {
                let identity = host.identity().context("Teams require a ChatGPT account")?;
                activities(host.as_ref(), &identity, &id, cursor.as_deref())
            }
        });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            cx.update(|cx| {
                update_detail(&id, cx, |detail| {
                    detail.loading_more = false;
                    match result {
                        Ok((items, cursor)) => {
                            detail.cursor = cursor;
                            match &mut detail.activity {
                                Load::Ready(log) => log.extend(items),
                                activity => *activity = Load::Ready(items),
                            }
                        }
                        Err(error) => {
                            eprintln!("team activity: {error:#}");
                            // A failed Load more keeps the loaded log, as the query keeps its pages.
                            if !matches!(detail.activity, Load::Ready(_)) {
                                detail.activity = Load::Failed(error.to_string());
                            }
                        }
                    }
                })
            });
        })
        .detach();
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

/// Applies `f` to the detail when team `id` is still the open one.
fn update_detail(id: &str, cx: &mut App, f: impl FnOnce(&mut Detail)) {
    cx.update_global::<State, _>(|state, _| {
        if let Some(detail) = state.detail.as_mut().filter(|d| d.team_id == id) {
            f(detail);
        }
    });
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

    /// Answers each request with the next recorded body and keeps the requests.
    struct Pages(std::sync::Mutex<(Vec<Value>, Vec<String>)>);

    impl Host for Pages {
        fn identity(&self) -> Option<Identity> {
            None
        }

        fn fetch(&self, request: &crate::host::Request) -> Result<Value> {
            let mut pages = self.0.lock().expect("test lock");
            pages.1.push(format!("{request:?}"));
            let body = pages.0.remove(0);
            Ok(json!({"status": 200, "bodyJsonString": body.to_string()}))
        }
    }

    #[test]
    fn activity_pages_follow_the_cursor_and_members_follow_has_more() -> Result<()> {
        let identity = Identity {
            account_id: "ws".into(),
            user_id: "u".into(),
            device_id: "d".into(),
        };
        let event = |id: &str| {
            json!({"id": id, "created_at": "2026-11-25T09:00:00Z", "actor": {"id": "u1"},
                "activity_data": {"type": "TEAM_CREATE"}})
        };
        let host = Pages(std::sync::Mutex::new((
            vec![
                json!({"items": [event("a1"), {"id": "x", "activity_data": {"type": "NEW"}}], "cursor": "c/2"}),
                json!({"items": [event("a2")], "cursor": null}),
                json!({"items": [{"principal_id": "u1", "role": "owner"}], "has_more": true}),
                json!({"items": [{"principal_id": "u2"}], "has_more": false}),
            ],
            Vec::new(),
        )));
        let (first, cursor) = activities(&host, &identity, "t1", None)?;
        ensure!(first.len() == 1 && cursor.as_deref() == Some("c/2"));
        let (second, cursor) = activities(&host, &identity, "t1", cursor.as_deref())?;
        ensure!(second[0].id == "a2" && cursor.is_none());
        let members = pages(&host, &identity, &["t1", "members"], Member::from_value)?;
        ensure!(members.len() == 2 && members[0].owner);
        let requests = &host.0.lock().expect("test lock").1;
        ensure!(requests[0].contains("/accounts/ws/teams/t1/activities?limit=20"));
        ensure!(requests[1].contains("activities?limit=20&cursor=c%2F2"));
        ensure!(requests[3].contains("/teams/t1/members?offset=100&limit=100"));
        Ok(())
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
