//! Side panel of a thread route: the split layout and titlebar tab strip that refs/diff and
//! refs/terminal share, and the Changes panel with the unified diff of the last turn's file
//! changes. All numbers are the spec's CSS px.

use crate::{
    MONO, SANS, Shell, composer::shadow, drag_layer, fixture, icon::icon, layout_color, over,
    screens::thread::view::ThreadView, text,
};
use bridge::{
    Event,
    types::{ThreadItem, ThreadReadResponse, ThreadStartResponse, Turn},
};
use gpui::{
    Action, AnyView, App, AppContext as _, ClickEvent, ClipboardItem, Context, Corners,
    CursorStyle, Div, Entity, FocusHandle, Font, FontFeatures, FontWeight, Global, HighlightStyle,
    Hsla, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Render, Rgba, ScrollHandle, SharedString, Stateful,
    StatefulInteractiveElement, Styled, StyledText, TextRun, Window, black, combine_highlights,
    deferred, div, font, prelude::FluentBuilder as _, px, rgb,
};
use shapes::Decoration;
use std::{cell::Cell, ops::Range, rc::Rc, sync::Arc};
use theme::{ActiveTheme as _, Appearance, Theme};

#[path = "diff/menu.rs"]
mod menu;

#[path = "diff/controls.rs"]
mod controls;
pub use controls::DiffLayout;

/// Panel width the capture had stored (`app-shell:right-panel-width:v3`) before clamping.
const STORED_WIDTH: f32 = 664.17;
/// Room the chat keeps beside the panel (`unified` layout), also the panel's minimum.
const MIN_CHAT: f32 = 320.;
/// `--diffs-font-size`, `--diffs-line-height`.
const CODE_SIZE: f32 = 12.;
const CODE_LINE: f32 = 21.6;
/// Menlo advance at 12 px (0.602em); `--diffs-min-number-column-width` is 4ch.
const CH: f32 = 7.224_609_4;

/// The panel width the user last dragged the divider to (DT-056); `STORED_WIDTH` until then.
struct PanelWidth(Pixels);

impl Global for PanelWidth {}

/// Panel width for the current window, as the original clamps its stored width: at least
/// `MIN_CHAT`, and the chat keeps `MIN_CHAT` beside it.
/// ponytail: assumes the sidebar is open; a collapsed sidebar only leaves the chat wider.
pub fn panel_width(window: &Window, cx: &App) -> Pixels {
    let stored = cx
        .try_global::<PanelWidth>()
        .map_or(px(STORED_WIDTH), |w| w.0);
    stored.clamp(px(MIN_CHAT), panel_max(window))
}

/// The widest panel: the main panel's inner width (window minus sidebar, the main panel's
/// 0.5 px hairline and 4 px inset) less the chat's `MIN_CHAT`.
fn panel_max(window: &Window) -> Pixels {
    let main = window.viewport_size().width - Theme::SIDEBAR_WIDTH - px(4.5);
    (main - px(MIN_CHAT)).max(px(MIN_CHAT))
}

/// The panel at its clamped width (the chat at `MIN_CHAT`): with `Shell::main_layer`,
/// Chromium hides the surface's own fill under the content's at the corners and clips that
/// content twice (coverage m m) instead of stacking both (each m). cc does that when the
/// synthetic effect of the main surface's rounded clip owns a render surface: it applies the
/// clip's rrect to that surface and again to the quads of its descendant effects that have
/// none (`GetMaskFilterInfoPair` in cc/trees/draw_property_utils.cc).
/// ponytail: the trigger is measured, not derived. Blink gives that effect its surface at
/// 1280 and 1300 px wide, not at 1400 and 1440. Panel geometry is not the cause (1440 with a
/// clamped-width layout still stacks), and neither are the effort labels or the word-wrap
/// button. Hiding the chat's scroller at 1280 clips once. Blink forces that surface
/// (`kRoundedCorner`) when a nested shader-based rounded clip sits under the surface's clip
/// with no non-conditional render surface between (`SynthesizeCcEffectsForClipsIfNeeded` in
/// property_tree_manager.cc), so some rounded clip in the chat is composited only at 1280.
pub fn panel_clamped(window: &Window, cx: &App) -> bool {
    panel_width(window, cx) < px(STORED_WIDTH)
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Context,
    Added,
    Removed,
}

struct Line {
    kind: Kind,
    /// New-file number; old-file number for removed lines.
    number: u32,
    text: SharedString,
    matches: Vec<Range<usize>>,
    old_number: u32,
    match_start: usize,
    word_change: Option<Range<usize>>,
}

struct Hunk {
    /// `N unmodified lines` bar for the old-file lines the diff skips before this hunk.
    separator: Option<SharedString>,
    lines: Vec<Line>,
}

pub struct FileDiff {
    /// Workspace-relative directory part (tertiary) and file name of the header path.
    pub dir: SharedString,
    pub name: SharedString,
    pub added: usize,
    pub removed: usize,
    hunks: Vec<Hunk>,
    pub open: bool,
    problem: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiffSource {
    LastTurn,
    Uncommitted,
    Unstaged,
    Staged,
    Branch,
    Commits,
}

impl DiffSource {
    fn label(self) -> &'static str {
        match self {
            Self::LastTurn => "Last Turn",
            Self::Uncommitted => "Uncommitted",
            Self::Unstaged => "Unstaged",
            Self::Staged => "Staged",
            Self::Branch => "Branch",
            Self::Commits => "Committed",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::LastTurn => "last-turn",
            Self::Uncommitted => "uncommitted",
            Self::Unstaged => "unstaged",
            Self::Staged => "staged",
            Self::Branch => "branch",
            Self::Commits => "commits",
        }
    }

    fn mutable(self) -> bool {
        matches!(self, Self::Uncommitted | Self::Unstaged | Self::Staged)
    }
}

#[derive(Clone)]
pub enum DiffRequest {
    Refresh,
    Stage(String),
    Unstage(String),
    Revert(RevertChanges),
    Changes {
        action: &'static str,
        files: Vec<RevertChanges>,
    },
    CreateRepository,
    /// Open this absolute path in the configured editor.
    Open(String),
}

#[derive(Clone, serde::Serialize)]
pub struct RevertChanges {
    pub path: String,
    pub source: &'static str,
    pub diff: String,
    pub hunk: Option<usize>,
}

#[derive(serde::Deserialize)]
pub struct RepositoryChanges {
    pub root: String,
    pub unstaged: Vec<RepositoryFile>,
    pub staged: Vec<RepositoryFile>,
    #[serde(default)]
    pub review: Vec<RepositoryFile>,
    #[serde(default)]
    pub branches: Vec<String>,
    #[serde(default)]
    pub commits: Vec<RepositoryCommit>,
    #[serde(default)]
    pub base_branch: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub not_repository: bool,
}

#[derive(serde::Deserialize)]
pub struct RepositoryCommit {
    pub id: String,
    pub label: String,
}

#[derive(serde::Deserialize)]
pub struct RepositoryFile {
    pub path: String,
    pub diff: String,
}

impl gpui::EventEmitter<DiffRequest> for DiffPanel {}

pub struct DiffPanel {
    pub files: Vec<FileDiff>,
    focus: Option<FocusHandle>,
    query: Option<String>,
    matches: usize,
    active_match: usize,
    selected_query: bool,
    aggregated_turn: Option<String>,
    selection: Option<((usize, usize), (usize, usize))>,
    selecting: bool,
    menu: Option<menu::Menu>,
    workspace: String,
    wrap: bool,
    source: DiffSource,
    turn_files: Vec<FileDiff>,
    repository: Option<RepositoryChanges>,
    pending: bool,
    error: Option<String>,
    layout: DiffLayout,
    show_tree: bool,
    active_file: usize,
    filter: controls::TextField,
    filter_focus: Option<FocusHandle>,
    scroll: ScrollHandle,
    standalone: bool,
    base_branch: Option<String>,
    commit: Option<String>,
}

impl DiffPanel {
    /// The subtract session as the reference shows it.
    pub fn subtract() -> Self {
        let start: ThreadStartResponse =
            fixture::response("sessions/subtract.jsonl", "thread/start", 0);
        let read: ThreadReadResponse = fixture::response("readback.jsonl", "thread/read", 1);
        Self::last_turn(&start.cwd, &read.thread.cwd, &read.thread.turns)
    }

    /// "Last Turn" source: every file change of the last of `turns`, a thread in `cwd`. Paths
    /// show relative to `workspace_root`; the server reports the thread cwd canonicalized
    /// (`/private/tmp/...`), which is why the reference shows `../../../private/tmp/...`.
    pub fn last_turn(workspace_root: &str, cwd: &str, turns: &[Turn]) -> Self {
        let mut files: Vec<FileDiff> = Vec::new();
        let changes = turns
            .last()
            .into_iter()
            .flat_map(|turn| &turn.items)
            .filter_map(|item| match item {
                ThreadItem::FileChange { changes, .. } => Some(changes),
                _ => None,
            })
            .flatten();
        for change in changes {
            let path = match change.path.strip_prefix(workspace_root) {
                Some(rel) => format!("{cwd}/{}", rel.trim_start_matches('/')),
                None => change.path.clone(),
            };
            let shown = relative(workspace_root, &path);
            let split = shown.rfind('/').map_or(0, |i| i + 1);
            let (dir, name) = shown.split_at(split);
            let raw = match change.kind["type"].as_str() {
                Some("add") => Some(Kind::Added),
                Some("delete") => Some(Kind::Removed),
                _ => None,
            };
            let parsed = parse(&change.diff, raw);
            let (hunks, problem) = match parsed {
                Ok(hunks) => (hunks, None),
                Err(problem) => (Vec::new(), Some(problem)),
            };
            let count = |k| {
                hunks
                    .iter()
                    .flat_map(|h| &h.lines)
                    .filter(|l| l.kind == k)
                    .count()
            };
            let (added, removed) = (count(Kind::Added), count(Kind::Removed));
            match files.iter_mut().find(|f| f.dir == dir && f.name == name) {
                Some(f) => {
                    f.added += added;
                    f.removed += removed;
                    f.hunks.extend(hunks);
                }
                None => files.push(FileDiff {
                    dir: dir.to_string().into(),
                    name: name.to_string().into(),
                    added,
                    removed,
                    hunks,
                    open: true,
                    problem,
                }),
            }
        }
        Self {
            files,
            focus: None,
            query: None,
            matches: 0,
            active_match: 0,
            selected_query: false,
            aggregated_turn: None,
            selection: None,
            selecting: false,
            menu: None,
            workspace: workspace_root.to_owned(),
            wrap: false,
            source: DiffSource::LastTurn,
            turn_files: Vec::new(),
            repository: None,
            pending: false,
            error: None,
            layout: DiffLayout::Auto,
            show_tree: false,
            active_file: 0,
            filter: Default::default(),
            filter_focus: None,
            scroll: ScrollHandle::new(),
            standalone: false,
            base_branch: None,
            commit: None,
        }
    }

