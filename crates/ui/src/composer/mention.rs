//! `@` mention list (recorded in refs/interactions/s019.json). An empty query lists the `Add`
//! actions and the `Files` section; a query ranks actions, skills and the workspace files that
//! `fuzzyFileSearch/sessionStart` + `sessionUpdate` return (notifications `sessionUpdated`,
//! `sessionCompleted`). Enter or Tab inserts the highlighted row as a [`Mention`] chip
//! (refs/interactions/s020.json); S018 owns the `+` button.
//! `cargo test -p app --test s019` replays the original's list, requests and colors;
//! `--test s020` its keys, chips and the prompt sent.
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

/// A mention chip in `content`: `range` holds [`ICON_SLOT`] and the label; the prompt sends
/// `link` in its place. One atom, like ProseMirror's: the caret never enters it and any edit
/// that touches it deletes it whole.
#[derive(Clone)]
pub struct Mention {
    pub(super) range: Range<usize>,
    pub(super) icon: &'static str,
    link: String,
}

/// Room for the chip's 16 px icon and 3 px gap: em space + three-per-em space (18.7 px at
/// 14 px). ponytail: a glyph placeholder instead of an inline box, which textsys lacks.
pub(super) const ICON_SLOT: &str = "\u{2003}\u{2004}";

struct FileHit {
    name: String,
    path: String,
}

