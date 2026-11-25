//! Settings ▸ Teams (`settings.teams`): the workspace's teams from the ChatGPT backend
//! (`/accounts/{account_id}/teams`). List the teams, create one and edit its name and
//! description, and open a team on its Activity and Members tabs. The app installs [`State`]
//! and answers its [`Action`]s.
mod activity;

use super::{
    Layer,
    account_page::{Kind, action, frame, paragraph, status},
    description, settings_row_content,
    worktrees::card,
};
use crate::{composer::Composer, screens::computer_use::Load, text};
pub use activity::Activity;
use gpui::{
    AnyElement, App, AppContext as _, BorrowAppContext as _, Context, Entity, Focusable as _,
    Global, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use std::sync::Arc;
use theme::{ActiveTheme as _, Theme};

/// One `GET /accounts/{account_id}/teams` item.
#[derive(Clone, Debug, PartialEq)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub description: String,
}

impl Team {
    /// `None` for an item without an id or a name.
    pub fn from_value(item: &Value) -> Option<Self> {
        Some(Self {
            id: item["id"].as_str()?.to_owned(),
            name: item["name"].as_str()?.to_owned(),
            description: item["description"].as_str().unwrap_or_default().to_owned(),
        })
    }
}

/// One `GET …/teams/{team_id}/members` item.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    /// `display_name ?? email ?? principal_id`, as the original's member row.
    pub name: String,
    pub email: Option<String>,
    pub owner: bool,
}

impl Member {
    pub fn from_value(item: &Value) -> Option<Self> {
        let id = item["principal_id"].as_str()?;
        let email = item["email"].as_str().map(str::to_owned);
        Some(Self {
            name: item["display_name"]
                .as_str()
                .or(email.as_deref())
                .unwrap_or(id)
                .to_owned(),
            email,
            owner: item["role"] == "owner",
        })
    }
}

/// The open team's members and activity log.
#[derive(Clone, Default)]
pub struct Detail {
    pub team_id: String,
    pub members: Load<Vec<Member>>,
    pub activity: Load<Vec<Activity>>,
    /// The next activity page; `None` when the log is complete.
    pub cursor: Option<String>,
    pub loading_more: bool,
}

#[derive(Clone, Debug)]
pub enum Action {
    /// Reads whether teams exist for the account and, if so, the list.
    Refresh,
    /// `POST /accounts/{account_id}/teams`.
    Create { name: String, description: String },
    /// `PATCH /accounts/{account_id}/teams/{team_id}`.
    Edit {
        id: String,
        name: String,
        description: String,
    },
    /// Reads a team's members and its first activity page into [`State::detail`].
    Open { id: String },
    /// `GET …/activities?cursor=` for the open team's next page.
    MoreActivity,
}

pub type Dispatch = Arc<dyn Fn(Action, &mut App) + Send + Sync>;

#[derive(Clone, Default)]
pub struct State {
    /// A workspace account with the teams gate: the nav item shows.
    pub available: bool,
    pub teams: Load<Vec<Team>>,
    pub saving: bool,
    /// The last save's error, shown in the open form.
    pub error: Option<String>,
    /// The open team, by `Action::Open`.
    pub detail: Option<Detail>,
    pub dispatch: Option<Dispatch>,
}
impl Global for State {}

pub fn dispatch(action: Action, cx: &mut App) {
    if let Some(dispatch) = cx.try_global::<State>().and_then(|s| s.dispatch.clone()) {
        dispatch(action, cx);
    }
}

pub fn available(cx: &App) -> bool {
    cx.try_global::<State>()
        .is_some_and(|state| state.available)
}

pub fn render(window: &mut Window, cx: &mut App) -> AnyElement {
    window
        .use_keyed_state("teams-settings", cx, |_, cx| {
            cx.observe_global::<State>(|this: &mut Teams, cx| {
                let state = cx.try_global::<State>();
                if this.submitted && state.is_some_and(|state| !state.saving) {
                    this.submitted = false;
                    if state.is_some_and(|state| state.error.is_none()) {
                        this.form = None;
                    }
                }
                cx.notify();
            })
            .detach();
            dispatch(Action::Refresh, cx);
            Teams {
                form: None,
                submitted: false,
                open: None,
                tab: Tab::Activity,
                layer: Layer::default(),
            }
        })
        .into_any_element()
}

struct Form {
    /// The team being edited; `None` creates one.
    id: Option<String>,
    name: Entity<Composer>,
    description: Entity<Composer>,
    empty_name: bool,
}