    pub fn pending(&self) -> bool {
        self.pending
    }

    pub fn repository_error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn review_params(&self) -> serde_json::Value {
        serde_json::json!({"reviewSource": self.source.key(), "baseBranch": self.base_branch, "commit": self.commit})
    }

    fn actionable_files(&self) -> &[RepositoryFile] {
        self.repository.as_ref().map_or(&[], |repository| {
            if self.source == DiffSource::Staged {
                &repository.staged
            } else {
                &repository.unstaged
            }
        })
    }

    fn request_changes(
        &self,
        action: &'static str,
        file: Option<(usize, usize)>,
        cx: &mut Context<Self>,
    ) {
        if self.pending
            || self.error.is_some()
            || !self.source.mutable()
            || (action == "revert" && self.source != DiffSource::Unstaged)
        {
            return;
        }
        let files = self
            .actionable_files()
            .iter()
            .enumerate()
            .filter(|(i, _)| file.is_none_or(|(selected, _)| selected == *i))
            .map(|(_, f)| RevertChanges {
                path: f.path.clone(),
                source: if self.source == DiffSource::Staged {
                    "staged"
                } else {
                    "unstaged"
                },
                diff: f.diff.clone(),
                hunk: file.map(|(_, h)| h),
            })
            .collect::<Vec<_>>();
        if !files.is_empty() {
            cx.emit(DiffRequest::Changes { action, files });
        }
    }
    fn operation_button(
        &self,
        label: &'static str,
        file: Option<(usize, usize)>,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let action = if label.starts_with("Unstage") {
            "unstage"
        } else if label.starts_with("Revert") {
            "revert"
        } else {
            "stage"
        };
        let id = match file {
            Some((i, h)) => format!("{action}-hunk-{i}-{h}"),
            None => format!("{action}-all"),
        };
        let enabled = !self.pending && self.error.is_none();
        div()
            .id(SharedString::from(id))
            .role(gpui::Role::Button)
            .aria_label(file.map_or_else(
                || label.to_owned(),
                |(_, h)| format!("{label} hunk {}", h + 1),
            ))
            .aria_disabled(!enabled)
            .focusable()
            .tab_index(0)
            .focus_visible(|s| s.border_2().border_color(rgb(0x3a83f7)))
            .cursor_pointer()
            .px(px(8.))
            .h(px(28.))
            .rounded(px(14.))
            .bg(cx.theme().list_hover)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.request_changes(action, file, cx);
                    cx.stop_propagation();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.request_changes(action, file, cx);
                cx.stop_propagation();
            }))
            .child(text(label, 12., 20., 430., cx.theme().text))
    }

    pub fn begin_repository_request(&mut self, cx: &mut Context<Self>) {
        self.pending = true;
        self.error = None;
        cx.notify();
    }

    pub fn set_repository(
        &mut self,
        result: Result<RepositoryChanges, String>,
        cx: &mut Context<Self>,
    ) {
        self.pending = false;
        match result {
            Ok(repository) => {
                self.base_branch = repository.base_branch.clone().or(self.base_branch.take());
                self.commit = repository.commit.clone().or(self.commit.take());
                self.repository = Some(repository);
                self.error = None;
            }
            Err(error) => {
                self.error = Some(error);
            }
        }
        if self.source != DiffSource::LastTurn {
            self.show_repository();
        }
        cx.notify();
    }

    fn request_revert(&self, file: usize, hunk: Option<usize>, cx: &mut Context<Self>) {
        if self.pending || self.error.is_some() || !self.source.mutable() {
            return;
        }
        let files = self.actionable_files();
        let Some(shown) = self.files.get(file) else {
            return;
        };
        let path = format!("{}{}", shown.dir, shown.name);
        let Some(file) = files.iter().find(|f| f.path == path) else {
            return;
        };
        cx.emit(DiffRequest::Revert(RevertChanges {
            path: file.path.clone(),
            source: if self.source == DiffSource::Staged {
                "staged"
            } else {
                "unstaged"
            },
            diff: file.diff.clone(),
            hunk,
        }));
    }

    fn select_source(&mut self, source: DiffSource, cx: &mut Context<Self>) {
        let previous = self.source;
        if previous == DiffSource::LastTurn && source != previous {
            self.turn_files = std::mem::take(&mut self.files);
        }
        self.source = source;
        if source == DiffSource::LastTurn && source != previous {
            let files = std::mem::take(&mut self.turn_files);
            self.replace_files(files);
        }
        if source != DiffSource::LastTurn {
            self.show_repository();
            if !self.pending {
                cx.emit(DiffRequest::Refresh);
            }
        }
        cx.notify();
    }

    fn show_repository(&mut self) {
        let files = self
            .repository
            .as_ref()
            .map(|repository| {
                match self.source {
                    DiffSource::Staged => &repository.staged,
                    DiffSource::Unstaged => &repository.unstaged,
                    _ => &repository.review,
                }
                .iter()
                .map(|file| FileDiff::patch(&file.path, &file.diff))
                .collect()
            })
            .unwrap_or_default();
        self.replace_files(files);
    }

    pub fn search(&self) -> Option<(&str, usize)> {
        self.query.as_deref().map(|query| (query, self.matches))
    }

    pub fn open_find(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self
            .focus
            .as_ref()
            .is_some_and(|focus| focus.is_focused(window))
        {
            return false;
        }
        self.query.get_or_insert_default();
        self.selected_query = true;
        cx.notify();
        true
    }

    pub fn apply(
        &mut self,
        event: &Event,
        workspace: &str,
        turns: &[Turn],
        cx: &mut Context<Self>,
    ) {
        let Event::Notification { method, params } = event else {
            return;
        };
        let Some(turn) = turns.last() else { return };
        if params["turnId"].as_str() != Some(&turn.id) {
            return;
        }
        let files = match method.as_str() {
            "turn/diff/updated" => {
                let Some(diff) = params["diff"].as_str() else {
                    return;
                };
                self.aggregated_turn = Some(turn.id.clone());
                controls::files(diff)
            }
            "item/started" | "item/completed"
                if params["item"]["type"] == "fileChange"
                    && self.aggregated_turn.as_deref() != Some(&turn.id) =>
            {
                Self::last_turn(workspace, workspace, turns).files
            }
            _ => return,
        };
        if self.source == DiffSource::LastTurn {
            self.replace_files(files);
        } else {
            self.turn_files = files;
        }
        cx.notify();
    }

    fn replace_files(&mut self, mut files: Vec<FileDiff>) {
        self.active_file = self
            .files
            .get(self.active_file)
            .and_then(|old| {
                files
                    .iter()
                    .position(|file| file.dir == old.dir && file.name == old.name)
            })
            .unwrap_or(0);
        for file in &mut files {
            if let Some(old) = self
                .files
                .iter()
                .find(|old| old.dir == file.dir && old.name == file.name)
            {
                file.open = old.open;
            }
        }
        self.files = files;
        self.selection = None;
        self.selecting = false;
        if self
            .menu
            .as_ref()
            .is_some_and(|m| matches!(m.kind, menu::Kind::Header | menu::Kind::Body))
        {
            self.menu = None;
        }
        self.search_changed();
    }

    fn search_changed(&mut self) {
        let query = self.query.as_deref().unwrap_or("");
        self.matches = 0;
        for file in &mut self.files {
            let before = self.matches;
            for line in file.hunks.iter_mut().flat_map(|h| &mut h.lines) {
                line.match_start = self.matches;
                line.matches.clear();
                if !query.is_empty() {
                    let mut end = 0;
                    for (start, _) in line.text.char_indices() {
                        if start >= end
                            && line
                                .text
                                .get(start..start + query.len())
                                .is_some_and(|s| s.eq_ignore_ascii_case(query))
                        {
                            end = start + query.len();
                            line.matches.push(start..end);
                        }
                    }
                }
                self.matches += line.matches.len();
            }
            if self.matches > before {
                file.open = true;
            }
        }
        self.active_match = self.active_match.min(self.matches.saturating_sub(1));
    }

    pub fn selected_text(&self) -> String {
        let Some((anchor, head)) = self.selection else {
            return String::new();
        };
        let (start, end) = (anchor.min(head), anchor.max(head));
        let mut text = String::new();
        for (i, line) in self
            .files
            .iter()
            .flat_map(|f| &f.hunks)
            .flat_map(|h| &h.lines)
            .enumerate()
        {
            if i < start.0 || i > end.0 {
                continue;
            }
            if i > start.0 {
                text.push('\n');
            }
            let a = if i == start.0 { start.1 } else { 0 };
            let b = if i == end.0 { end.1 } else { line.text.len() };
            text.push_str(&line.text[a..b]);
        }
        text
    }

    fn copy_selection(&self, cx: &App) {
        let text = self.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() {
            self.menu_key(event, window, cx);
            return;
        }
        let key = &event.keystroke;
        if self
            .filter_focus
            .as_ref()
            .is_some_and(|focus| focus.is_focused(window))
        {
            if key.key == "escape" {
                self.filter.text.clear();
                if let Some(focus) = &self.focus {
                    window.focus(focus, cx);
                }
            } else if !self.filter.key(event) {
                return;
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if key.modifiers.platform && key.key == "f" {
            self.open_find(window, cx);
            cx.stop_propagation();
            return;
        } else if let Some(query) = &mut self.query {
            match key.key.as_str() {
                "escape" => self.query = None,
                "enter" if self.matches > 0 => {
                    self.active_match = if key.modifiers.shift {
                        (self.active_match + self.matches - 1) % self.matches
                    } else {
                        (self.active_match + 1) % self.matches
                    };
                }
                "backspace" => {
                    if self.selected_query {
                        query.clear();
                    } else {
                        query.pop();
                    }
                    self.selected_query = false;
                }
                _ if !key.modifiers.platform && !key.modifiers.control => {
                    let Some(text) = &key.key_char else { return };
                    if self.selected_query {
                        query.clear();
                    }
                    query.push_str(text);
                    self.selected_query = false;
                    self.active_match = 0;
                }
                _ => return,
            }
        } else {
            return;
        }
        self.search_changed();
        cx.stop_propagation();
        cx.notify();
    }

    fn find_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let query = self.query.as_deref().unwrap_or("");
        let count = format!(
            "{} / {} results",
            if self.matches == 0 {
                0
            } else {
                self.active_match + 1
            },
            self.matches
        );
        let button = |id, name, label| {
            round_icon(name, 28., t)
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .cursor_pointer()
        };
        deferred(
            div()
                .absolute()
                .top(px(-36.))
                .right(px(12.))
                .w(px(340.))
                .h(px(82.))
                .rounded(px(20.))
                .border(px(0.5))
                .border_color(t.border)
                .bg(t.surface_secondary)
                .flex()
                .flex_col()
                .font_family(SANS)
                .child(
                    div()
                        .h(px(44.))
                        .flex()
                        .items_center()
                        .pl(px(16.))
                        .pr(px(12.))
                        .gap(px(8.))
                        .child(icon("search", 16., t.text))
                        .child(
                            div()
                                .id("diff-find-input")
                                .flex_1()
                                .cursor_text()
                                .role(gpui::Role::TextInput)
                                .aria_label("Find in diff")
                                .aria_placeholder("Search diff…")
                                .aria_value(query.to_owned())
                                .aria_active_descendant()
                                .child(text(
                                    if query.is_empty() {
                                        "Search diff…"
                                    } else {
                                        query
                                    },
                                    14.,
                                    24.,
                                    400.,
                                    if query.is_empty() {
                                        t.text_tertiary
                                    } else {
                                        t.text
                                    },
                                )),
                        )
                        .child(button("diff-find-close", "close", "Close find").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.query = None;
                                this.search_changed();
                                cx.notify();
                            }),
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .px(px(8.))
                        .border_t_1()
                        .border_color(t.border)
                        .child(
                            button("diff-find-previous", "composer-arrow-up", "Previous result")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if this.matches > 0 {
                                        this.active_match =
                                            (this.active_match + this.matches - 1) % this.matches;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            button("diff-find-next", "diff-chevron-down", "Next result").on_click(
                                cx.listener(|this, _, _, cx| {
                                    if this.matches > 0 {
                                        this.active_match = (this.active_match + 1) % this.matches;
                                    }
                                    cx.notify();
                                }),
                            ),
                        )
                        .child(div().flex_1())
                        .child(text(count, 12., 20., 400., t.text_tertiary).mr(px(8.))),
                ),
        )
        .with_priority(3)
    }

    fn header(&self, t: &Theme, wide: bool, clamped: bool, cx: &Context<Self>) -> Div {
        let added = self.files.iter().map(|f| f.added).sum();
        let removed = self.files.iter().map(|f| f.removed).sum();
        let tool = |name| round_icon(name, 28., t);
        let source_open = self
            .menu
            .as_ref()
            .is_some_and(|menu| matches!(menu.kind, menu::Kind::Source | menu::Kind::Commits));
        // A button with its own `backdrop-filter: blur(24px)`, nested in the capsule's.
        let lens = || {
            shapes::backdrop_blur(
                px(24.),
                shapes::Fill::Color,
                false,
                Decoration {
                    radius: Corners::all(px(14.)),
                    ..Default::default()
                },
            )
        };
        // A tool with a lens is its own composited layer: the icon blends onto transparent.
        let lensed = |name| {
            div()
                .relative()
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .layer()
                .child(lens())
                .child(icon(name, 16., t.text_tertiary))
        };
        div()
            .h(px(48.))
            .flex_none()
            .p(px(8.))
            .flex()
            .justify_between()
            .gap(px(8.))
            .child(
                capsule(t, clamped)
                    .child(
                        div()
                            .id("diff-source")
                            .role(gpui::Role::Button)
                            .aria_label("Changes source")
                            .aria_expanded(source_open)
                            .focusable()
                            .cursor_pointer()
                            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                                this.open_menu(0, menu::Kind::Source, event.position(), window, cx);
                            }))
                            .relative()
                            .h(px(28.))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .pl(px(12.))
                            .pr(px(6.))
                            .child(lens())
                            .child(text(self.source.label(), 13., 20., 430., t.text))
                            .child(icon("diff-chevron-down", 12., t.text)),
                    )
                    .child(stats(added, removed, 14., t).mr(px(4.))),
            )
            .child(
                capsule(t, clamped)
                    .child(
                        tool("diff-more")
                            .id("diff-options")
                            .role(gpui::Role::Button)
                            .aria_label("Changes options")
                            .focusable()
                            .cursor_pointer()
                            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                                this.open_menu(
                                    0,
                                    menu::Kind::Options,
                                    event.position(),
                                    window,
                                    cx,
                                );
                            }))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.open_menu(
                                        0,
                                        menu::Kind::Options,
                                        gpui::point(
                                            window.viewport_size().width - px(228.),
                                            px(92.),
                                        ),
                                        window,
                                        cx,
                                    );
                                    cx.stop_propagation();
                                }
                            })),
                    )
                    .child(tool("diff-jump-to-file"))
                    // `@container review-header (min-width: 625px)`
                    .when(wide, |d| {
                        d.child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .child(
                                    tool("diff-refresh")
                                        .id("diff-refresh")
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if this.source != DiffSource::LastTurn && !this.pending
                                            {
                                                cx.emit(DiffRequest::Refresh);
                                            }
                                        })),
                                )
                                .child(
                                    lensed("diff-word-wrap")
                                        .id("diff-wrap")
                                        .role(gpui::Role::Button)
                                        .aria_label("Word wrap")
                                        .aria_toggled(if self.wrap {
                                            gpui::Toggled::True
                                        } else {
                                            gpui::Toggled::False
                                        })
                                        .focusable()
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.wrap = !this.wrap;
                                            cx.notify();
                                        }))
                                        .on_key_down(cx.listener(
                                            |this, event: &KeyDownEvent, _, cx| {
                                                if matches!(
                                                    event.keystroke.key.as_str(),
                                                    "enter" | "space"
                                                ) {
                                                    this.wrap = !this.wrap;
                                                    cx.stop_propagation();
                                                    cx.notify();
                                                }
                                            },
                                        )),
                                )
                                .child(tool("diff-load-full-files"))
                                .child(self.layout_button(t, cx)),
                        )
                    })
                    .child(
                        lensed("diff-show-files")
                            .id("diff-show-files")
                            .role(gpui::Role::Button)
                            .aria_label(if self.show_tree {
                                "Hide files"
                            } else {
                                "Show files"
                            })
                            .aria_toggled(if self.show_tree {
                                gpui::Toggled::True
                            } else {
                                gpui::Toggled::False
                            })
                            .focusable()
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_tree = !this.show_tree;
                                cx.notify();
                            }))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.show_tree = !this.show_tree;
                                    cx.stop_propagation();
                                    cx.notify();
                                }
                            })),
                    ),
            )
    }
}

