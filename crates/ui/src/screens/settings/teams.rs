//! Settings ▸ Teams (`settings.teams`): the workspace's teams from the ChatGPT backend
//! (`/accounts/{account_id}/teams`). List the teams, create one and edit its name and
//! description. The app installs [`State`] and answers its [`Action`]s.
use super::{
    Layer,
    account_page::{Kind, action, frame, paragraph, status},
    description, settings_row_content,
    worktrees::card,
};
use crate::{composer::Composer, screens::computer_use::Load, text};
use gpui::{
    AnyElement, App, AppContext as _, BorrowAppContext as _, Context, Entity, Focusable as _,
    Global, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder as _, px,
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

struct Teams {
    form: Option<Form>,
    submitted: bool,
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
                        this.open_form(Some(&edit), window, cx)
                    }))
                    .into_any_element(),
                    t,
                )
            })
            .collect();
        div().child(card(rows, Some(&self.layer), t))
    }
}

impl Render for Teams {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let state = cx.try_global::<State>().cloned().unwrap_or_default();
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
}