/// The open team's tabs, as the original's `settings.teams.tabs.*`.
// shortcut: the Shared tab (`shared`, the original's default) needs the sites, tasks, spaces,
// scheduled-task and plugin queries; it lands with #1613's next slice.
#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Activity,
    Members,
}

struct Teams {
    form: Option<Form>,
    submitted: bool,
    /// The team whose detail page shows.
    open: Option<Team>,
    tab: Tab,
    layer: Layer,
}

impl Teams {
    fn open_form(&mut self, team: Option<&Team>, window: &mut Window, cx: &mut Context<Self>) {
        let field = |placeholder, label, value: &str, cx: &mut Context<Self>| {
            cx.new(|cx| Composer::settings_compact_field(placeholder, value, cx).field_label(label))
        };
        let (name, description) = team.map_or(("", ""), |t| (&t.name, &t.description));
        let name = field("Team name", "Name", name, cx);
        let description = field(
            "Description—what does your team work on?",
            "Description",
            description,
            cx,
        );
        window.focus(&name.focus_handle(cx), cx);
        self.form = Some(Form {
            id: team.map(|t| t.id.clone()),
            name,
            description,
            empty_name: false,
        });
        if cx.has_global::<State>() {
            cx.update_global::<State, _>(|state, _| state.error = None);
        }
        cx.notify();
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else { return };
        if cx.try_global::<State>().is_some_and(|state| state.saving) {
            return;
        }
        let name = form.name.read(cx).field_text().trim().to_owned();
        let description = form.description.read(cx).field_text().trim().to_owned();
        form.empty_name = name.is_empty();
        if form.empty_name {
            cx.notify();
            return;
        }
        self.submitted = true;
        let id = form.id.clone();
        dispatch(
            match id {
                Some(id) => Action::Edit {
                    id,
                    name,
                    description,
                },
                None => Action::Create { name, description },
            },
            cx,
        );
        cx.notify();
    }