impl Render for DiffPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let width = if self.standalone {
            window.viewport_size().width
        } else {
            panel_width(window, cx) - px(1.)
        };
        let wide = width >= px(625.);
        let tree = self.show_tree.then(|| self.file_tree(width, window, cx));
        let code_width = width
            - if self.standalone { px(40.) } else { px(0.) }
            - if self.show_tree {
                px(250.).min(width * 0.6)
            } else {
                px(0.)
            };
        let not_repository = self.source != DiffSource::LastTurn
            && self.repository.as_ref().is_some_and(|r| r.not_repository);
        let boundary = if not_repository {
            None
        } else {
            self.boundary(&t, cx)
        };
        let status = self.repository_status(&t, cx);
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        let line_index = Cell::new(0);
        let files = self.files.iter().enumerate().map(|(i, file)| {
            div()
                .flex()
                .flex_col()
                .flex_none()
                .when(self.standalone, |d| {
                    d.relative()
                        .overflow_hidden()
                        .child(shapes::decoration(Decoration {
                            fill: Some(t.surface),
                            ..Default::default()
                        }))
                })
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                        this.open_menu(i, menu::Kind::Body, event.position, window, cx);
                    }),
                )
                .child(file_header(i, file, self, &t, cx).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.files[i].open ^= true;
                        this.active_file = i;
                        this.selection = None;
                        cx.notify();
                    },
                )))
                .map(|d| {
                    if file.open && file.problem.is_some() {
                        d.child(
                            text(
                                file.problem.unwrap_or_default(),
                                13.,
                                20.,
                                400.,
                                t.text_tertiary,
                            )
                            .p(px(12.)),
                        )
                    } else if file.open {
                        d.children(file.hunks.iter().enumerate().map(|(index, h)| {
                            let controls =
                                matches!(self.source, DiffSource::Unstaged | DiffSource::Staged)
                                    && h.lines.iter().any(|l| l.kind != Kind::Context);
                            div()
                                .when(controls, |d| {
                                    d.child(
                                        div()
                                            .flex()
                                            .justify_end()
                                            .gap(px(8.))
                                            .px(px(12.))
                                            .py(px(4.))
                                            .child(self.operation_button(
                                                if self.source == DiffSource::Staged {
                                                    "Unstage"
                                                } else {
                                                    "Stage"
                                                },
                                                Some((i, index)),
                                                cx,
                                            ))
                                            .child(
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "revert-hunk-{i}-{index}"
                                                    )))
                                                    .role(gpui::Role::Button)
                                                    .aria_label(format!(
                                                        "Revert hunk {}",
                                                        index + 1
                                                    ))
                                                    .aria_disabled(
                                                        self.pending || self.error.is_some(),
                                                    )
                                                    .focusable()
                                                    .tab_index(0)
                                                    .focus_visible(|s| {
                                                        s.border_2().border_color(rgb(0x3a83f7))
                                                    })
                                                    .cursor_pointer()
                                                    .px(px(8.))
                                                    .h(px(24.))
                                                    .rounded(px(12.))
                                                    .bg(t.list_hover)
                                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                        cx.stop_propagation()
                                                    })
                                                    .on_key_down(cx.listener(
                                                        move |this, event: &KeyDownEvent, _, cx| {
                                                            if matches!(
                                                                event.keystroke.key.as_str(),
                                                                "enter" | "space"
                                                            ) {
                                                                this.request_revert(
                                                                    i,
                                                                    Some(index),
                                                                    cx,
                                                                );
                                                                cx.stop_propagation();
                                                            }
                                                        },
                                                    ))
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.request_revert(i, Some(index), cx);
                                                        cx.stop_propagation();
                                                    }))
                                                    .child(text("Revert", 12., 20., 430., t.text)),
                                            ),
                                    )
                                })
                                .child(hunk(
                                    h,
                                    &t,
                                    &line_index,
                                    self,
                                    self.layout.split(file, code_width),
                                    cx,
                                ))
                        }))
                    } else {
                        line_index.set(
                            line_index.get()
                                + file.hunks.iter().map(|h| h.lines.len()).sum::<usize>(),
                        );
                        d
                    }
                })
                .map(|d| {
                    if self.standalone {
                        crate::rounded_clip(px(12.5), shapes::Corner::Squircle, d)
                            .into_any_element()
                    } else {
                        d.into_any_element()
                    }
                })
        });
        // No fill: the main panel's rounded fill (a `shapes` decoration the capsules' backdrop
        // blur reads) shows through, so its corners stay round. The body's 1 px top border
        // (translucent) sits in the layer under the capsules, whose shadows blend over it; they
        // do not reach the opaque scroller under it, which Chromium composites in its own layer
        // above them: clip them there.
        div()
            .relative()
            .track_focus(&focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.selection = None;
                    this.selecting = false;
                    window.focus(&focus, cx);
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|this, _: &crate::Copy, window, cx| {
                if this
                    .filter_focus
                    .as_ref()
                    .is_some_and(|focus| focus.is_focused(window))
                {
                    if this.filter.selected && !this.filter.text.is_empty() {
                        cx.write_to_clipboard(ClipboardItem::new_string(this.filter.text.clone()));
                    }
                } else {
                    this.copy_selection(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &crate::Paste, window, cx| {
                if this
                    .filter_focus
                    .as_ref()
                    .is_some_and(|focus| focus.is_focused(window))
                {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        this.filter.paste(&text);
                        cx.notify();
                    }
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &crate::SelectAll, window, cx| {
                if this
                    .filter_focus
                    .as_ref()
                    .is_some_and(|focus| focus.is_focused(window))
                {
                    this.filter.selected = true;
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.selecting = false;
                }),
            )
            .size_full()
            .when(self.standalone, |d| d.bg(t.background_surface_under))
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .flex_none()
                    .overflow_hidden()
                    .when(!self.standalone, |d| d.pb(px(1.)))
                    // Skia blends it like Chromium (a GPUI quad rounds apart).
                    .when(!self.standalone, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left_0()
                                .right_0()
                                .h(px(1.))
                                .child(shapes::decoration(Decoration {
                                    fill: Some(t.border),
                                    ..Default::default()
                                })),
                        )
                    })
                    .child(if self.standalone {
                        self.standalone_header(&t, cx)
                    } else {
                        self.header(&t, wide, panel_clamped(window, cx), cx)
                    }),
            )
            .children(status)
            .when(not_repository, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(12.))
                        .m(px(24.))
                        .child(text("Create a Git repository", 16., 24., 500., t.text))
                        .child(
                            div()
                                .id("create-git-repository")
                                .role(gpui::Role::Button)
                                .aria_label("Create git repository")
                                .aria_disabled(self.pending)
                                .focusable()
                                .tab_index(0)
                                .cursor_pointer()
                                .focus_visible(|s| s.border_2().border_color(rgb(0x3a83f7)))
                                .px(px(12.))
                                .py(px(6.))
                                .rounded(px(16.))
                                .bg(t.list_hover)
                                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                                    if !this.pending
                                        && matches!(event.keystroke.key.as_str(), "enter" | "space")
                                    {
                                        cx.emit(DiffRequest::CreateRepository);
                                        cx.stop_propagation();
                                    }
                                }))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if !this.pending {
                                        cx.emit(DiffRequest::CreateRepository);
                                    }
                                }))
                                .child(text("Create git repository", 13., 20., 430., t.text)),
                        ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(boundary.unwrap_or_else(|| {
                        div()
                            .id("diff-files")
                            .flex_1()
                            .min_w_0()
                            .overflow_y_scroll()
                            .overflow_x_scroll()
                            .track_scroll(&self.scroll)
                            .flex()
                            .flex_col()
                            .pb(px(12.))
                            .when(self.standalone, |d| d.px(px(20.)).gap(px(4.)))
                            .children(files)
                            .into_any_element()
                    }))
                    .children(tree),
            )
            .when(
                self.source.mutable() && !self.actionable_files().is_empty(),
                |d| {
                    d.child(
                        div()
                            .absolute()
                            .bottom(px(20.))
                            .left(px(20.))
                            .right(px(20.))
                            .flex()
                            .justify_center()
                            .gap(px(8.))
                            .when(self.source == DiffSource::Unstaged, |d| {
                                d.child(self.operation_button("Revert all", None, cx))
                            })
                            .child(self.operation_button(
                                if self.source == DiffSource::Staged {
                                    "Unstage all"
                                } else {
                                    "Stage all"
                                },
                                None,
                                cx,
                            )),
                    )
                },
            )
            .when(self.query.is_some(), |d| d.child(self.find_bar(cx)))
            .when(self.menu.is_some(), |d| {
                d.child(self.menu_overlay(window, cx))
            })
    }
}