/// One row of [`Composer::sections`]: a ranked item `(selected, index)`, a disabled item,
/// or a note.
enum Row {
    Item(usize, usize),
    Disabled(usize),
    Note(&'static str),
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
    pub(super) fn toggle_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.slash.add_mouse_open) || (self.slash.open && self.slash.mention)
        {
            self.close_slash(cx);
        } else {
            self.end_search(cx);
            self.slash.reset();
            self.slash.add = Some(self.cursor());
            self.slash.open = true;
            self.refresh_slash(cx);
        }
        self.focus_handle.focus(window, cx);
    }

    /// Workspace roots the `Files` section searches; none hides the section (Home).
    pub fn set_search_roots(&mut self, roots: Vec<String>) {
        self.slash.search.roots = roots;
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
        // Stable sort keeps the chain's index order among equal keys.
        ranked.sort_by_key(|&(_, kind, degree, title)| {
            (kind, std::cmp::Reverse(degree), title.len(), title)
        });
        // `oYc` caps the ranked rows at 8.
        self.slash.matches = ranked.iter().take(8).map(|r| r.0).collect();
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
                        path: path.into(),
                    })
                })
                .collect();
        }
        if self.slash.open && self.slash.mention {
            let selected = self.slash.selected;
            self.refresh_slash(cx);
            self.slash.selected = selected.min(self.slash.matches.len().saturating_sub(1));
        }
    }

    pub(super) fn mention_label(&self, index: usize) -> (&str, &str, &str) {
        if let Some(action) = ACTIONS.get(index) {
            (action.title, action.description, action.icon)
        } else if let Some(skill) = self.slash.skills.get(index - ACTIONS.len()) {
            (&skill.title, &skill.description, "sidebar-cmd-create-page")
        } else {
            let file = &self.slash.search.files[index - ACTIONS.len() - self.slash.skills.len()];
            // The directory part shows after the name (`src`).
            let dir = file.path.rsplit_once('/').map_or("", |(dir, _)| dir);
            (&file.name, dir, "diff-jump-to-file")
        }
    }

    /// Enter, Tab or a click on the highlighted row. `Files and folders` opens the picker; a
    /// skill or a file becomes a chip followed by a space, the list closes. ponytail: the
    /// other actions (goal, sketch, project, plan mode) belong to their own rows.
    pub(super) fn activate_mention(&mut self, index: usize, cx: &mut Context<Self>) {
        let files = ACTIONS.len() + self.slash.skills.len();
        let (label, link, icon) = match ACTIONS.get(index) {
            Some(action) if action.id == "files-and-folders" => {
                let range = self.splice(self.slash.range.clone(), "");
                self.selected_range = range.start..range.start;
                self.marked_range = None;
                self.slash.open = false;
                self.end_search(cx);
                self.add_files(cx);
                cx.notify();
                return;
            }
            Some(_) => return,
            None if index < files => {
                let skill = &self.slash.skills[index - ACTIONS.len()];
                let link = format!("[${}]({})", skill.name, skill.path);
                (skill.title.clone(), link, "sidebar-cmd-create-page")
            }
            None => {
                let file = &self.slash.search.files[index - files];
                let link = format!("[{}]({})", file.name, file.path);
                let icon = if file.name.ends_with(".py") {
                    "thread-lang-python"
                } else {
                    "diff-jump-to-file"
                };
                (file.name.clone(), link, icon)
            }
        };
        self.remember(&self.slash.range.clone());
        let range = self.splice(self.slash.range.clone(), &format!("{ICON_SLOT}{label} "));
        let end = range.start + ICON_SLOT.len() + label.len();
        let at = self
            .mentions
            .partition_point(|m| m.range.start < range.start);
        self.mentions.insert(
            at,
            Mention {
                range: range.start..end,
                icon,
                link,
            },
        );
        self.selected_range = end + 1..end + 1;
        self.marked_range = None;
        self.slash.open = false;
        self.slash.dismissed = None;
        self.end_search(cx);
        cx.notify();
    }

    /// Replaces `range` of `content` with `text`: a chip the range touches goes whole, the
    /// chips after it shift. Returns the range replaced.
    pub(super) fn splice(&mut self, mut range: Range<usize>, text: &str) -> Range<usize> {
        self.mentions.retain(|m| {
            let hit = m.range.start < range.end && m.range.end > range.start;
            if hit {
                range.start = range.start.min(m.range.start);
                range.end = range.end.max(m.range.end);
            }
            !hit
        });
        if self.slash.add.is_some_and(|start| range.start < start) {
            self.slash.open = false;
            self.slash.add = None;
        }
        self.content.replace_range(range.clone(), text);
        let delta = text.len() as isize - range.len() as isize;
        for m in &mut self.mentions {
            if m.range.start >= range.end {
                let shift = |at: usize| (at as isize + delta) as usize;
                m.range = shift(m.range.start)..shift(m.range.end);
            }
        }
        range
    }

    /// `offset` moved out of a chip, to its nearer edge (`forward`: the far edge when inside).
    pub(super) fn snap(&self, offset: usize, forward: Option<bool>) -> usize {
        let Some(m) = self
            .mentions
            .iter()
            .find(|m| m.range.start < offset && offset < m.range.end)
        else {
            return offset;
        };
        match forward {
            Some(true) => m.range.end,
            Some(false) => m.range.start,
            None if offset - m.range.start < m.range.end - offset => m.range.start,
            None => m.range.end,
        }
    }

    /// The prompt the composer sends: `content` with each chip replaced by its markdown link.
    pub fn prompt(&self) -> String {
        let mut out = String::with_capacity(self.content.len());
        let mut at = 0;
        for m in &self.mentions {
            out.push_str(&self.content[at..m.range.start]);
            out.push_str(&m.link);
            at = m.range.end;
        }
        out.push_str(&self.content[at..]);
        out
    }

    /// The chips in order: `(label, icon)`, for the tests.
    pub fn chips(&self) -> Vec<(String, &'static str)> {
        self.mentions
            .iter()
            .map(|m| {
                let label = &self.content[m.range.start + ICON_SLOT.len()..m.range.end];
                (label.to_owned(), m.icon)
            })
            .collect()
    }

    /// The list's sections: `Add` and `Files` for an empty query, else the ranked rows or
    /// one note (`Searching…`, `No results`).
    fn sections(&self) -> Vec<(Option<&'static str>, Vec<Row>)> {
        let start = self.slash.range.start + usize::from(self.slash.add.is_none());
        if self.content[start..self.slash.range.end].is_empty() {
            let add = ACTIONS
                .iter()
                .enumerate()
                .filter(|(_, a)| !(a.home_only && self.docked))
                .map(
                    |(i, _)| match self.slash.matches.iter().position(|&m| m == i) {
                        Some(selected) => Row::Item(selected, i),
                        None => Row::Disabled(i),
                    },
                )
                .collect();
            let mut sections = vec![(Some("Add"), add)];
            if !self.slash.search.roots.is_empty() {
                sections.push((Some("Files"), vec![Row::Note("Type to search for files")]));
            }
            return sections;
        }
        let rows = if self.slash.matches.is_empty() {
            vec![Row::Note(if self.slash.search.searching {
                "Searching…"
            } else {
                "No results"
            })]
        } else {
            self.slash
                .matches
                .iter()
                .enumerate()
                .map(|(selected, &i)| Row::Item(selected, i))
                .collect()
        };
        vec![(None, rows)]
    }

    /// Sections `(heading, rows (text, highlighted))` of the open `@` list, for the tests.
    pub fn mention_state(&self) -> Option<Vec<MentionSection>> {
        if !self.slash.open || !self.slash.mention {
            return None;
        }
        let text = |index: usize| {
            let (title, description, _) = self.mention_label(index);
            if description.is_empty() {
                title.to_owned()
            } else {
                format!("{title} | {description}")
            }
        };
        Some(
            self.sections()
                .into_iter()
                .map(|(heading, rows)| {
                    let rows = rows
                        .into_iter()
                        .map(|row| match row {
                            Row::Note(note) => (note.to_owned(), false),
                            Row::Item(selected, i) => (text(i), selected == self.slash.selected),
                            Row::Disabled(i) => (text(i), false),
                        })
                        .collect();
                    (heading, rows)
                })
                .collect(),
        )
    }

    /// The list's content, 13/18.57 text: headings at 50%, notes at 50% in a row's height,
    /// disabled rows (`Attach appshot`) at 50% over 50%.
    pub(super) fn mention_list(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        let dim = |s: SharedString| text(s, 13., 18.5714, 400., t.text.opacity(0.5));
        let mut list = div().flex().flex_col();
        for (n, (heading, rows)) in self.sections().into_iter().enumerate() {
            if let Some(heading) = heading {
                list = list.child(
                    dim(heading.into())
                        .px(px(8.))
                        .pt(px(if n == 0 { 4. } else { 8. }))
                        .pb(px(4.)),
                );
            }
            for row in rows {
                let plain = div().h(px(28.5625)).px(px(8.)).flex().items_center();
                list = list.child(match row {
                    Row::Item(selected, i) => self
                        .suggestion_row(t, selected, i, self.mention_label(i), cx)
                        .into_any_element(),
                    Row::Note(note) => plain.child(dim(note.into())).into_any_element(),
                    Row::Disabled(i) => {
                        let (title, _, icon_name) = self.mention_label(i);
                        plain
                            .gap(px(8.))
                            .opacity(0.5)
                            .child(icon(icon_name, 16., t.text.opacity(0.5)))
                            .child(dim(title.to_owned().into()))
                            .into_any_element()
                    }
                });
            }
        }
        list
    }
}
