//! `@` mention list (recorded in refs/interactions/s019.json). An empty query lists the `Add`
//! actions and the `Files` section; a query ranks actions, skills and the workspace files that
//! `fuzzyFileSearch/sessionStart` + `sessionUpdate` return (notifications `sessionUpdated`,
//! `sessionCompleted`). S020 owns the keyboard rows and the mention chip, S018 the `+` button.
//! `cargo test -p app --test s019` replays the original's list, requests and colors.
use super::slash::Trigger;
use super::*;
use crate::screens::settings::AppServer;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Action {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    icon: &'static str,
    /// `Work in a project` and `Plan mode` show on Home only (recordings 07 and s019).
    home_only: bool,
}

/// `composer.atMentionList.contextActions` rows, in the original's order.
const ACTIONS: &[Action] = &[
    Action {
        id: "files-and-folders",
        title: "Files and folders",
        description: "",
        icon: "folder",
        home_only: false,
    },
    Action {
        id: "appshot",
        title: "Attach appshot",
        description: "",
        icon: "settings-appshots",
        home_only: false,
    },
    Action {
        id: "project",
        title: "Work in a project",
        description: "Choose project for new chats",
        icon: "folder",
        home_only: true,
    },
    Action {
        id: "goal",
        title: "Goal",
        description: "Set a goal to keep pursuing",
        icon: "thread-summary",
        home_only: false,
    },
    Action {
        id: "plan-mode",
        title: "Plan mode",
        description: "Turn plan mode on",
        icon: "settings-personalization",
        home_only: true,
    },
    Action {
        id: "sketch",
        title: "Sketch",
        description: "Draw a sketch",
        icon: "pencil",
        home_only: false,
    },
];

/// One section of [`Composer::mention_state`]: heading and rows `(text, highlighted)`.
pub type MentionSection = (Option<&'static str>, Vec<(String, bool)>);

/// Rows of the ranked list (`oYc` caps at 8).
const MAX_RESULTS: usize = 8;
const EMPTY_QUERY: &str = "Type to search for files";
const SEARCHING: &str = "Searching…";
const NO_RESULTS: &str = "No results";

struct FileHit {
    name: String,
    /// Directory part of the path, shown after the name (`src`).
    dir: String,
    path: String,
}

/// One fuzzy file search session per open `@` token with a query.
#[derive(Default)]
pub(super) struct Search {
    roots: Vec<String>,
    session: Option<String>,
    query: String,
    files: Vec<FileHit>,
    searching: bool,
}

/// How well `query` (lowercase) matches `label`: 3 prefix, 2 word start, 1 inside, 0 none.
/// ponytail: substring ranks instead of the original's fuzzy subsequence scorer; the recorded
/// orders (prefix, then word start, then shorter labels) come out the same.
fn degree(label: &str, query: &str) -> u8 {
    let label = label.to_lowercase();
    if label.starts_with(query) {
        3
    } else if label
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| word.starts_with(query))
    {
        2
    } else if label.contains(query) {
        1
    } else {
        0
    }
}