/// A side panel tab: its icon and label.
pub type Tab = (&'static str, SharedString);

/// Click on a tab of the side panel's strip (`activateOnClick`): index into the strip.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = ui, no_json)]
pub struct ActivateSideTab(pub usize);

/// A tab's `Close {title} tab` button: index into the strip.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = ui, no_json)]
pub struct CloseSideTab(pub usize);

gpui::actions!(ui, [ToggleSidePanelVisibility]);

impl SplitScreen {
    /// Titlebar row over a side panel (`Task tabs`): tabs use slots up to 240 px wide (the
    /// selected one on its surface, the others `text-tertiary`, surface and `×` on hover),
    /// `New tab`, and the `Enter full view` / `Hide tabs` buttons. The caller places it from
    /// the panel's left edge to the window's right edge, 44 px tall.
    fn tab_strip(&self, t: &Theme, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let slot_width = ((f32::from(panel_width(window, cx)) - 121.).round()
            / self.tabs.len().max(1) as f32)
            .clamp(90., 240.);
        const FADE: f32 = 16.;
        let layout = layout_color(t);
        let fill = match t.appearance {
            Appearance::Dark => t.ghost_hover,
            Appearance::Light => t.surface_elevated_secondary,
        };
        let mut tab = |ix: usize, (tab_icon, label): &Tab, cx: &mut Context<Self>| {
            let selected = ix == self.active;
            let hovered = self.hovered == Some(ix);
            let color = if selected { t.text } else { t.text_tertiary };
            let label = label.clone();
            let run = TextRun {
                len: label.len(),
                font: Font {
                    weight: FontWeight(430.),
                    ..font(SANS)
                },
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let width = window
                .text_system()
                .shape_line(label.clone(), px(13.), &[run], None)
                .width;
            // `.text-fade-truncate` masks the label's last 16 px: in its own layer, as Chromium
            // paints a masked box. Only when glyphs reach the fade. An unselected tab shows the
            // `×` (and its `pe-5`) on hover only, so its label box is 20 px wider until then.
            let closable = selected || hovered;
            let label_width = if closable {
                slot_width - 59.
            } else {
                slot_width - 39.
            };
            let fade = width > px(label_width - FADE);
            let focus = crate::focus::handle(("side-tab", ix), 0, window, cx);
            let control = div()
                .id(("side-tab", ix))
                .tab_index(0)
                .role(gpui::Role::Tab)
                .aria_label(label.clone())
                .on_key_down(move |event, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        window.dispatch_action(Box::new(ActivateSideTab(ix)), cx);
                        cx.stop_propagation();
                    }
                })
                .relative()
                .w(px(slot_width - 2.))
                .h(px(32.))
                .flex()
                .items_center()
                .pl(px(10.))
                .pr(px(7.))
                // `no-drag`: the titlebar behind must not start a window move.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.hovered = hovered.then_some(ix);
                    cx.notify();
                }))
                .when(!selected, |d| {
                    d.on_click(dispatch_action(ActivateSideTab(ix)))
                })
                // `_SelectedSurface`: its own squircle layer under the tab button, with a 0.5 px
                // `border-hairline`. Chromium composites it (its rect: the ink of
                // `0 1px 2px -1px`), so the shadow lands in the layer's transparent tiles and
                // blends onto the titlebar once. Hovered unselected: `bg-primary-ghost-hover`,
                // transparent border, no shadow.
                .when(closable, |d| {
                    d.child(
                        div()
                            .absolute()
                            .left(px(-1.))
                            .top_0()
                            .w(px(slot_width))
                            .h(px(34.))
                            .layer()
                            .child(
                                div()
                                    .absolute()
                                    .left(px(1.))
                                    .top_0()
                                    .w(px(slot_width - 2.))
                                    .h(px(32.))
                                    .child(shapes::decoration(Decoration {
                                        radius: Corners::all(Theme::RADIUS_MD),
                                        corner: shapes::Corner::Squircle,
                                        fill: Some(if selected { fill } else { t.ghost_hover }),
                                        border: selected.then_some((px(0.5), t.border)),
                                        shadows: if selected {
                                            vec![shadow(t.shadow_sm, 1., 2., -1., false)]
                                        } else {
                                            vec![]
                                        },
                                        ..Default::default()
                                    })),
                            ),
                    )
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .when(closable, |d| d.pr(px(20.)))
                        .child(icon(tab_icon, 16., color))
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .ml(px(-4.))
                                .overflow_hidden()
                                .child(text(label, 13., 18.5714, 430., color))
                                .when(fade, |d| d.layer().child(shapes::x_fade(px(0.), px(FADE)))),
                        ),
                )
                .when(closable, |d| {
                    let close_focus = crate::focus::handle(("side-tab-close", ix), 0, window, cx);
                    let close = div()
                        .id("side-panel-close-tab")
                        .tab_index(0)
                        .aria_label(format!("Close {} tab", self.tabs[ix].1))
                        .on_key_down(move |event, window, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                window.dispatch_action(Box::new(CloseSideTab(ix)), cx);
                                cx.stop_propagation();
                            }
                        })
                        .absolute()
                        .top(px(6.))
                        .right(px(7.))
                        .size(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(Box::new(CloseSideTab(ix)), cx);
                            cx.stop_propagation();
                        })
                        .child(icon("diff-tab-close", 14., t.text_tertiary));
                    d.child(crate::focus::ring(
                        close,
                        &close_focus,
                        Theme::RADIUS_SM,
                        px(0.),
                        cx,
                    ))
                });
            crate::focus::ring(control, &focus, Theme::RADIUS_MD, px(0.), cx)
        };
        // 28 px `_Button`: 12.5 px `--codex-corner-shape` squircle, `border border-transparent`.
        // Blink's clip layer for its fill composites onto the titlebar layer's transparent tile
        // unchanged, so the fill paints straight into the strip's layer.
        let button = |name: &str, color: Hsla, fill: Option<Hsla>| {
            div()
                .relative()
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .when_some(fill, |d, fill| {
                    d.child(shapes::decoration(Decoration {
                        radius: Corners::all(px(12.5)),
                        corner: shapes::Corner::Squircle,
                        fill: Some(fill),
                        ..Default::default()
                    }))
                })
                .child(icon(name, 16., color))
        };
        // `_strip` is `[contain:layout_paint]`.
        div()
            .relative()
            .snap_origin()
            .h(Theme::TITLEBAR_HEIGHT)
            .flex()
            .items_center()
            .pl(px(8.))
            // `border-b border-transparent`: the content box is 43 px, so the tab sits at y 5.5.
            .pb(px(1.))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(12.))
                    .w(px(1.))
                    .h(px(20.))
                    .bg(over(t.border, layout)),
            )
            .children(self.tabs.iter().enumerate().map(|(ix, t)| {
                // Each slot clips its tab shadow.
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .w(px(slot_width))
                    .pr(px(2.))
                    .overflow_hidden()
                    .child(tab(ix, t, cx))
            }))
            .child(
                div()
                    .pl(px(6.))
                    .child(button("diff-new-tab", t.text_tertiary, None)),
            )
            .child(
                div()
                    .absolute()
                    .top(px(8.))
                    .right(px(8.))
                    .flex()
                    .gap(px(6.))
                    .child(button("diff-full-view", t.text_tertiary, None))
                    // `bg-text/5` (dark: a float color, see `Theme::text_alpha_5`).
                    .child(
                        button("diff-hide-tabs", t.text, Some(t.text_alpha_5))
                            .id("hide-side-tabs")
                            .aria_label("Hide tabs")
                            .on_click(dispatch_action(ToggleSidePanelVisibility)),
                    ),
            )
    }

    fn compact_controls(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = cx.theme().clone();
        let focus = crate::focus::handle("compact-show-tabs", 0, window, cx);
        let count = div()
            .relative()
            .size(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .child(
                icon("tab-count", 16., t.text_tertiary)
                    .absolute()
                    .top_0()
                    .left_0(),
            )
            .child(text(
                self.tabs.len().to_string(),
                8.,
                8.,
                600.,
                t.text_tertiary,
            ));
        let show = crate::icon_button("compact-show-tabs", count, &t)
            .tab_index(0)
            .aria_label("Show tabs")
            .on_click(dispatch_action(ToggleSidePanelVisibility))
            .on_key_down(|event, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    window.dispatch_action(Box::new(ToggleSidePanelVisibility), cx);
                    cx.stop_propagation();
                }
            });
        div()
            .absolute()
            .right(px(8.))
            .top(px(8.))
            .flex()
            .gap(px(6.))
            .child(
                div()
                    .absolute()
                    .left(px(-7.))
                    .top(px(6.))
                    .w(px(1.))
                    .h(px(16.))
                    .bg(over(t.border, layout_color(&t))),
            )
            .child(
                crate::icon_button(
                    "compact-full-view",
                    icon("diff-full-view", 16., t.text_tertiary),
                    &t,
                )
                .aria_label("Enter full view"),
            )
            .child(crate::focus::ring(
                show,
                &focus,
                Theme::RADIUS_LG,
                px(0.),
                cx,
            ))
    }
}