    fn form_view(
        &self,
        form: &Form,
        state: &State,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let field = |label: &'static str, editor: &Entity<Composer>| {
            div()
                .debug_selector(move || format!("teams-field-{label}"))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(text(label, 13., 18., 500., t.text))
                .child(editor.clone())
        };
        let (title, submit) = if form.id.is_some() {
            ("Edit team", "Save changes")
        } else {
            ("Create a team", "Create")
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .p(px(16.))
            .child(text(title, 16., 22., 500., t.text))
            .child(field("Name", &form.name))
            .when(form.empty_name, |d| {
                d.child(status("Enter a team name", true, t))
            })
            .child(field("Description", &form.description))
            .children(state.error.clone().map(|message| status(message, true, t)))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        action("teams-cancel", "Cancel", Kind::Secondary, false, t).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.form = None;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        action("teams-save", submit, Kind::Primary, state.saving, t)
                            .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
                    ),
            );
        div().child(card(vec![body], Some(&self.layer), t))
    }

    fn list_view(&self, teams: &[Team], t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        if teams.is_empty() {
            return div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(8.))
                .child(text("No teams yet", 16., 22., 500., t.text))
                .child(paragraph(
                    "Bring people together to share chats, spaces, and scheduled tasks",
                    t,
                ));
        }
        let rows = teams
            .iter()
            .enumerate()
            .map(|(index, team)| {
                let edit = team.clone();
                let open = team.clone();
                let detail = (!team.description.is_empty())
                    .then(|| description(&team.description, t).into_any_element());
                settings_row_content(
                    team.name.clone(),
                    detail,
                    action(
                        format!("teams-edit-{index}"),
                        "Edit",
                        Kind::Secondary,
                        false,
                        t,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_form(Some(&edit), window, cx)
                    }))
                    .into_any_element(),
                    t,
                )
                .id(format!("teams-row-{index}"))
                .debug_selector(move || format!("teams-row-{index}"))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.open_team(open.clone(), cx)))
            })
            .map(|row| div().child(row))
            .collect();
        div().child(card(rows, Some(&self.layer), t))
    }

    /// `/settings/teams/:teamId`: the original's list row navigates there.
    fn open_team(&mut self, team: Team, cx: &mut Context<Self>) {
        dispatch(
            Action::Open {
                id: team.id.clone(),
            },
            cx,
        );
        self.open = Some(team);
        self.tab = Tab::Activity;
        cx.notify();
    }

    fn tabs(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        div()
            .id("teams-tabs")
            .role(gpui::Role::TabList)
            .aria_label("Team sections")
            .flex()
            .gap(px(4.))
            .children(
                [(Tab::Activity, "Activity"), (Tab::Members, "Members")].map(|(tab, label)| {
                    let selected = self.tab == tab;
                    let id = format!("teams-tab-{}", label.to_lowercase());
                    div()
                        .id(SharedString::from(id.clone()))
                        .debug_selector(move || id.clone())
                        .role(gpui::Role::Tab)
                        .aria_selected(selected)
                        .h(px(36.))
                        .px(px(16.))
                        .flex()
                        .items_center()
                        .rounded_full()
                        .cursor_pointer()
                        .when(selected, |d| d.bg(t.ghost_hover))
                        .when(!selected, |d| d.hover(|s| s.bg(t.ghost_hover.opacity(0.5))))
                        .child(text(
                            label,
                            13.,
                            18.571_428,
                            500.,
                            if selected { t.text } else { t.text_secondary },
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.tab = tab;
                            cx.notify();
                        }))
                }),
            )
    }

    fn members_view(
        &self,
        members: &Load<Vec<Member>>,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let members = match members {
            Load::Loading => return div().child(status("Loading members…", false, t)),
            Load::Failed(_) => return self.retry("Unable to load members", t, cx),
            Load::Ready(members) if members.is_empty() => {
                return div().child(status("No members yet", false, t));
            }
            Load::Ready(members) => members,
        };
        let count = match members.len() {
            1 => "1 member".to_owned(),
            n => format!("{n} members"),
        };
        let rows = members
            .iter()
            .map(|member| {
                let owner = member.owner.then(|| {
                    div()
                        .flex_none()
                        .px(px(6.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(t.border)
                        .child(text("Owner", 12., 16., 500., t.text_secondary))
                });
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .px(px(16.))
                    .py(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(text(member.name.clone(), 13., 18.571_428, 500., t.text))
                            .children(owner),
                    )
                    .children(member.email.as_deref().map(|email| description(email, t)))
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(status(count, false, t))
            .child(card(rows, Some(&self.layer), t))
    }

    fn activity_view(&self, detail: &Detail, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let items = match &detail.activity {
            Load::Loading => return div().child(status("Loading activity…", false, t)),
            Load::Failed(_) => return self.retry("Team activity could not be loaded", t, cx),
            Load::Ready(items) if items.is_empty() && detail.cursor.is_none() => {
                return div().child(status("No activity yet", false, t));
            }
            Load::Ready(items) => items,
        };
        let now = chrono::Local::now();
        let mut days: Vec<(chrono::NaiveDate, Vec<&Activity>)> = Vec::new();
        for item in items {
            let Some(day) = local(&item.created_at).map(|time| time.date_naive()) else {
                continue;
            };
            match days.last_mut() {
                Some((last, group)) if *last == day => group.push(item),
                _ => days.push((day, vec![item])),
            }
        }
        let groups = days.into_iter().map(|(day, group)| {
            let rows = group
                .into_iter()
                .map(|item| {
                    let time = local(&item.created_at)
                        .map(|time| time.format("%-I:%M %p").to_string())
                        .unwrap_or_default();
                    div()
                        .flex()
                        .items_start()
                        .justify_between()
                        .gap(px(24.))
                        .px(px(16.))
                        .py(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(
                                    text(item.message.clone(), 13., 18.571_428, 500., t.text)
                                        .whitespace_normal(),
                                )
                                .children(item.details.iter().map(|line| description(line, t))),
                        )
                        .child(text(time, 12., 16., 430., t.text_secondary).flex_none())
                })
                .collect();
            div()
                .flex()
                .flex_col()
                .child(div().px(px(16.)).pb(px(8.)).child(text(
                    day_label(day, now.date_naive()),
                    14.,
                    20.,
                    430.,
                    t.text_secondary,
                )))
                .child(card(rows, Some(&self.layer), t))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .children(groups)
            .when(detail.cursor.is_some(), |d| {
                d.child(
                    div().flex().justify_center().child(
                        action(
                            "teams-activity-more",
                            "Load more",
                            Kind::Secondary,
                            detail.loading_more,
                            t,
                        )
                        .on_click(cx.listener(|_, _, _, cx| dispatch(Action::MoreActivity, cx))),
                    ),
                )
            })
    }

    /// An error line and Retry, which reopens the team.
    fn retry(&self, message: &'static str, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(12.))
            .child(status(message, true, t))
            .child(
                action("teams-detail-retry", "Retry", Kind::Secondary, false, t).on_click(
                    cx.listener(|this, _, _, cx| {
                        if let Some(team) = this.open.clone() {
                            this.open_team(team, cx);
                        }
                    }),
                ),
            )
    }

    fn detail_view(
        &self,
        team: &Team,
        state: &State,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let back = action("teams-back", "Teams", Kind::Secondary, false, t).on_click(cx.listener(
            |this, _, _, cx| {
                this.open = None;
                cx.notify();
            },
        ));
        let detail = state
            .detail
            .as_ref()
            .filter(|detail| detail.team_id == team.id)
            .cloned()
            .unwrap_or_default();
        let panel = match self.tab {
            Tab::Activity => self.activity_view(&detail, t, cx),
            Tab::Members => self.members_view(&detail.members, t, cx),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(div().flex().child(back))
            .child(self.tabs(t, cx))
            .child(panel)
    }
}

fn local(time: &str) -> Option<chrono::DateTime<chrono::Local>> {
    chrono::DateTime::parse_from_rfc3339(time)
        .ok()
        .map(|time| time.with_timezone(&chrono::Local))
}

/// `Fn`: `Today`, `Yesterday, Nov 24`, else the weekday and date, with the year when it differs.
fn day_label(day: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    use chrono::Datelike as _;
    if day == today {
        "Today".into()
    } else if today.pred_opt() == Some(day) {
        day.format("Yesterday, %b %-d").to_string()
    } else if day.year() == today.year() {
        day.format("%A, %b %-d").to_string()
    } else {
        day.format("%A, %b %-d, %Y").to_string()
    }
}

impl Render for Teams {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let state = cx.try_global::<State>().cloned().unwrap_or_default();
        if let Some(team) = self.open.clone() {
            // The page title and subtitle are the team's name and description, as `Hr`'s header.
            let intro = (!team.description.is_empty())
                .then(|| paragraph(team.description.clone(), &t))
                .into_iter()
                .collect();
            let content = self.detail_view(&team, &state, &t, cx);
            return frame(
                "teams-detail",
                team.name,
                intro,
                content,
                None,
                &self.layer,
                window,
                &t,
            );
        }
        let content = match (&state.teams, &self.form) {
            (_, Some(form)) => self.form_view(form, &state, &t, cx),
            (Load::Loading, _) => div().child(status("Loading…", false, &t)),
            (Load::Failed(_), _) => div()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(12.))
                .child(status("Unable to load teams", true, &t))
                .child(
                    action("teams-retry", "Retry", Kind::Secondary, false, &t)
                        .on_click(cx.listener(|_, _, _, cx| dispatch(Action::Refresh, cx))),
                ),
            (Load::Ready(teams), None) => {
                let list = self.list_view(teams, &t, cx);
                let create = action("teams-create", "Create team", Kind::Primary, false, &t)
                    .on_click(cx.listener(|this, _, window, cx| this.open_form(None, window, cx)));
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(div().flex().child(create))
                    .child(list)
            }
        };
        frame(
            "teams-page",
            "Teams",
            vec![paragraph(
                "Manage the teams you belong to in this workspace",
                &t,
            )],
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
    fn team_needs_id_and_name_and_defaults_description() {
        let item = json!({"id": "t1", "name": "Core", "avatar": null, "created_at": "x"});
        assert_eq!(
            Team::from_value(&item),
            Some(Team {
                id: "t1".into(),
                name: "Core".into(),
                description: String::new()
            })
        );
        assert_eq!(Team::from_value(&json!({"name": "x"})), None);
        assert_eq!(Team::from_value(&json!({"id": "x"})), None);
    }

    #[test]
    fn member_name_falls_back_to_email_then_id() {
        let member = |v| Member::from_value(&v).map(|m| (m.name, m.owner));
        assert_eq!(
            member(json!({"principal_id": "u1", "email": "a@x", "role": "owner"})),
            Some(("a@x".into(), true))
        );
        assert_eq!(
            member(json!({"principal_id": "u1"})),
            Some(("u1".into(), false))
        );
        assert_eq!(member(json!({"email": "a@x"})), None);
    }

    #[test]
    fn day_labels_follow_the_original() {
        let day = |s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let today = day("2026-11-25");
        assert_eq!(day_label(today, today), "Today");
        assert_eq!(day_label(day("2026-11-24"), today), "Yesterday, Nov 24");
        assert_eq!(day_label(day("2026-11-02"), today), "Monday, Nov 2");
        assert_eq!(
            day_label(day("2025-12-31"), today),
            "Wednesday, Dec 31, 2025"
        );
    }
}