/// Sends `requests` to the app-server in order, like the original awaits each; failures are
/// logged. Foreground tasks start in spawn order, so later calls keep the order too.
fn request(requests: Vec<(&'static str, Value)>, cx: &App) {
    let Some(AppServer(bridge)) = cx.try_global::<AppServer>() else {
        return;
    };
    let bridge = bridge.clone();
    cx.foreground_executor()
        .spawn(async move {
            for (method, params) in requests {
                if let Err(e) = bridge.request::<_, Value>(method, params).await {
                    eprintln!("{method}: {e:#}");
                }
            }
        })
        .detach();
}

impl Composer {
    /// Workspace roots the `Files` section searches; none hides the section (Home).
    pub fn set_search_roots(&mut self, roots: Vec<String>) {
        self.slash.search.roots = roots;
    }

    fn action_available(&self, action: &Action) -> bool {
        !(action.home_only && self.docked)
    }

    /// Fills `matches` for the `@` token with `query` (raw text after `@`) and drives the file
    /// search session.
    pub(super) fn refresh_mention(&mut self, query: &str, cx: &mut Context<Self>) {
        let docked = self.docked;
        let actions = ACTIONS
            .iter()
            .enumerate()
            .filter(move |(_, a)| !(a.home_only && docked) && a.id != "appshot");
        if query.is_empty() {
            self.slash.matches = actions.map(|(i, _)| i).collect();
            self.end_search(cx);
            return;
        }
        self.update_search(query, cx);
        let lower = query.to_lowercase();
        let skills = self.slash.skills.iter().enumerate().map(|(i, s)| {
            (
                ACTIONS.len() + i,
                2,
                degree(&s.title, &lower).max(degree(&s.name, &lower)),
                s.title.as_str(),
            )
        });
        let files = self.slash.search.files.iter().enumerate().map(|(i, f)| {
            (
                ACTIONS.len() + self.slash.skills.len() + i,
                3,
                degree(&f.name, &lower).max(degree(&f.path, &lower)),
                f.name.as_str(),
            )
        });
        let mut ranked: Vec<_> = actions
            .map(|(i, a)| (i, 2, degree(a.title, &lower), a.title))
            .chain(skills)
            .chain(files)
            .filter(|&(_, _, degree, _)| degree > 0)
            .collect();
        ranked
            .sort_by(|a, b| (a.1, b.2, a.3.len(), a.3, a.0).cmp(&(b.1, a.2, b.3.len(), b.3, b.0)));
        self.slash.matches = ranked.iter().take(MAX_RESULTS).map(|r| r.0).collect();
    }

    /// `sessionStart` on the first query of a token, then one `sessionUpdate` per query.
    fn update_search(&mut self, query: &str, cx: &App) {
        let search = &mut self.slash.search;
        if search.roots.is_empty() || search.query == query {
            return;
        }
        search.query = query.into();
        search.searching = true;
        let mut requests = Vec::new();
        let session = search.session.clone().unwrap_or_else(|| {
            // The server treats the id as opaque (the original sends a UUID); a counter
            // keeps the replayed notifications of `tests/s019.rs` addressable.
            static SESSIONS: AtomicUsize = AtomicUsize::new(1);
            let session = format!("fuzzy-{}", SESSIONS.fetch_add(1, Ordering::Relaxed));
            search.session = Some(session.clone());
            requests.push((
                "fuzzyFileSearch/sessionStart",
                json!({ "sessionId": session, "roots": search.roots }),
            ));
            session
        });
        requests.push((
            "fuzzyFileSearch/sessionUpdate",
            json!({ "sessionId": session, "query": query }),
        ));
        request(requests, cx);
    }

    /// `sessionStop` when the token closes.
    pub(super) fn end_search(&mut self, cx: &App) {
        let search = &mut self.slash.search;
        search.query.clear();
        search.files.clear();
        search.searching = false;
        if let Some(session) = search.session.take() {
            request(
                vec![(
                    "fuzzyFileSearch/sessionStop",
                    json!({ "sessionId": session }),
                )],
                cx,
            );
        }
    }

    /// `fuzzyFileSearch/sessionUpdated` (`completed == false`) replaces the files of the open
    /// session; `sessionCompleted` ends the loading state.
    pub fn apply_file_search(&mut self, completed: bool, params: &Value, cx: &mut Context<Self>) {
        let search = &mut self.slash.search;
        if params["sessionId"].as_str() != search.session.as_deref() {
            return;
        }
        if completed {
            search.searching = false;
        } else {
            search.files = params["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| {
                    let path = f["path"].as_str()?;
                    let name = f["file_name"].as_str().unwrap_or(path);
                    Some(FileHit {
                        name: name.into(),
                        dir: path.rsplit_once('/').map_or("", |(dir, _)| dir).into(),
                        path: path.into(),
                    })
                })
                .collect();
        }
        if self.slash.open && self.slash.trigger == Trigger::Mention {
            self.refresh_slash(cx);
        }
    }

    pub(super) fn mention_label(&self, index: usize) -> (&str, &str, &str) {
        if let Some(action) = ACTIONS.get(index) {
            (action.title, action.description, action.icon)
        } else if let Some(skill) = self.slash.skills.get(index - ACTIONS.len()) {
            (&skill.title, &skill.description, "sidebar-cmd-create-page")
        } else {
            let file = &self.slash.search.files[index - ACTIONS.len() - self.slash.skills.len()];
            (&file.name, &file.dir, "diff-jump-to-file")
        }
    }

    /// Enter, Tab or a click on the highlighted row. `Files and folders` opens the picker; a
    /// file inserts its path. ponytail: plain `@path ` text until S020 adds the mention chip;
    /// the other actions (goal, sketch, project, plan mode) belong to their own rows.
    pub(super) fn activate_mention(&mut self, index: usize, cx: &mut Context<Self>) {
        let range = self.slash.range.clone();
        if let Some(action) = ACTIONS.get(index) {
            if action.id != "files-and-folders" {
                return;
            }
            self.content.replace_range(range.clone(), "");
            self.selected_range = range.start..range.start;
            self.slash.open = false;
            self.slash.dismissed = None;
            self.end_search(cx);
            self.add_files(cx);
        } else if index >= ACTIONS.len() + self.slash.skills.len() {
            let file = &self.slash.search.files[index - ACTIONS.len() - self.slash.skills.len()];
            let text = format!("@{} ", file.path);
            self.content.replace_range(range.clone(), &text);
            let end = range.start + text.len();
            self.selected_range = end..end;
            self.slash.open = false;
            self.slash.dismissed = Some(range.start);
            self.end_search(cx);
        } else {
            return;
        }
        self.marked_range = None;
        cx.notify();
    }

    /// Sections `(heading, rows (text, highlighted))` of the open `@` list, for the tests.
    pub fn mention_state(&self) -> Option<Vec<MentionSection>> {
        if !self.slash.open || self.slash.trigger != Trigger::Mention {
            return None;
        }
        let row = |index: usize| {
            let (title, description, _) = self.mention_label(index);
            if description.is_empty() {
                title.to_owned()
            } else {
                format!("{title} | {description}")
            }
        };
        let current = |index| self.slash.matches.get(self.slash.selected) == Some(&index);
        if self.mention_query().is_empty() {
            let add = ACTIONS
                .iter()
                .enumerate()
                .filter(|(_, a)| self.action_available(a))
                .map(|(i, _)| (row(i), current(i)))
                .collect();
            let mut sections = vec![(Some("Add"), add)];
            if !self.slash.search.roots.is_empty() {
                sections.push((Some("Files"), vec![(EMPTY_QUERY.into(), false)]));
            }
            return Some(sections);
        }
        let rows: Vec<_> = if self.slash.matches.is_empty() {
            let text = if self.slash.search.searching {
                SEARCHING
            } else {
                NO_RESULTS
            };
            vec![(text.into(), false)]
        } else {
            self.slash
                .matches
                .iter()
                .map(|&i| (row(i), current(i)))
                .collect()
        };
        Some(vec![(None, rows)])
    }

    fn mention_query(&self) -> &str {
        &self.content[self.slash.range.start + 1..self.slash.range.end]
    }

    /// The list's content: sections with headings for an empty query, else the ranked rows.
    pub(super) fn mention_list(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let heading = |s: &'static str, first: bool| {
            text(s, 13., 18.5714, 400., t.text.opacity(0.5))
                .px(px(8.))
                .pt(px(if first { 4. } else { 8. }))
                .pb(px(4.))
        };
        let note = |s: &'static str| {
            div()
                .h(px(28.5625))
                .px(px(8.))
                .flex()
                .items_center()
                .child(text(s, 13., 18.5714, 400., t.text.opacity(0.5)))
        };
        let mut list = div().flex().flex_col();
        if self.mention_query().is_empty() {
            list = list.child(heading("Add", true));
            for (index, action) in ACTIONS.iter().enumerate() {
                if !self.action_available(action) {
                    continue;
                }
                let selected = self.slash.matches.iter().position(|&m| m == index);
                list = list.child(match selected {
                    Some(selected) => self
                        .suggestion_row(t, selected, index, self.mention_label(index), cx)
                        .into_any_element(),
                    // `Attach appshot`: aria-disabled, 50% text at half opacity.
                    None => div()
                        .h(px(28.5625))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .opacity(0.5)
                        .child(icon(action.icon, 16., t.text.opacity(0.5)))
                        .child(text(action.title, 13., 18.5714, 400., t.text.opacity(0.5)))
                        .into_any_element(),
                });
            }
            if !self.slash.search.roots.is_empty() {
                list = list.child(heading("Files", false)).child(note(EMPTY_QUERY));
            }
            return list;
        }
        if self.slash.matches.is_empty() {
            return list.child(note(if self.slash.search.searching {
                SEARCHING
            } else {
                NO_RESULTS
            }));
        }
        for (selected, &index) in self.slash.matches.iter().enumerate() {
            list =
                list.child(self.suggestion_row(t, selected, index, self.mention_label(index), cx));
        }
        list
    }
}