/// Click handler dispatching `action`.
fn dispatch_action(action: impl Action) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)
}

/// Main panel of a thread with a side panel open: the thread, then the panel behind its 1 px
/// `border-l`, ending at the main panel's right edge. The divider (`Resize workspace panes`,
/// a 16 px hit area centered on the border) drags the panel's width live, edge
/// `left`: width = width at press - pointer travel, clamped so both panes keep `MIN_CHAT`.
struct SplitMain {
    thread: AnyView,
    panel: AnyView,
    visibility: Rc<Cell<crate::SidePanelVisibility>>,
    /// Width at press plus the pointer x at press, while the divider is dragged.
    drag_anchor: Option<Pixels>,
}

impl Render for SplitMain {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visibility.get().visible {
            return div().size_full().child(self.thread.clone());
        }
        let t = cx.theme();
        let width = panel_width(window, cx);
        div()
            .size_full()
            .relative()
            .flex()
            .child(div().flex_1().min_w_0().child(self.thread.clone()))
            .child(
                div()
                    .w(width)
                    .flex_none()
                    .border_l_1()
                    // The panel content's `[contain:layout_paint]`.
                    .snap_origin()
                    .border_color(over(t.border, t.surface))
                    .child(self.panel.clone()),
            )
            .child(
                div()
                    .id("panel-resize")
                    .role(gpui::Role::Splitter)
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(width - px(8.))
                    .w(px(16.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                            this.drag_anchor = Some(width + e.position.x);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .when_some(self.drag_anchor, |d, anchor| {
                let released = cx.entity().downgrade();
                d.child(drag_layer(
                    move |x, window, cx| {
                        cx.set_global(PanelWidth(
                            (anchor - x).clamp(px(MIN_CHAT), panel_max(window)),
                        ));
                    },
                    move |cx| {
                        released
                            .update(cx, |this, cx| {
                                this.drag_anchor = None;
                                cx.notify();
                            })
                            .ok();
                    },
                ))
            })
    }
}

/// The shell with the thread header and, with a side panel, the panel's tab strip laid over
/// its titlebar.
pub struct SplitScreen {
    shell: Entity<Shell>,
    thread: Entity<ThreadView>,
    /// The strip's tabs (empty: no panel) and the selected one, whose view the panel shows.
    tabs: Vec<Tab>,
    active: usize,
    /// The tab under the pointer.
    hovered: Option<usize>,
    visibility: Rc<Cell<crate::SidePanelVisibility>>,
    panel_layer: bool,
}

impl Render for SplitScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = window.viewport_size().width <= px(960.);
        let mut visibility = self.visibility.get();
        if compact != visibility.compact {
            visibility.compact = compact;
            visibility.visible = !compact;
            self.visibility.set(visibility);
        }
        let visible = !self.tabs.is_empty() && visibility.visible;
        let hidden_tabs = !self.tabs.is_empty() && !visible;
        self.thread.update(cx, |view, cx| {
            if view.clip_right != !visible {
                view.clip_right = !visible;
                cx.notify();
            }
        });
        self.shell.update(cx, |shell, cx| {
            if shell.main_clips_right != !visible
                || shell.main_layer != (visible && self.panel_layer)
            {
                shell.main_clips_right = !visible;
                shell.main_layer = visible && self.panel_layer;
                cx.notify();
            }
        });
        // The panel ends at the main panel's right edge, 4 px
        // (`--app-shell-page-inline-end-inset`) from the window's.
        let left = visible.then(|| window.viewport_size().width - px(4.) - panel_width(window, cx));
        let open = self.shell.read(cx).sidebar_open;
        let sidebar_width = self.shell.read(cx).sidebar_width;
        let header_left = if open {
            sidebar_width
        } else {
            TITLEBAR_LAYER_X
        };
        div()
            .size_full()
            .relative()
            .font_family(SANS)
            .on_key_down(crate::focus::tab)
            .on_action(
                cx.listener(|this, _: &ToggleSidePanelVisibility, window, cx| {
                    let mut state = this.visibility.get();
                    state.visible = !state.visible;
                    this.visibility.set(state);
                    window.refresh();
                    cx.notify();
                }),
            )
            .child(self.shell.clone())
            .child(
                self.thread
                    .update(cx, |view, cx| view.titlebar_header(cx))
                    .left(header_left)
                    .when_some(left, |h, left| h.w(left - header_left))
                    .when(hidden_tabs, |h| {
                        h.w(window.viewport_size().width - header_left - px(78.))
                    }),
            )
            .when(!open, |d| {
                let t = cx.theme();
                d.child(
                    crate::icon_button("new-chat", icon("new-chat", 16., t.text_tertiary), t)
                        .aria_label("New chat")
                        .tab_index(0)
                        .absolute()
                        .left(px(190.))
                        .top(px(8.))
                        .on_click(crate::dispatch::<crate::NewChat>),
                )
            })
            // Without a panel the shell's `New tab` group follows the header behind a divider.
            .when(open && left.is_none() && !hidden_tabs, |d| {
                let t = cx.theme();
                d.child(
                    div()
                        .absolute()
                        .top(px(14.))
                        .right(px(42.))
                        .w(px(1.))
                        .h(px(16.))
                        .bg(over(t.border, layout_color(t))),
                )
            })
            .when(hidden_tabs, |d| d.child(self.compact_controls(window, cx)))
            .when_some(left, |d, left| {
                d.child(
                    // In the composited `group/titlebar`, like the thread's header: the
                    // layer (and its tile grid) starts at its left edge. Its rect is
                    // Chromium's (2112 x 88 device px at 1280), so cc gives it long tiles.
                    div()
                        .absolute()
                        .top_0()
                        .left(TITLEBAR_LAYER_X)
                        .right_0()
                        .h(Theme::TITLEBAR_HEIGHT)
                        .cc_tiles()
                        .child(
                            self.tab_strip(&cx.theme().clone(), window, cx)
                                .absolute()
                                .top_0()
                                .left(left - TITLEBAR_LAYER_X)
                                .right_0(),
                        ),
                )
            })
            .when_some(
                self.hovered.filter(|ix| {
                    visible
                        && *ix != self.active
                        && self
                            .tabs
                            .get(*ix)
                            .is_some_and(|tab| tab.0 == "thread-terminal")
                }),
                |d, ix| {
                    let left = (left.unwrap_or_default() + px(9. + ix as f32 * 240.))
                        .min(window.viewport_size().width - px(328.))
                        .max(px(8.));
                    d.child(
                        deferred(
                            div()
                                .absolute()
                                .top(Theme::TITLEBAR_HEIGHT - px(3.5))
                                .left(left)
                                .child(crate::screens::terminal::thumbnail(
                                    self.thread.read(cx),
                                    window,
                                    cx,
                                )),
                        )
                        .with_priority(4),
                    )
                },
            )
    }
}

/// Left edge of Chromium's composited `group/titlebar` layer.
const TITLEBAR_LAYER_X: Pixels = px(224.);

/// A thread route (refs/thread): `thread` beside the sidebar; with `panel` (refs/diff,
/// refs/terminal) the panel view on the right under a titlebar tab `(view, icon, label)`.
pub fn split_screen(
    sidebar: AnyView,
    thread: Entity<ThreadView>,
    panel: Option<(AnyView, &'static str, SharedString)>,
    cx: &mut App,
) -> AnyView {
    let shell = cx.new(|_| {
        let mut shell = Shell::new(sidebar, thread.clone().into());
        shell.can_go_back = true;
        shell
    });
    with_side_panel(shell, thread, panel, cx).into()
}

/// `shell`'s thread route: `thread` in its main panel and, with `panel`, the panel on the
/// right under its titlebar tab `(view, icon, label)`.
pub fn with_side_panel(
    shell: Entity<Shell>,
    thread: Entity<ThreadView>,
    panel: Option<(AnyView, &'static str, SharedString)>,
    cx: &mut App,
) -> Entity<SplitScreen> {
    let panel = panel.map(|(view, icon, label)| (view, vec![(icon, label)], 0));
    with_side_tabs(shell, thread, panel, cx)
}

/// `shell`'s thread route: `thread` in its main panel and, with `panel`, the view of tab
/// `active` on the right under the titlebar strip of every open `tabs`.
pub fn with_side_tabs(
    shell: Entity<Shell>,
    thread: Entity<ThreadView>,
    panel: Option<(AnyView, Vec<Tab>, usize)>,
    cx: &mut App,
) -> Entity<SplitScreen> {
    let visibility = shell.read(cx).side_panel_visibility.clone();
    let panel_layer = panel
        .as_ref()
        .is_some_and(|(view, ..)| view.clone().downcast::<DiffPanel>().is_ok());
    let main = match &panel {
        Some((panel, ..)) => cx
            .new(|_| SplitMain {
                thread: thread.clone().into(),
                panel: panel.clone(),
                visibility: visibility.clone(),
                drag_anchor: None,
            })
            .into(),
        None => thread.clone().into(),
    };
    shell.update(cx, |shell, cx| {
        // A panel's tab strip brings its own `New tab`.
        shell.new_tab = panel.is_none();
        // The thread route's main surface is a composited layer; with the Changes panel,
        // Chromium composites the surface itself too.
        shell.main_composited = true;
        shell.main_layer = panel
            .as_ref()
            .is_some_and(|(view, ..)| view.clone().downcast::<DiffPanel>().is_ok());
        // A lone thread draws its own px at the surface's right corners.
        shell.main_clips_right = panel.is_none();
        shell.set_main(main, cx);
    });
    thread.update(cx, |thread, _| thread.clip_right = panel.is_none());
    let (tabs, active) = panel
        .map(|(_, tabs, active)| (tabs, active))
        .unwrap_or_default();
    cx.new(|cx: &mut Context<SplitScreen>| {
        cx.observe(&thread, |_, _, cx| cx.notify()).detach();
        SplitScreen {
            shell,
            thread,
            tabs,
            active,
            hovered: None,
            visibility,
            panel_layer,
        }
    })
}

/// A file link in an answer (`data-file-reference`): open the file viewer tab at `line`.
/// `path` is as written in the markdown; a relative one is under the thread's cwd.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = shell, no_json)]
pub struct OpenFile {
    pub path: String,
    pub line: Option<u32>,
}

/// The side panel's file viewer (`review-file-source-tab`): the breadcrumb header and the
/// file's lines, scrolled to the cited line.
pub struct FileView {
    /// Absolute path.
    pub path: String,
    /// The workspace folder name, then every path segment under it.
    pub crumbs: Vec<SharedString>,
    pub line: Option<u32>,
    /// `None` while the host reads; `Err` shows `Could not open file`.
    pub contents: Option<Result<String, String>>,
    scroll: ScrollHandle,
    focus: Option<FocusHandle>,
    go_to_line: Option<controls::TextField>,
    line_count: u32,
}

impl FileView {
    /// `workspace` is the thread's cwd; the server reports it canonicalized
    /// (`/private/tmp/...`) while links carry the written path, so the crumbs start at the
    /// first segment named like the workspace folder.
    pub fn new(workspace: &str, path: String, line: Option<u32>) -> Self {
        let root = workspace.trim_end_matches('/');
        let name = root.rsplit('/').next().unwrap_or(root);
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let start = segments.iter().position(|s| *s == name).unwrap_or(0);
        let crumbs = segments[start..]
            .iter()
            .map(|s| SharedString::from((*s).to_owned()))
            .collect();
        Self {
            path,
            crumbs,
            line,
            contents: None,
            scroll: ScrollHandle::new(),
            focus: None,
            go_to_line: None,
            line_count: 0,
        }
    }

    pub fn name(&self) -> SharedString {
        self.crumbs.last().cloned().unwrap_or_default()
    }

    /// The tab strip icon: the Python sprite for `.py`, else the file change card's file glyph.
    pub fn tab_icon(&self) -> &'static str {
        if self.path.ends_with(".py") {
            "diff-file-python"
        } else {
            "thread-file-diff"
        }
    }

    /// The host's answer; the cited line scrolls to the third row of the viewport.
    /// ponytail: a fixed top offset, not the original's centring, which was not measured.
    pub fn set_contents(&mut self, contents: Result<String, String>, cx: &mut Context<Self>) {
        self.line_count = contents
            .as_ref()
            .map_or(0, |text| controls::lines(text).count() as u32);
        if self.line_count == 0 {
            self.go_to_line = None;
        }
        if let (Ok(_), Some(line)) = (&contents, self.line) {
            let top = line.saturating_sub(3) as f32 * CODE_LINE;
            self.scroll.set_offset(gpui::point(px(0.), -px(top)));
        }
        self.contents = Some(contents);
        cx.notify();
    }

    fn header(&self, t: &Theme, clamped: bool) -> Div {
        let last = self.crumbs.len().saturating_sub(1);
        let crumbs = self.crumbs.iter().enumerate().flat_map(|(i, crumb)| {
            let color = if i == last { t.text } else { t.text_secondary };
            (i > 0)
                .then(|| icon("thread-chevron-right", 12., t.text_tertiary).into_any_element())
                .into_iter()
                .chain(std::iter::once(
                    text(crumb.clone(), 13., 19., 430., color).into_any_element(),
                ))
        });
        div()
            .h(px(48.))
            .flex_none()
            .p(px(8.))
            .flex()
            .justify_between()
            .gap(px(8.))
            .child(
                capsule(t, clamped).child(
                    div()
                        .id("file-path")
                        .role(gpui::Role::Group)
                        .aria_label("File path")
                        .h(px(28.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .px(px(8.))
                        .children(crumbs),
                ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(
                        capsule(t, clamped)
                            .child(
                                round_icon("diff-jump-to-file", 28., t)
                                    .id("file-search")
                                    .aria_label("Search in files"),
                            )
                            .child(
                                round_icon("diff-show-files", 28., t)
                                    .id("file-tree")
                                    .aria_label("Toggle file tree"),
                            ),
                    )
                    .child(
                        capsule(t, clamped).child(
                            div()
                                .h(px(28.))
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .pl(px(12.))
                                .pr(px(6.))
                                .child(text("Open", 13., 20., 430., t.text))
                                .child(icon("diff-chevron-down", 12., t.text)),
                        ),
                    ),
            )
    }
}

impl Render for FileView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        let body = match &self.contents {
            None => div().flex_1().into_any_element(),
            Some(Err(_)) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(text("Could not open file", 13., 20., 430., t.text_tertiary))
                .into_any_element(),
            Some(Ok(contents)) => {
                let syntax = Syntax::of(&t);
                let python = self.path.ends_with(".py");
                let lines: Vec<_> = controls::lines(contents).collect();
                let gutter = px(CH * lines.len().to_string().len().max(4) as f32 + 16.);
                let cited = over(t.info_solid.opacity(0.12), t.surface);
                div()
                    .id("file-lines")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .py(px(8.))
                    .font_family(MONO)
                    .text_size(px(CODE_SIZE))
                    .line_height(px(CODE_LINE))
                    .text_color(t.text)
                    .children(lines.into_iter().enumerate().map(|(i, line)| {
                        let number = i as u32 + 1;
                        let bg = if Some(number) == self.line {
                            cited
                        } else {
                            t.surface
                        };
                        let text: SharedString = line.to_owned().into();
                        let content = if python {
                            StyledText::new(text.clone()).with_highlights(highlight(&text, &syntax))
                        } else {
                            StyledText::new(text)
                        };
                        div()
                            .h(px(CODE_LINE))
                            .flex()
                            .bg(bg)
                            .child(
                                div()
                                    .w(gutter)
                                    .flex_none()
                                    .flex()
                                    .justify_end()
                                    .items_center()
                                    .pr(px(8.))
                                    .text_color(t.text_tertiary)
                                    .child(number.to_string()),
                            )
                            .child(div().flex_1().pl(px(8.)).whitespace_nowrap().child(content))
                    }))
                    .into_any_element()
            }
        };
        div()
            .relative()
            .track_focus(&focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, window, cx| window.focus(&focus, cx)),
            )
            .on_key_down(cx.listener(Self::line_key))
            .on_action(cx.listener(|this, _: &crate::Paste, _, cx| {
                if let Some(query) = &mut this.go_to_line {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        query.paste(&text);
                        cx.notify();
                    }
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &crate::SelectAll, _, cx| {
                if let Some(query) = &mut this.go_to_line {
                    query.selected = true;
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .border_b_1()
                    .border_color(t.border)
                    .child(self.header(&t, panel_clamped(window, cx))),
            )
            .child(body)
            .when(self.go_to_line.is_some(), |d| d.child(self.line_bar(cx)))
    }
}

/// A `rounded-full` `size` px box around a 16 px tertiary icon.
fn round_icon(name: &str, size: f32, t: &Theme) -> Div {
    div()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .child(icon(name, 16., t.text_tertiary))
}

/// `_capsule _controlSurface`: 32 px pill holding the header controls; `clamped`: see
/// [`panel_clamped`] (the backdrop's `gap_row`).
fn capsule(t: &Theme, clamped: bool) -> Div {
    let (fill, shadows) = match t.appearance {
        Appearance::Dark => (
            t.control_background,
            vec![
                shadow(t.border, 0., 0., 0.5, false),
                shadow(t.border, 0., 0., 0.5, false),
                shadow(black().opacity(0.12), 8., 16., -4., false),
            ],
        ),
        // The gradient is `rgba(255, 255, 255, 0.96)` in light.
        Appearance::Light => (
            t.control_background.opacity(0.96),
            vec![
                shadow(t.text.opacity(0.047), 0., 0., 1., false),
                shadow(black().opacity(0.05), 4., 16., 0., false),
            ],
        ),
    };
    // `backdrop-filter: blur(24px)` under a one-color `linear-gradient` fill.
    div()
        .relative()
        .h(px(32.))
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(2.))
        .child(shapes::backdrop_blur(
            px(24.),
            shapes::Fill::Gradient,
            clamped,
            Decoration {
                // `rounded-full`, constrained to the 32 px box's half height.
                radius: Corners::all(px(16.)),
                fill: Some(fill),
                shadows,
                ..Default::default()
            },
        ))
}

/// `+N -M` in git colors, `size` px with `tracking-tight` (-0.025em letter spacing),
/// `tabular-nums` and `disambiguated-digits` (`cv01`, `cv02`).
pub fn stats(added: usize, removed: usize, size: f32, t: &Theme) -> Div {
    let mut features = vec![("tnum".into(), 1), ("cv01".into(), 1), ("cv02".into(), 1)];
    features.extend(textsys::letter_spacing(-0.025 * size).0.iter().cloned());
    let features = FontFeatures(Arc::new(features));
    let tracked =
        |s: String, color: Hsla| text(s, size, size, 430., color).font_features(features.clone());
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(tracked(format!("+{added}"), t.git_added))
        .child(tracked(format!("-{removed}"), t.git_deleted))
}

fn file_header(
    i: usize,
    file: &FileDiff,
    panel: &DiffPanel,
    t: &Theme,
    cx: &Context<DiffPanel>,
) -> Stateful<Div> {
    if panel.standalone {
        return panel.standalone_file_header(i, file, t, cx);
    }
    let small = |name| round_icon(name, 20., t);
    let python = match t.appearance {
        Appearance::Dark => rgb(0x69b1ff),
        Appearance::Light => rgb(0x1a85d4),
    };
    div()
        .id(("diff-file", i))
        .flex_none()
        .h(px(38.))
        .mb(px(1.))
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(12.))
        .pr(px(8.))
        .border_b_1()
        .border_color(over(t.border, t.surface))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .pl(px(4.))
                .gap(px(8.))
                .when(file.name.ends_with(".py"), |d| {
                    // `#file-tree-builtin-python` sprite.
                    d.child(icon("diff-file-python", 16., python.into()))
                })
                .child(
                    div()
                        .flex()
                        .overflow_hidden()
                        .child(text(file.dir.clone(), 14., 21., 430., t.text_tertiary))
                        .child(text(file.name.clone(), 14., 21., 430., t.text)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(stats(file.added, file.removed, 14., t).mx(px(4.)))
                .when(
                    matches!(panel.source, DiffSource::Unstaged | DiffSource::Staged),
                    |d| {
                        d.child(
                            small("thread-undo")
                                .id(("revert-file", i))
                                .role(gpui::Role::Button)
                                .aria_label("Revert file")
                                .aria_disabled(panel.pending || panel.error.is_some())
                                .focusable()
                                .tab_index(0)
                                .focus_visible(|s| s.border_2().border_color(rgb(0x3a83f7)))
                                .cursor_pointer()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_key_down(cx.listener(
                                    move |this, event: &KeyDownEvent, _, cx| {
                                        if matches!(event.keystroke.key.as_str(), "enter" | "space")
                                        {
                                            this.request_revert(i, None, cx);
                                            cx.stop_propagation();
                                        }
                                    },
                                ))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.request_revert(i, None, cx);
                                    cx.stop_propagation();
                                })),
                        )
                    },
                )
                .child(
                    small("diff-open-in")
                        .id(("diff-open-editor", i))
                        .role(gpui::Role::Button)
                        .aria_label("Open in editor")
                        .focusable()
                        .tab_index(0)
                        .focus_visible(|s| s.border_2().border_color(rgb(0x3a83f7)))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                this.request_open(i, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.request_open(i, cx);
                            cx.stop_propagation();
                        })),
                )
                .child(
                    small("diff-more")
                        .id(("diff-actions", i))
                        .role(gpui::Role::Button)
                        .aria_label("File actions")
                        .focusable()
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(
                            move |this, event: &gpui::ClickEvent, window, cx| {
                                let position = event.position() + gpui::point(px(10.), px(12.));
                                this.open_menu(i, menu::Kind::Header, position, window, cx);
                            },
                        )),
                ),
        )
}

/// `--color-codex-syntax-*` of the codex themes.
struct Syntax {
    keyword: Hsla,
    name: Hsla,
    string: Hsla,
    comment: Hsla,
    literal: Hsla,
    parameter: Hsla,
    punctuation: Hsla,
    operator: Hsla,
}

impl Syntax {
    fn of(t: &Theme) -> Self {
        let c = |dark: u32, light: u32| -> Hsla {
            rgb(if t.appearance == Appearance::Dark {
                dark
            } else {
                light
            })
            .into()
        };
        Self {
            keyword: c(0xf67576, 0xd53538),
            name: c(0xb06dff, 0x751ed9),
            string: c(0x83d197, 0x3a843f),
            comment: c(0xb9b9b9, 0x4f4f4f),
            literal: c(0xf1a275, 0xac4f23),
            parameter: c(0xfa994c, 0xbd5800),
            punctuation: c(0x999999, 0x666666),
            operator: c(0x6dcbf4, 0x0071ea),
        }
    }
}

const KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];

/// Python token colors for one line: keywords, `def`/`class` names, strings, comments, numbers.
/// ponytail: a line tokenizer, so strings spanning lines color only their first line.
fn highlight(line: &str, s: &Syntax) -> Vec<(Range<usize>, HighlightStyle)> {
    let b = line.as_bytes();
    let (mut i, mut prev, mut out) = (0, "", Vec::new());
    let signature =
        line.trim_start().starts_with("def ") || line.trim_start().starts_with("async def ");
    while i < b.len() {
        let start = i;
        let color = match b[i] {
            b'#' => {
                i = b.len();
                Some(s.comment)
            }
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(b.len());
                Some(s.string)
            }
            c if c.is_ascii_digit() => {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.') {
                    i += 1;
                }
                Some(s.literal)
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                let word = &line[start..i];
                let color = if matches!(word, "def" | "class") {
                    Some(s.name)
                } else if KEYWORDS.contains(&word) {
                    Some(s.keyword)
                } else if matches!(word, "True" | "False" | "None") {
                    Some(s.literal)
                } else if matches!(prev, "def" | "class") {
                    Some(s.name)
                } else if signature {
                    Some(s.parameter)
                } else {
                    None
                };
                prev = word;
                color
            }
            b'(' | b')' | b'[' | b']' | b'{' | b'}' | b',' | b':' | b'.' => {
                i += 1;
                Some(s.punctuation)
            }
            b'+' | b'-' | b'*' | b'/' | b'%' | b'=' | b'<' | b'>' | b'&' | b'|' | b'^' | b'~' => {
                i += 1;
                Some(s.operator)
            }
            _ => {
                i += 1;
                None
            }
        };
        if let Some(color) = color {
            let style = HighlightStyle {
                color: Some(color),
                ..Default::default()
            };
            out.push((start..i, style));
        }
    }
    out
}

/// `color-mix(in srgb, a (1 - w), b w)`.
fn mix(a: Hsla, b: Hsla, w: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    let m = |x: f32, y: f32| x + (y - x) * w;
    Rgba {
        r: m(a.r, b.r),
        g: m(a.g, b.g),
        b: m(a.b, b.b),
        a: 1.,
    }
    .into()
}

fn line_colors(kind: Kind, t: &Theme) -> (Hsla, Hsla, Hsla) {
    let color = |dark, light| {
        rgb(if t.appearance == Appearance::Dark {
            dark
        } else {
            light
        })
        .into()
    };
    match kind {
        Kind::Context => (t.surface, t.surface, color(0xa1a1a1, 0x585858)),
        Kind::Added => (
            color(0x132017, 0xedf7ed),
            color(0x1f3124, 0xe7f4e7),
            t.git_added,
        ),
        Kind::Removed => (
            color(0x28130e, 0xfdece9),
            color(0x3b1f1a, 0xfbe6e2),
            t.git_deleted,
        ),
    }
}

fn word_mark(line: &Line, layout: gpui::TextLayout, t: &Theme) -> Option<gpui::AnyElement> {
    let range = line.word_change.clone()?;
    let text = line.text.clone();
    let color = if line.kind == Kind::Added {
        t.git_added
    } else {
        t.git_deleted
    };
    Some(
        gpui::canvas(
            |_, _, _| {},
            move |_, (), window, cx| {
                let Some(mut start) = layout.position_for_index(range.start) else {
                    return;
                };
                let mut end = start;
                let decoration = Decoration {
                    radius: Corners::all(px(3.)),
                    fill: Some(color.opacity(0.2)),
                    ..Default::default()
                };
                let paint =
                    |start: gpui::Point<Pixels>, end: gpui::Point<Pixels>, window: &mut Window| {
                        shapes::paint(
                            &decoration,
                            None,
                            gpui::Bounds::new(
                                start + gpui::point(px(0.), px(3.5)),
                                gpui::size(end.x - start.x, px(14.)),
                            ),
                            window,
                            cx,
                        );
                    };
                for (offset, ch) in text[range.clone()].char_indices() {
                    let Some(here) = layout.position_for_index(range.start + offset) else {
                        continue;
                    };
                    if here.y != start.y {
                        paint(start, end, window);
                        start = here;
                    }
                    let next = layout.position_for_index(range.start + offset + ch.len_utf8());
                    end = next
                        .filter(|p| p.y == here.y)
                        .unwrap_or(here + gpui::point(px(CH), px(0.)));
                }
                paint(start, end, window);
            },
        )
        .absolute()
        .size_full()
        .into_any_element(),
    )
}

fn hunk(
    h: &Hunk,
    t: &Theme,
    line_index: &Cell<usize>,
    panel: &DiffPanel,
    split: bool,
    cx: &Context<DiffPanel>,
) -> Div {
    let syntax = Syntax::of(t);
    let digits = h
        .lines
        .iter()
        .map(|l| l.number)
        .max()
        .unwrap_or(0)
        .to_string()
        .len();
    let gutter = px(52.5625 + CH * digits.saturating_sub(4) as f32);
    let base_index = line_index.get();
    line_index.set(base_index + h.lines.len());
    let content_width = h
        .lines
        .iter()
        .map(|line| line.text.len())
        .max()
        .unwrap_or(0) as f32
        * CH;
    let minimum_width = (gutter + px(content_width + CH)) * if split { 2. } else { 1. };
    let row = |number: Option<u32>,
               (number_bg, number_fg): (Hsla, Hsla),
               bg: Hsla,
               content: StyledText,
               mark: Option<gpui::AnyElement>,
               bar: Option<Hsla>| {
        div()
            .h(px(CODE_LINE))
            .when(panel.wrap, |d| d.h_auto().min_h(px(CODE_LINE)))
            .flex()
            .gap(px(2.))
            .child(
                div()
                    .relative()
                    .w(gutter - px(2.))
                    .flex_none()
                    .h_full()
                    .when(panel.wrap, |d| d.h_auto())
                    .flex()
                    .justify_end()
                    .items_center()
                    .when(panel.wrap, |d| d.items_start())
                    .pr(px(CH))
                    .child(shapes::decoration(Decoration {
                        fill: Some(number_bg),
                        ..Default::default()
                    }))
                    .when_some(bar, |d, color| {
                        d.child(div().absolute().left_0().top_0().w(px(4.)).h_full().child(
                            shapes::decoration(Decoration {
                                fill: Some(color),
                                ..Default::default()
                            }),
                        ))
                    })
                    .text_color(number_fg)
                    .when_some(number, |d, n| d.child(n.to_string())),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .h_full()
                    .when(panel.wrap, |d| d.h_auto().min_h(px(CODE_LINE)).min_w_0())
                    .flex()
                    .items_center()
                    .when(panel.wrap, |d| d.flex_col().items_stretch())
                    .pl(px(7.21875))
                    .child(shapes::decoration(Decoration {
                        fill: Some(bg),
                        ..Default::default()
                    }))
                    .whitespace_nowrap()
                    .when(panel.wrap, |d| d.whitespace_normal())
                    .children(mark)
                    .child(content),
            )
    };
    let separator = mix(t.surface, t.info_solid, 0.06);
    let render_line = |l: &Line, index: usize, number: u32| {
        let (number_bg, bg, number_fg) = line_colors(l.kind, t);
        let found = l.matches.iter().enumerate().map(|(ordinal, range)| {
            let index = l.match_start + ordinal;
            (
                range.clone(),
                HighlightStyle {
                    color: Some(t.text),
                    background_color: Some(
                        rgb(if index == panel.active_match {
                            0xfb6a22
                        } else {
                            0xffd240
                        })
                        .into(),
                    ),
                    ..Default::default()
                },
            )
        });
        let selection = panel.selection.and_then(|(anchor, head)| {
            let (start, end) = (anchor.min(head), anchor.max(head));
            if index < start.0 || index > end.0 {
                return None;
            }
            let a = if index == start.0 { start.1 } else { 0 };
            let b = if index == end.0 { end.1 } else { l.text.len() };
            (a < b).then_some((
                a..b,
                HighlightStyle {
                    background_color: Some(t.text_selection),
                    ..Default::default()
                },
            ))
        });
        let word = l.word_change.clone().map(|range| {
            (
                range,
                HighlightStyle {
                    color: Some(t.text),
                    ..Default::default()
                },
            )
        });
        let content = StyledText::new(l.text.clone()).with_highlights(combine_highlights(
            combine_highlights(combine_highlights(highlight(&l.text, &syntax), word), found),
            selection,
        ));
        let down_layout = content.layout().clone();
        let move_layout = down_layout.clone();
        let mark = word_mark(l, down_layout.clone(), t);
        row(
            Some(number),
            (number_bg, number_fg),
            bg,
            content,
            mark,
            (l.kind != Kind::Context).then_some(number_fg),
        )
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                let offset = down_layout
                    .index_for_position(event.position)
                    .unwrap_or_else(|i| i);
                let point = (index, offset);
                let anchor = if event.modifiers.shift {
                    this.selection.map_or(point, |s| s.0)
                } else {
                    point
                };
                this.selection = Some((anchor, point));
                this.selecting = true;
                if let Some(focus) = &this.focus {
                    window.focus(focus, cx);
                }
                cx.stop_propagation();
                cx.notify();
            }),
        )
        .on_mouse_move(
            cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                if this.selecting && event.pressed_button == Some(MouseButton::Left) {
                    let offset = move_layout
                        .index_for_position(event.position)
                        .unwrap_or_else(|i| i);
                    if let Some(selection) = &mut this.selection {
                        selection.1 = (index, offset);
                        cx.notify();
                    }
                }
            }),
        )
    };
    div()
        .flex()
        .flex_col()
        .when(!panel.wrap, |d| d.min_w(minimum_width))
        .font_family(MONO)
        .font_weight(FontWeight(430.))
        .text_size(px(CODE_SIZE))
        .line_height(px(CODE_LINE))
        .text_color(rgb(if t.appearance == Appearance::Dark {
            0xfcfcfc
        } else {
            0x0d0d0d
        }))
        .when_some(h.separator.clone(), |d, label| {
            let len = label.len();
            d.child(row(
                None,
                (separator, t.text_tertiary),
                separator,
                StyledText::new(label).with_highlights([(
                    0..len,
                    HighlightStyle {
                        color: Some(t.text_tertiary),
                        ..Default::default()
                    },
                )]),
                None,
                None,
            ))
        })
        .map(|d| {
            if split {
                d.children(controls::split_rows(&h.lines).map(|(old, new)| {
                    div()
                        .flex()
                        .gap(px(2.))
                        .items_stretch()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .min_h(px(CODE_LINE))
                                .children(old.map(|i| {
                                    render_line(&h.lines[i], base_index + i, h.lines[i].old_number)
                                })),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .min_h(px(CODE_LINE))
                                .children(new.map(|i| {
                                    render_line(&h.lines[i], base_index + i, h.lines[i].number)
                                })),
                        )
                }))
            } else {
                d.children(
                    h.lines
                        .iter()
                        .enumerate()
                        .map(|(i, line)| render_line(line, base_index + i, line.number)),
                )
            }
        })
}

/// Hunks of a unified diff; file headers (`diff --git`, `---`, `+++`) before the first `@@`
/// are skipped.
fn parse(diff: &str, raw: Option<Kind>) -> Result<Vec<Hunk>, &'static str> {
    const TOO_LARGE: &str = "Diff too large to display";
    const INVALID: &str = "Diff failed to render";
    if diff.len() > 2_000_000
        || diff
            .lines()
            .enumerate()
            .any(|(index, line)| index >= 20_000 || line.len() > 10_000)
    {
        return Err(TOO_LARGE);
    }
    if let Some(kind) = raw {
        return Ok(vec![Hunk {
            separator: None,
            lines: diff
                .lines()
                .enumerate()
                .map(|(i, text)| Line {
                    kind,
                    number: i as u32 + 1,
                    old_number: i as u32 + 1,
                    text: text.to_owned().into(),
                    matches: Vec::new(),
                    match_start: 0,
                    word_change: None,
                })
                .collect(),
        }]);
    }
    let mut hunks: Vec<Hunk> = Vec::new();
    let (mut old, mut new): (u32, u32) = (1, 0);
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("@@ -") {
            let start = |s: &str| {
                s.split(',')
                    .next()
                    .and_then(|n| n.parse::<u32>().ok())
                    .ok_or(INVALID)
            };
            let mut ranges = rest.split(' ');
            let old_start = start(ranges.next().ok_or(INVALID)?)?;
            let skipped = old_start.saturating_sub(old);
            old = old_start;
            new = start(
                ranges
                    .next()
                    .and_then(|range| range.strip_prefix('+'))
                    .ok_or(INVALID)?,
            )?;
            if ranges.next() != Some("@@") {
                return Err(INVALID);
            }
            hunks.push(Hunk {
                separator: (skipped > 0).then(|| {
                    format!(
                        "{skipped} unmodified line{}",
                        if skipped == 1 { "" } else { "s" }
                    )
                    .into()
                }),
                lines: Vec::new(),
            });
            continue;
        }
        let Some(hunk) = hunks.last_mut() else {
            continue;
        };
        let (kind, number) = match line.as_bytes().first() {
            Some(b'+') => (Kind::Added, new),
            Some(b'-') => (Kind::Removed, old),
            Some(b' ') | None => (Kind::Context, new),
            _ => continue,
        };
        let old_number = old;
        if kind != Kind::Removed {
            new = new.checked_add(1).ok_or(INVALID)?;
        }
        if kind != Kind::Added {
            old = old.checked_add(1).ok_or(INVALID)?;
        }
        hunk.lines.push(Line {
            kind,
            number,
            old_number,
            text: line.get(1..).unwrap_or("").to_owned().into(),
            matches: Vec::new(),
            match_start: 0,
            word_change: None,
        });
    }
    for hunk in &mut hunks {
        let mut removed = 0..0;
        for index in 0..hunk.lines.len() {
            match hunk.lines[index].kind {
                Kind::Removed => {
                    if index == 0 || hunk.lines[index - 1].kind != Kind::Removed {
                        removed = index..index + 1;
                    } else {
                        removed.end = index + 1;
                    }
                }
                Kind::Added => {
                    if let Some(old) = removed.next() {
                        let (a, b) = word_changes(&hunk.lines[old].text, &hunk.lines[index].text);
                        hunk.lines[old].word_change = a;
                        hunk.lines[index].word_change = b;
                    }
                }
                Kind::Context => removed = 0..0,
            }
        }
    }
    Ok(hunks)
}

// ponytail: one changed span per paired line; use token diff for separate marks within a line.
fn word_changes(old: &str, new: &str) -> (Option<Range<usize>>, Option<Range<usize>>) {
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let end_old = old.len() - suffix;
    let end_new = new.len() - suffix;
    (
        (prefix < end_old).then_some(prefix..end_old),
        (prefix < end_new).then_some(prefix..end_new),
    )
}

/// `path` relative to directory `base`, both absolute (`path.relative` of the original).
fn relative(base: &str, path: &str) -> String {
    let base: Vec<_> = base.split('/').filter(|s| !s.is_empty()).collect();
    let path: Vec<_> = path.split('/').filter(|s| !s.is_empty()).collect();
    let common = base.iter().zip(&path).take_while(|(a, b)| a == b).count();
    let up = std::iter::repeat_n("..", base.len() - common);
    up.chain(path[common..].iter().copied())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtract_fixture() {
        let panel = DiffPanel::subtract();
        let [file] = &panel.files[..] else {
            panic!("one file")
        };
        assert_eq!(file.dir, "../../../private/tmp/cgx-mac/proj/src/");
        assert_eq!(file.name, "math.py");
        assert_eq!((file.added, file.removed), (4, 0));
        assert_eq!(
            file.hunks[0].separator.as_deref(),
            Some("1 unmodified line")
        );
        let lines = &file.hunks[0].lines;
        assert_eq!((lines[0].kind, lines[0].number), (Kind::Context, 2));
        assert_eq!((lines[4].kind, lines[4].number), (Kind::Added, 6));
    }

    #[test]
    fn word_changes_keep_unicode_boundaries_and_pair_replacements() {
        assert_eq!(word_changes("é = old", "é = new"), (Some(5..8), Some(5..8)));
        assert_eq!(word_changes("αβ", "αxβ"), (None, Some(2..3)));
        assert_eq!(word_changes("same", "same"), (None, None));
        let hunks = parse(
            "@@ -1,3 +1,3 @@\n-a = 1\n-b = 2\n+a = 3\n+b = 4\n same",
            None,
        )
        .unwrap();
        let lines = &hunks[0].lines;
        assert_eq!(
            lines
                .iter()
                .map(|line| line.word_change.clone())
                .collect::<Vec<_>>(),
            vec![Some(4..5), Some(4..5), Some(4..5), Some(4..5), None]
        );
    }
}
