//! Side panel of a thread route: the split layout and titlebar tab strip that refs/diff and
//! refs/terminal share, and the Changes panel with the unified diff of the last turn's file
//! changes. All numbers are the spec's CSS px.

use crate::workspace::WorkspaceLayout;
use crate::{
    MONO, SANS, Shell, composer::shadow, drag_layer, fixture, icon::icon, layout_color, over,
    screens::thread::view::ThreadView, text,
};
use bridge::{
    Event,
    types::{ThreadItem, ThreadReadResponse, ThreadStartResponse, Turn},
};
use gpui::{
    Action, AnimationExt as _, AnyView, App, AppContext as _, ClickEvent, ClipboardItem, Context,
    Corners, CursorStyle, Div, Entity, FocusHandle, Font, FontFeatures, FontWeight, Global,
    HighlightStyle, Hsla, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, ParentElement, Pixels, Render, ScrollHandle, SharedString, Stateful,
    StatefulInteractiveElement, Styled, StyledText, TextRun, Window, black, combine_highlights,
    deferred, div, font, prelude::FluentBuilder as _, px, rgb,
};
use shapes::Decoration;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ops::Range,
    rc::Rc,
    sync::Arc,
};
use theme::{ActiveTheme as _, Appearance, Theme};

#[path = "diff/menu.rs"]
mod menu;

#[path = "diff/controls.rs"]
mod controls;

#[path = "diff/syntax.rs"]
mod syntax;

#[path = "diff/navigation.rs"]
mod navigation;

#[path = "diff/find.rs"]
mod find;
pub use controls::DiffLayout;
pub(crate) use find::find_scope_buttons;
pub use find::{FindInChat, FindInDiff};

#[path = "diff/states.rs"]
mod states;
pub use states::{LastTurnState, OmittedFiles};

#[path = "diff/preview.rs"]
mod preview;
pub use preview::{PreviewContent, PreviewRequest, PreviewSource};

#[path = "diff/pdf.rs"]
mod pdf;

#[path = "diff/comment_view.rs"]
mod comment_view;
#[path = "diff/comments.rs"]
pub mod comments;

#[path = "diff/repositories.rs"]
mod repositories;
pub use repositories::ReviewRepository;

#[path = "diff/context.rs"]
mod context;
#[path = "diff/imports.rs"]
mod imports;
#[path = "diff/options.rs"]
mod options;
#[path = "diff/viewed.rs"]
mod viewed;
pub use context::{ContextRequest, OpenTargetRequest, OpenTargets};

#[path = "diff/content.rs"]
mod content;
pub use content::{
    BlobError, BlobReply, BlobRequest, FullError, FullFileLoad, FullFileRequest, FullText,
};

#[path = "diff/word.rs"]
pub mod word;

#[path = "diff/blame.rs"]
mod blame;
#[path = "diff/file_tree.rs"]
mod file_tree;
#[path = "diff/patch.rs"]
pub mod patch;
pub use blame::{BlameLine, GitBlame};
pub use file_tree::FileTreeRequest;

/// Panel width the capture had stored (`app-shell:right-panel-width:v3`, a width ratio) before
/// clamping. The spec prints it as 664.17; the 2x refs put its edge at 85014 LayoutUnits (1/128
/// px at 2x), so the computed width is at least that, which Blink's truncation keeps.
const STORED_WIDTH: f32 = 85014. / 128.;
/// Room the chat keeps beside the panel (`unified` layout), also the panel's minimum.
const MIN_CHAT: f32 = 320.;
/// `--diffs-font-size`, `--diffs-line-height`.
const CODE_SIZE: f32 = 12.;
const CODE_LINE: f32 = 21.6;
/// The row box: Blink rounds the 1.8 line height to 2765/64 device px at 2x, while an authored
/// length truncates (`to_taffy`), so 21.6 would leave each row 1/64 px short.
const CODE_ROW: f32 = 2765. / 128.;
const FILE_LINE_HEIGHT: f32 = 22.;
/// Menlo advance at 12 px (0.602em); `--diffs-min-number-column-width` is 4ch.
const CH: f32 = 7.224_609_4;
const MAX_DIFF_LINES: usize = 15_000;
const MAX_DIFF_UNITS: usize = 3_145_728;
const MAX_DIFF_LINE_UNITS: usize = 1_048_576;

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

#[derive(Clone)]
struct Line {
    kind: Kind,
    /// New-file number; old-file number for removed lines.
    number: u32,
    text: SharedString,
    matches: Vec<Range<usize>>,
    old_number: u32,
    match_start: usize,
    word_changes: Vec<Range<usize>>,
    syntax: Vec<syntax::Mark>,
}

#[derive(Clone)]
struct Hunk {
    old_start: u32,
    new_start: u32,
    /// `N unmodified lines` bar for the old-file lines the diff skips before this hunk.
    separator: Option<SharedString>,
    lines: Vec<Line>,
    layout: Rc<RefCell<content::ContextLayout>>,
}

pub struct FileDiff {
    /// Workspace-relative directory part (tertiary) and file name of the header path.
    pub dir: SharedString,
    pub name: SharedString,
    path_key: String,
    display_dir: SharedString,
    display_name: SharedString,
    pub added: usize,
    pub removed: usize,
    changed_bytes: usize,
    hunks: Vec<Hunk>,
    pub open: bool,
    problem: Option<&'static str>,
    raw: Arc<str>,
    raw_kind: Option<Kind>,
    line_info_separators: bool,
    full: Option<FullText>,
    full_error: Option<FullError>,
    full_loading: bool,
    original_hunks: Option<Vec<Hunk>>,
    imports: Option<imports::ImportFilter>,
    pending_expansion: Option<(usize, content::Expand)>,
    preview: Option<preview::Preview>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiffSource {
    LastTurn,
    Uncommitted,
    Unstaged,
    Staged,
    Branch,
    Commits,
    Cloud,
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
            Self::Cloud => "Cloud changes",
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
            Self::Cloud => "cloud",
        }
    }

    fn mutable(self) -> bool {
        matches!(self, Self::Unstaged | Self::Staged)
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
    OpenTab {
        path: String,
        line: Option<u32>,
    },
    SourceChanged,
    Notice(&'static str),
    CopyGitApply,
    ContextMenu(ContextRequest),
    OpenTarget(OpenTargetRequest),
    LoadFullFile(FullFileLoad),
    LoadPreview(PreviewRequest),
    Preference(&'static str, serde_json::Value),
    ViewedFile {
        key: String,
        revision: Option<String>,
    },
    CommentsChanged(Vec<comments::Comment>),
    DismissModelComment(String),
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
    #[serde(default, rename = "untrackedFilesOmitted")]
    pub untracked_files_omitted: Option<OmittedFiles>,
    #[serde(default)]
    pub base_branch: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub head_branch: Option<String>,
    #[serde(default)]
    pub before_ref: Option<String>,
    #[serde(default)]
    pub after_ref: Option<String>,
    #[serde(default, rename = "hideWhitespace")]
    pub hide_whitespace: bool,
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
    match_total: usize,
    find_scroll: find::FindScroll,
    active_match: usize,
    selected_query: bool,
    aggregated_turn: Option<String>,
    selection: Option<((usize, usize), (usize, usize))>,
    selecting: bool,
    menu: Option<menu::Menu>,
    workspace: String,
    cwd: String,
    files_expanded: bool,
    file_expansion: HashMap<String, bool>,
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
    container_width: Option<Pixels>,
    base_branch: Option<String>,
    commit: Option<String>,
    repositories: Vec<ReviewRepository>,
    selected_repository: Option<String>,
    word_diffs: bool,
    hide_whitespace: bool,
    hide_imports: bool,
    jump: Option<navigation::JumpToFile>,
    watch_limited: bool,
    last_turn_state: Option<LastTurnState>,
    full_files: bool,
    content_generation: u64,
    rich_preview: bool,
    annotations: comment_view::Annotations,
    context: context::State,
    full_view: Option<gpui::WeakEntity<Shell>>,
    viewed_files: serde_json::Map<String, serde_json::Value>,
}

impl DiffPanel {
    pub fn file_tree_open(&self) -> bool {
        self.show_tree
    }

    pub fn toggle_file_tree(&mut self, cx: &mut Context<Self>) {
        self.show_tree = !self.show_tree;
        cx.notify();
    }

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
            let path = std::path::Path::new(&change.path);
            let path = if path.starts_with(cwd) {
                change.path.clone()
            } else if let Ok(relative) = path.strip_prefix(workspace_root) {
                std::path::Path::new(cwd)
                    .join(relative)
                    .to_string_lossy()
                    .into_owned()
            } else {
                change.path.clone()
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
            let (added, removed) = if problem.is_some() {
                patch::counts(&change.diff, raw)
            } else {
                (count(Kind::Added), count(Kind::Removed))
            };
            match files.iter_mut().find(|f| f.dir == dir && f.name == name) {
                Some(f) => {
                    f.added += added;
                    f.removed += removed;
                    f.changed_bytes += change.diff.len();
                    f.hunks.extend(hunks);
                    f.raw = format!("{}\n{}", f.raw, change.diff).into();
                }
                None => {
                    let dir: SharedString = dir.to_owned().into();
                    let name: SharedString = name.to_owned().into();
                    let (display_dir, display_name) = controls::display_parts(&shown, &dir, &name);
                    files.push(FileDiff {
                        dir,
                        name,
                        path_key: path,
                        display_dir,
                        display_name,
                        added,
                        removed,
                        changed_bytes: change.diff.len(),
                        hunks,
                        open: true,
                        problem,
                        raw: change.diff.as_str().into(),
                        raw_kind: raw,
                        line_info_separators: false,
                        full: None,
                        full_error: None,
                        full_loading: false,
                        original_hunks: None,
                        imports: None,
                        pending_expansion: None,
                        preview: None,
                    });
                }
            }
        }
        for file in &mut files {
            syntax::apply(&file.name, &mut file.hunks);
        }
        let mut panel = Self {
            files,
            focus: None,
            query: None,
            matches: 0,
            match_total: 0,
            find_scroll: find::FindScroll::default(),
            active_match: 0,
            selected_query: false,
            aggregated_turn: None,
            selection: None,
            selecting: false,
            menu: None,
            workspace: workspace_root.to_owned(),
            cwd: cwd.to_owned(),
            files_expanded: true,
            file_expansion: HashMap::new(),
            wrap: false,
            source: DiffSource::LastTurn,
            turn_files: Vec::new(),
            repository: None,
            pending: false,
            error: None,
            layout: DiffLayout::Auto,
            word_diffs: true,
            hide_whitespace: false,
            hide_imports: false,
            show_tree: false,
            active_file: 0,
            filter: Default::default(),
            filter_focus: None,
            scroll: ScrollHandle::new(),
            standalone: false,
            container_width: None,
            base_branch: None,
            commit: None,
            repositories: Vec::new(),
            selected_repository: None,
            jump: None,
            watch_limited: false,
            last_turn_state: None,
            full_files: true,
            content_generation: 0,
            rich_preview: false,
            annotations: comment_view::Annotations::default(),
            context: context::State::default(),
            full_view: None,
            viewed_files: Default::default(),
        };
        panel
            .annotations
            .models(turns, cwd, &[workspace_root.to_owned()]);
        panel
    }

    pub fn set_container_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        if self.container_width != Some(width) {
            self.container_width = Some(width);
            cx.notify();
        }
    }

    pub fn pending(&self) -> bool {
        self.pending
    }

    pub fn repository_error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn review_params(&self) -> serde_json::Value {
        serde_json::json!({"cwd": self.working_directory(), "reviewSource": self.source.key(), "baseBranch": self.base_branch, "commit": self.commit, "hideWhitespace": self.hide_whitespace})
    }

    pub fn view_changes(&mut self, path: &str, cx: &mut Context<Self>) {
        if let Some(query) = &mut self.query {
            query.clear();
        }
        self.filter = Default::default();
        self.select_source(DiffSource::LastTurn, cx);
        self.search_changed();
        self.set_all_files_open(false);
        self.file_expansion.insert(path.to_owned(), true);
        if let Some(index) = self.files.iter().position(|file| file.path_key == path) {
            self.files[index].open = true;
            self.active_file = index;
            self.scroll.scroll_to_top_of_item(index);
        }
        cx.notify();
    }

    fn set_all_files_open(&mut self, open: bool) {
        self.files_expanded = open;
        self.file_expansion.clear();
        for file in &mut self.files {
            file.open = open;
        }
    }

    fn set_file_open(&mut self, index: usize, open: bool) {
        let file = &mut self.files[index];
        file.open = open;
        if let Some(value) = self.file_expansion.get_mut(&file.path_key) {
            *value = open;
        } else {
            self.file_expansion.insert(file.path_key.clone(), open);
        }
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
        let refresh = result.as_ref().is_ok_and(|repository| {
            !repository.not_repository && repository.hide_whitespace != self.hide_whitespace
        });
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
        self.refresh_picker();
        if refresh && !matches!(self.source, DiffSource::LastTurn | DiffSource::Cloud) {
            cx.emit(DiffRequest::Refresh);
        }
        cx.notify();
    }

    fn request_revert(&self, file: usize, hunk: Option<usize>, cx: &mut Context<Self>) {
        if self.pending
            || self.error.is_some()
            || !matches!(self.source, DiffSource::Unstaged | DiffSource::Staged)
        {
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
        cx.emit(DiffRequest::SourceChanged);
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
        self.apply_viewed_files();
    }

    pub fn search(&self) -> Option<(&str, usize)> {
        self.query.as_deref().map(|query| (query, self.match_total))
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle());
        window.focus(focus, cx);
    }

    fn available_width(&self, window: &Window, cx: &App) -> Pixels {
        if self.standalone {
            window.viewport_size().width
        } else if let Some(shell) = self.full_view.as_ref().and_then(|shell| shell.upgrade()) {
            shell.read(cx).main_width(window, cx)
        } else {
            panel_width(window, cx) - px(1.)
        }
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

    pub fn apply(&mut self, event: &Event, cwd: &str, turns: &[Turn], cx: &mut Context<Self>) {
        let Event::Notification { method, params } = event else {
            return;
        };
        let Some(turn) = turns.last() else { return };
        if method == "turn/completed" {
            self.annotations.models(
                std::slice::from_ref(turn),
                cwd,
                std::slice::from_ref(&self.workspace),
            );
            cx.notify();
            return;
        }
        if params["turnId"].as_str() != Some(&turn.id) {
            return;
        }
        let had_changes = if self.source == DiffSource::LastTurn {
            !self.files.is_empty()
        } else {
            !self.turn_files.is_empty()
        };
        let files = match method.as_str() {
            "turn/diff/updated" => {
                let Some(diff) = params["diff"].as_str() else {
                    return;
                };
                self.aggregated_turn = Some(turn.id.clone());
                controls::files(diff)
            }
            "item/started" | "item/completed" | "item/fileChange/patchUpdated"
                if (method == "item/fileChange/patchUpdated"
                    || params["item"]["type"] == "fileChange")
                    && self.aggregated_turn.as_deref() != Some(&turn.id) =>
            {
                self.aggregated_turn = None;
                Self::last_turn(&self.workspace, cwd, turns).files
            }
            _ => return,
        };
        self.last_turn_state = if files.is_empty() {
            self.last_turn_state
                .or(had_changes.then_some(LastTurnState::CommittedOrReverted))
        } else {
            None
        };
        let mut files = files;
        for file in &mut files {
            if !file.path_key.starts_with('/') {
                file.path_key = menu::absolute_path(cwd, &file.path_key);
            }
        }
        if self.source == DiffSource::LastTurn {
            self.replace_files(files);
        } else {
            self.turn_files = files;
        }
        cx.notify();
    }

    fn replace_files(&mut self, mut files: Vec<FileDiff>) {
        self.content_generation = self.content_generation.wrapping_add(1);
        self.cancel_context_request();
        for file in &mut files {
            if !file.path_key.starts_with('/') {
                file.path_key = menu::absolute_path(self.file_root(), &file.path_key);
            }
        }
        self.active_file = self
            .files
            .get(self.active_file)
            .and_then(|old| files.iter().position(|file| file.path_key == old.path_key))
            .unwrap_or(0);
        for file in &mut files {
            file.full_loading = false;
            if let Some(old) = self
                .files
                .iter_mut()
                .find(|old| old.path_key == file.path_key)
                && file.raw == old.raw
            {
                file.full = old.full.take();
                file.imports = old.imports.take();
                file.full_error = old.full_error.take();
                file.original_hunks = old.original_hunks.take();
                if file.original_hunks.is_some() {
                    file.hunks = std::mem::take(&mut old.hunks);
                }
                if old.preview.as_ref().is_some_and(preview::Preview::loaded) {
                    file.preview = old.preview.take();
                }
            }
            file.open = self
                .file_expansion
                .get(&file.path_key)
                .copied()
                .unwrap_or(self.files_expanded);
        }
        self.files = files;
        self.refresh_jump();
        self.selection = None;
        self.selecting = false;
        if self.menu.as_ref().is_some_and(|m| {
            matches!(
                m.kind,
                menu::Kind::Header | menu::Kind::Body | menu::Kind::OpenWith
            )
        }) {
            self.menu = None;
        }
        self.refresh_filtered_view();
    }

    fn search_changed(&mut self) {
        let query = self.query.as_deref().unwrap_or("").trim().to_lowercase();
        self.matches = 0;
        self.match_total = 0;
        for file in &mut self.files {
            let before = self.matches;
            for (index, hunk) in file.hunks.iter_mut().enumerate() {
                let hidden = file
                    .imports
                    .as_ref()
                    .is_some_and(|filter| filter.hidden.get(index) == Some(&true));
                for line in &mut hunk.lines {
                    line.match_start = self.matches;
                    if hidden {
                        line.matches.clear();
                        continue;
                    }
                    let (matches, total) = crate::screens::thread::view::find_ranges(
                        &line.text,
                        &query,
                        250 - self.matches,
                    );
                    line.matches = matches;
                    self.matches += line.matches.len();
                    self.match_total += total;
                }
            }
            if self.matches > before {
                file.open = true;
                if let Some(value) = self.file_expansion.get_mut(&file.path_key) {
                    *value = true;
                } else {
                    self.file_expansion.insert(file.path_key.clone(), true);
                }
            }
        }
        self.active_match = self.active_match.min(self.matches.saturating_sub(1));
        self.reveal_find_match();
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
            .flat_map(|file| file.visible_hunks().flat_map(|(_, hunk)| &hunk.lines))
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
        if self.context.pending && event.keystroke.key == "escape" {
            self.cancel_context_request();
            cx.stop_propagation();
            return;
        }
        if self.jump.is_some() {
            self.jump_key(event, cx);
            return;
        }
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
        } else if key.key == "enter" && self.query.is_some() {
            self.next_find(key.modifiers.shift, cx);
            cx.stop_propagation();
            return;
        } else if let Some(query) = &mut self.query {
            match key.key.as_str() {
                "escape" => self.query = None,
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
        self.find_scroll.pending.set(self.matches > 0);
        self.find_scroll.typing.set(true);
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
            self.match_total
        );
        let button = |id, name, label, size, color| {
            div()
                .size(px(size))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(10.))
                .child(icon(name, 16., color))
                .id(id)
                .role(gpui::Role::Button)
                .aria_label(label)
                .cursor_pointer()
                .hover(|style| style.bg(t.text_alpha_10))
        };
        let find_shadow = shadow(black().opacity(0.12), 8., 16., -4., false);
        let bar = div()
            .w(px(340.))
            .h(px(81.))
            .rounded(px(20.))
            .p(px(0.5))
            .child(shapes::decoration(Decoration {
                fill: Some(t.surface_secondary),
                border: Some((px(0.5), t.border)),
                radius: Corners::all(px(20.)),
                shadows: vec![find_shadow],
                ..Default::default()
            }))
            .flex()
            .flex_col()
            .font_family(SANS)
            // `overflow-hidden rounded-[20px]` with a 0.5px border: the content sits in an AA
            // rounded clip (padding box, radius 19.5) that changes the icons' edge pixels.
            .child(crate::rounded_clip(
                px(19.5),
                shapes::Corner::Round,
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .h(px(44.))
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .pl(px(16.))
                                    .gap(px(8.))
                                    .child(icon("diff-find-search", 16., t.text))
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
                                                430.,
                                                if query.is_empty() {
                                                    t.text_tertiary
                                                } else {
                                                    t.text
                                                },
                                            )),
                                    ),
                            )
                            .when(!self.standalone, |d| {
                                d.child(find_scope_buttons(query, true, t))
                            })
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .pr(px(16.))
                                    .child(div().mx(px(8.)).w(px(1.)).h(px(16.)).bg(t.border))
                                    .child(
                                        button(
                                            "diff-find-close",
                                            "diff-find-close",
                                            "Close find",
                                            24.,
                                            t.text,
                                        )
                                        .mx(px(-2.))
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.close_find(cx)),
                                        ),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .px(px(16.))
                            .gap(px(12.))
                            .border_t_1()
                            .border_color(t.border)
                            .child(
                                button(
                                    "diff-find-previous",
                                    "diff-find-previous",
                                    "Previous result",
                                    16.,
                                    t.text_tertiary,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.next_find(true, cx);
                                    },
                                )),
                            )
                            .child(
                                button(
                                    "diff-find-next",
                                    "diff-find-next",
                                    "Next result",
                                    16.,
                                    t.text_tertiary,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.next_find(false, cx);
                                    },
                                )),
                            )
                            .child(div().flex_1())
                            .child(
                                text(count, 14., 24., 430., t.text_tertiary)
                                    .self_start()
                                    .mt(px(7.)),
                            ),
                    ),
            ));
        // Include the shadow's three-sigma raster extent in the composited layer.
        let outset = find_shadow.blur * 1.5 + find_shadow.spread;
        deferred(
            div()
                .absolute()
                .top(px(-36.) - (outset - find_shadow.offset.y))
                .right(px(12.) - outset)
                .w(px(340.) + outset * 2.)
                .h(px(81.) + outset * 2.)
                .pt(outset - find_shadow.offset.y)
                .pl(outset)
                .top_layer()
                .child(bar),
        )
        .with_priority(3)
    }

    fn header(
        &self,
        t: &Theme,
        wide: bool,
        clamped: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> Div {
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
                .when(self.show_tree, |d| {
                    d.child(shapes::decoration(Decoration {
                        radius: Corners::all(px(14.)),
                        fill: Some(over(t.accent.opacity(0.1), t.surface)),
                        ..Default::default()
                    }))
                })
                .child(icon(
                    name,
                    16.,
                    if self.show_tree {
                        if t.appearance == Appearance::Dark {
                            over(t.accent.opacity(0.61), gpui::white())
                        } else {
                            t.text_accent
                        }
                    } else {
                        t.text_tertiary
                    },
                ))
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
                    .children(self.repository_control(t, cx))
                    .child(
                        div()
                            .id("diff-source")
                            .role(if self.source == DiffSource::Cloud {
                                gpui::Role::Label
                            } else {
                                gpui::Role::Button
                            })
                            .aria_label(if self.source == DiffSource::Cloud {
                                "Cloud changes"
                            } else {
                                "Review source"
                            })
                            .aria_expanded(source_open)
                            .when(self.source != DiffSource::Cloud, |d| {
                                d.tab_index(0).cursor_pointer().on_click(cx.listener(
                                    |this, event: &ClickEvent, window, cx| {
                                        this.open_menu(
                                            0,
                                            menu::Kind::Source,
                                            event.position(),
                                            window,
                                            cx,
                                        );
                                    },
                                ))
                            })
                            .relative()
                            .h(px(28.))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .pl(px(12.))
                            .pr(px(6.))
                            .child(lens())
                            .child(text(self.source.label(), 13., 20., 430., t.text))
                            .when(self.source != DiffSource::Cloud, |d| {
                                d.child(icon("diff-chevron-down", 12., t.text))
                            }),
                    )
                    .children(self.comparison_control(t, cx))
                    .child(stats(added, removed, 14., t).mr(px(4.))),
            )
            .child(
                capsule(t, clamped)
                    .child(
                        tool("diff-more")
                            .id("diff-options")
                            .relative()
                            .role(gpui::Role::Button)
                            .aria_label("Changes options")
                            .tab_index(0)
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
                            }))
                            .when(
                                self.menu
                                    .as_ref()
                                    .is_some_and(|menu| menu.kind == menu::Kind::Options),
                                |button| {
                                    button.child(
                                        deferred(
                                            div().absolute().top_0().right_0().child(
                                                gpui::anchored()
                                                    .anchor(gpui::Anchor::TopRight)
                                                    .offset(gpui::point(
                                                        px(menu::SHADOW_SIDE_PADDING),
                                                        px(29.),
                                                    ))
                                                    .snap_to_window_with_margin(px(8.))
                                                    .child(self.options_popup(window, cx)),
                                            ),
                                        )
                                        .with_priority(5),
                                    )
                                },
                            ),
                    )
                    .child(
                        tool("diff-jump-to-file")
                            .id("diff-jump")
                            .role(gpui::Role::Button)
                            .aria_label("Jump to file")
                            .tab_index(0)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| this.open_jump(window, cx)))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.open_jump(window, cx);
                                    cx.stop_propagation();
                                }
                            })),
                    )
                    // `@container review-header (min-width: 625px)`
                    .when(wide, |d| {
                        d.child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .when(self.source != DiffSource::Cloud, |d| {
                                    d.child(
                                        tool("diff-refresh")
                                            .id("diff-refresh")
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if !this.pending {
                                                    cx.emit(DiffRequest::Refresh);
                                                }
                                            })),
                                    )
                                })
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
                                        .tab_index(0)
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.toggle_wrap(cx);
                                            cx.notify();
                                        }))
                                        .on_key_down(cx.listener(
                                            |this, event: &KeyDownEvent, _, cx| {
                                                if matches!(
                                                    event.keystroke.key.as_str(),
                                                    "enter" | "space"
                                                ) {
                                                    this.toggle_wrap(cx);
                                                    cx.stop_propagation();
                                                    cx.notify();
                                                }
                                            },
                                        )),
                                )
                                .child(
                                    tool(if self.all_expanded() {
                                        "diff-collapse-all"
                                    } else {
                                        "diff-expand-all"
                                    })
                                    .id("diff-expand-all")
                                    .role(gpui::Role::Button)
                                    .aria_label(if self.all_expanded() {
                                        "Collapse all diffs"
                                    } else {
                                        "Expand all diffs"
                                    })
                                    .tab_index(0)
                                    .cursor_pointer()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_all_files(cx)),
                                    )
                                    .on_key_down(cx.listener(
                                        |this, event: &KeyDownEvent, _, cx| {
                                            if matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            ) {
                                                this.toggle_all_files(cx);
                                                cx.stop_propagation();
                                            }
                                        },
                                    )),
                                )
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
                                this.toggle_file_tree(cx);
                            }))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    this.toggle_file_tree(cx);
                                    cx.stop_propagation();
                                }
                            })),
                    ),
            )
    }
}

impl Render for DiffPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.request_visible_full_files(cx);
        *self.find_scroll.target.borrow_mut() = None;
        self.find_scroll.lines.borrow_mut().clear();
        let t = cx.theme().clone();
        let width = self
            .container_width
            .unwrap_or_else(|| self.available_width(window, cx));
        let clamped = if self.full_view.is_some() {
            width < px(STORED_WIDTH)
        } else {
            panel_clamped(window, cx)
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
        self.request_visible_previews(code_width, window.scale_factor(), cx);
        let not_repository = self.source != DiffSource::LastTurn
            && self.repository.as_ref().is_some_and(|r| r.not_repository);
        let boundary = if not_repository {
            None
        } else {
            self.boundary(&t, cx)
        };
        let mut status = self.repository_status(&t, cx);
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        let capped = self.capped();
        let mut notices = self.state_notices(capped, window, cx);
        let first_line = if capped {
            self.files
                .iter()
                .take(self.active_file)
                .flat_map(|file| &file.hunks)
                .map(|hunk| hunk.lines.len())
                .sum()
        } else {
            0
        };
        let line_index = Cell::new(first_line);
        let (first, offset) = self.scroll.logical_scroll_top();
        let scroll_top = (if capped { self.active_file } else { first }, offset);
        let files = self
            .files
            .iter()
            .enumerate()
            .filter(|(index, _)| !capped || *index == self.active_file)
            .map(|(i, file)| {
                div()
                    .relative()
                    .pt(px(if self.standalone { 36. } else { 39. }))
                    .flex()
                    .flex_col()
                    .flex_none()
                    .pb(px(2.))
                    .when(self.standalone, |d| {
                        d.overflow_hidden().child(shapes::decoration(Decoration {
                            fill: Some(t.surface),
                            ..Default::default()
                        }))
                    })
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                            this.request_context_menu(i, None, event.position, window, cx);
                        }),
                    )
                    .when(file.open, |d| d.children(self.full_error_notice(i, cx)))
                    .map(|d| {
                        if file.open
                            && let Some(preview) = self.preview(i, code_width, window, cx)
                        {
                            d.child(preview)
                        } else if file.open && file.problem.is_some() {
                            d.child(self.file_placeholder(i, file, &t, cx))
                        } else if file.open {
                            d.child(
                                div()
                                    .relative()
                                    .flex()
                                    .flex_col()
                                    .layer()
                                    .child(shapes::decoration(Decoration {
                                        fill: Some(t.surface),
                                        ..Default::default()
                                    }))
                                    .children(file.visible_hunks().map(|(index, h)| {
                                        let controls =
                                            matches!(
                                                self.source,
                                                DiffSource::Unstaged | DiffSource::Staged
                                            ) && h.lines.iter().any(|l| l.kind != Kind::Context);
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
                                                (i, index),
                                                &line_index,
                                                self,
                                                code_width,
                                                window,
                                                cx,
                                            ))
                                    })),
                            )
                        } else {
                            line_index.set(
                                line_index.get()
                                    + file
                                        .visible_hunks()
                                        .map(|(_, hunk)| hunk.lines.len())
                                        .sum::<usize>(),
                            );
                            d
                        }
                    })
                    .child(self.scrolling_file_header(i, file, scroll_top, &t, cx))
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
            .key_context("SideTabs")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.selection = None;
                    this.cancel_context_request();
                    this.selecting = false;
                    window.focus(&focus, cx);
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|this, _: &crate::Copy, window, cx| {
                if let Some(jump) = &this.jump {
                    if jump.query.selected {
                        cx.write_to_clipboard(ClipboardItem::new_string(jump.query.text.clone()));
                    }
                } else if this
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
                    .menu
                    .as_ref()
                    .is_some_and(|menu| menu.kind == menu::Kind::Branches)
                {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        this.paste_picker(&text, cx);
                    }
                } else if let Some(jump) = &mut this.jump {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        jump.query.paste(&text);
                        this.refresh_jump();
                        cx.notify();
                    }
                } else if this
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
                if this.select_picker_text(cx) {
                    return;
                }
                if let Some(jump) = &mut this.jump {
                    jump.query.selected = true;
                    cx.notify();
                } else if this
                    .filter_focus
                    .as_ref()
                    .is_some_and(|focus| focus.is_focused(window))
                {
                    this.filter.selected = true;
                    cx.notify();
                } else if this.query.is_some() {
                    this.selected_query = true;
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.selecting = false;
                    this.end_gutter_selection();
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
                    .when(!self.standalone && notices.is_none(), |d| d.pb(px(1.)))
                    // Skia blends it like Chromium (a GPUI quad rounds apart).
                    .when(!self.standalone && notices.is_none(), |d| {
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
                        self.header(&t, wide, clamped, window, cx)
                    })
                    // The original clips header shadows with the notices, not at the header edge.
                    .when(notices.is_some(), |d| {
                        d.children(status.take()).children(notices.take())
                    }),
            )
            .children(status)
            .children(notices)
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
                                .aria_label(if self.pending {
                                    "Creating…"
                                } else {
                                    "Create git repository"
                                })
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
                                .child(text(
                                    if self.pending {
                                        "Creating…"
                                    } else {
                                        "Create git repository"
                                    },
                                    13.,
                                    20.,
                                    430.,
                                    t.text,
                                )),
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
                            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
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
            .when(self.jump.is_some(), |d| {
                d.child(self.jump_overlay(window, cx))
            })
            .when(self.query.is_some(), |d| {
                d.child(
                    self.find_scroll
                        .probe(self.scroll.clone(), cx.entity().entity_id()),
                )
            })
    }
}

/// A side panel tab, with italic text for a preview file.
#[derive(Clone)]
pub struct Tab {
    pub icon: &'static str,
    pub label: SharedString,
    pub preview: bool,
}

impl Tab {
    fn label_font(&self) -> Font {
        Font {
            weight: FontWeight(430.),
            style: if self.preview {
                gpui::FontStyle::Italic
            } else {
                gpui::FontStyle::Normal
            },
            ..font(SANS)
        }
    }
}

/// Click on a tab of the side panel's strip (`activateOnClick`): index into the strip.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = ui, no_json)]
pub struct ActivateSideTab(pub usize);

/// A tab's `Close {title} tab` button: index into the strip.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = ui, no_json)]
pub struct CloseSideTab(pub usize);

fn side_tab_icon(name: &'static str, index: usize, color: Hsla) -> gpui::AnyElement {
    let icon = icon(name, 16., color);
    if name != "sidebar-cmd-spinner" {
        return icon.into_any_element();
    }
    icon.with_animation(
        ("side-chat-running", index),
        gpui::Animation::new(std::time::Duration::from_secs(1)).repeat_synced(),
        |icon, progress| {
            icon.with_transformation(gpui::Transformation::rotate(gpui::radians(
                progress * std::f32::consts::TAU,
            )))
        },
    )
    .into_any_element()
}

impl SplitScreen {
    pub fn set_side_chat_running(&mut self, index: usize, running: bool, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(index) {
            let icon = if running {
                "sidebar-cmd-spinner"
            } else {
                "side-chat"
            };
            if tab.icon != icon {
                tab.icon = icon;
                cx.notify();
            }
        }
    }

    /// A browser tab's page title changed: its strip label follows.
    pub fn set_tab_label(&mut self, index: usize, label: SharedString, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(index)
            && tab.label != label
        {
            tab.label = label;
            cx.notify();
        }
    }

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
        let mut tab = |ix: usize, tab: &Tab, cx: &mut Context<Self>| {
            let Tab {
                icon: tab_icon,
                label,
                preview,
            } = tab;
            let selected = ix == self.active;
            let hovered = self.hovered == Some(ix);
            let color = if selected { t.text } else { t.text_tertiary };
            let label = label.clone();
            let run = TextRun {
                len: label.len(),
                font: tab.label_font(),
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
                        .child(side_tab_icon(tab_icon, ix, color))
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .ml(px(-4.))
                                .overflow_hidden()
                                .child(
                                    text(label, 13., 18.5714, 430., color)
                                        .when(*preview, |label| label.italic()),
                                )
                                .when(fade, |d| d.layer().child(shapes::x_fade(px(0.), px(FADE)))),
                        ),
                )
                .when(closable, |d| {
                    let close_focus = crate::focus::handle(("side-tab-close", ix), 0, window, cx);
                    let close = div()
                        .id("side-panel-close-tab")
                        .tab_index(0)
                        .aria_label(format!("Close {} tab", self.tabs[ix].label))
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
                    .child(strip_button("diff-new-tab", t.text_tertiary, None)),
            )
            .child(self.layout_controls(t))
    }

    fn layout_controls(&self, t: &Theme) -> Div {
        let full = self.layout == WorkspaceLayout::Full;
        let (glyph, label, step_label) = if full {
            (
                "workspace-exit-full-view",
                "Exit full view",
                "Enter split view",
            )
        } else {
            ("diff-full-view", "Enter full view", "Hide tabs")
        };
        div()
            .absolute()
            .top(px(8.))
            .right(px(8.))
            .flex()
            .gap(px(6.))
            .child(
                strip_button(glyph, t.text_tertiary, full.then_some(t.text_alpha_5))
                    .id("workspace-full-view")
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .focusable()
                    .tab_index(0)
                    .on_key_down(dispatch_key(crate::ShowWorkspaceTabView))
                    .focus_visible(|style| style.border_2().border_color(rgb(0x3a83f7)))
                    .on_click(dispatch_action(crate::ShowWorkspaceTabView)),
            )
            .child(
                strip_button("diff-hide-tabs", t.text, Some(t.text_alpha_5))
                    .id("workspace-step-layout")
                    .role(gpui::Role::Button)
                    .aria_label(step_label)
                    .focusable()
                    .tab_index(0)
                    .on_key_down(dispatch_key(crate::StepWorkspaceLayout))
                    .focus_visible(|style| style.border_2().border_color(rgb(0x3a83f7)))
                    .on_click(dispatch_action(crate::StepWorkspaceLayout)),
            )
    }

    /// The compact titlebar of a hidden panel: a `w-px` divider, then `Enter full view` and
    /// `Show tabs` with the tab count.
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
            .on_click(dispatch_action(crate::StepWorkspaceLayout))
            .on_key_down(dispatch_key(crate::StepWorkspaceLayout));
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
                .aria_label("Enter full view")
                .on_click(dispatch_action(crate::ShowWorkspaceTabView)),
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

// Paint the fill directly into the titlebar layer's transparent tile.
fn strip_button(name: &str, color: Hsla, fill: Option<Hsla>) -> Div {
    div()
        .relative()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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
}

/// Click handler dispatching `action`.
fn dispatch_action(action: impl Action) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)
}

fn dispatch_key(action: impl Action) -> impl Fn(&KeyDownEvent, &mut Window, &mut App) {
    move |event, window, cx| {
        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
            window.dispatch_action(action.boxed_clone(), cx);
            cx.stop_propagation();
        }
    }
}

/// Main panel of a thread with a side panel open: the thread, then the panel behind its 1 px
/// `border-l`, ending at the main panel's right edge. The divider (`Resize workspace panes`,
/// a 16 px hit area centered on the border) drags the panel's width live, edge
/// `left`: width = width at press - pointer travel, clamped so both panes keep `MIN_CHAT`.
struct SplitMain {
    thread: AnyView,
    panel: AnyView,
    /// Width at press plus the pointer x at press, while the divider is dragged.
    drag_anchor: Option<Pixels>,
    layout: WorkspaceLayout,
    focus: FocusHandle,
}

impl Render for SplitMain {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.layout == WorkspaceLayout::Full {
            return div()
                .size_full()
                .track_focus(&self.focus)
                .child(self.panel.clone());
        }
        let width = panel_width(window, cx);
        // The original's resizer covers the panel's left 16 px (not centered on the border),
        // except while the thread summary pane lies over it.
        let summary_open = self
            .thread
            .clone()
            .downcast::<ThreadView>()
            .ok()
            .is_some_and(|thread| thread.read(cx).summary_open);
        let resizer_focus = crate::focus::handle("panel-resize", -1, window, cx)
            .read(cx)
            .handle
            .clone()
            .tab_stop(false);
        let t = cx.theme();
        div()
            .size_full()
            .relative()
            .flex()
            .child(div().flex_1().min_w_0().child(self.thread.clone()))
            .child(
                div()
                    .w(width)
                    .flex_none()
                    .track_focus(&self.focus)
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
                    .track_focus(&resizer_focus)
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(width - px(if summary_open { 8. } else { 16. }))
                    .w(px(16.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            window.focus(&resizer_focus, cx);
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
    layout: WorkspaceLayout,
    panel_focus: FocusHandle,
}

impl gpui::Focusable for SplitScreen {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.panel_focus.clone()
    }
}

impl Render for SplitScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The panel ends at the main panel's right edge, 4 px
        // (`--app-shell-page-inline-end-inset`) from the window's.
        let left = match self.layout {
            WorkspaceLayout::Chat => None,
            WorkspaceLayout::Split => {
                Some(window.viewport_size().width - px(4.) - panel_width(window, cx))
            }
            WorkspaceLayout::Full => Some(if self.shell.read(cx).sidebar_open {
                self.shell.read(cx).sidebar_width
            } else {
                crate::RAIL_WIDTH
            }),
        };
        let open = self.shell.read(cx).sidebar_open;
        let chat = self.layout == WorkspaceLayout::Chat;
        self.thread
            .update(cx, |view, cx| view.set_clips(chat, chat && !open, cx));
        // A visible panel's tab strip brings its own `New tab`; a hidden one at compact width
        // shows its own controls instead.
        let compact =
            chat && !self.tabs.is_empty() && window.viewport_size().width <= SIDE_PANEL_BREAKPOINT;
        let new_tab = chat && !compact;
        self.shell.update(cx, |shell, cx| {
            if shell.new_tab != new_tab {
                shell.new_tab = new_tab;
                cx.notify();
            }
        });
        let sidebar_width = self.shell.read(cx).sidebar_width;
        // Without an open sidebar the header starts where the leading titlebar controls end.
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
            .child(self.shell.clone())
            .when(self.layout != WorkspaceLayout::Full, |d| {
                d.child(titlebar_layer(
                    open.then_some(sidebar_width),
                    self.thread
                        .update(cx, |view, cx| view.titlebar_header(left.is_some(), cx))
                        .when_some(left, |h, left| h.w(left - header_left))
                        .when(compact, |h| {
                            h.w(window.viewport_size().width - header_left - COMPACT_CONTROLS_WIDTH)
                        }),
                    header_left,
                ))
            })
            // Without a panel the shell's `New tab` group follows the header behind a divider.
            .when(open && left.is_none() && !compact, |d| {
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
            .when(chat && !self.tabs.is_empty() && !compact, |d| {
                d.child(
                    div()
                        .id("workspace-show-tabs")
                        .role(gpui::Role::Button)
                        .aria_label("Show tabs")
                        .focusable()
                        .tab_index(0)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .absolute()
                        .top(px(8.))
                        .right(px(8.))
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_click(dispatch_action(crate::StepWorkspaceLayout))
                        .on_key_down(dispatch_key(crate::StepWorkspaceLayout))
                        .focus_visible(|style| style.border_2().border_color(rgb(0x3a83f7)))
                        .child(icon("diff-hide-tabs", 16., cx.theme().text_tertiary)),
                )
            })
            .when(compact, |d| d.child(self.compact_controls(window, cx)))
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
                    left.is_some()
                        && *ix != self.active
                        && self
                            .tabs
                            .get(*ix)
                            .is_some_and(|tab| tab.icon == "thread-terminal")
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

/// Chromium's composited `group/titlebar` layer around the thread `header` at `header_left`:
/// from the end of the leading titlebar controls (`TITLEBAR_LAYER_X`), or from the titlebar
/// box where that starts further left (the rail, with no open sidebar), to the window's right
/// edge. cc tiles only its recorded bounds, the union of what the header paints.
fn titlebar_layer(sidebar: Option<Pixels>, header: Div, header_left: Pixels) -> Div {
    let layer_left = TITLEBAR_LAYER_X.min(sidebar.unwrap_or(crate::RAIL_WIDTH));
    div()
        .absolute()
        .top_0()
        .left(layer_left)
        .right_0()
        .h(Theme::TITLEBAR_HEIGHT)
        .layer()
        .child(crate::recorded_bounds(
            header.left(header_left - layer_left),
        ))
}

/// Left edge of Chromium's composited `group/titlebar` layer.
const TITLEBAR_LAYER_X: Pixels = px(224.);

/// At most this wide, a thread's side panel hides like the original's compact layout.
pub const SIDE_PANEL_BREAKPOINT: Pixels = px(960.);

/// The compact titlebar's controls (`compact_controls`): a `w-px` divider, then `Enter full
/// view` and `Show tabs` (`size-7`), `gap-1.5` apart, inside `pe-2`. The thread's toolbar ends
/// at the divider.
const COMPACT_CONTROLS_WIDTH: Pixels = px(1. + 6. + 28. + 6. + 28. + 8.);

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
    let panel = panel.map(|(view, icon, label)| {
        (
            view,
            vec![Tab {
                icon,
                label,
                preview: false,
            }],
            0,
        )
    });
    with_side_tabs(shell, thread, panel, WorkspaceLayout::Split, cx)
}

/// `shell`'s thread route: `thread` in its main panel and, with `panel`, the view of tab
/// `active` on the right under the titlebar strip of every open `tabs`.
pub fn with_side_tabs(
    shell: Entity<Shell>,
    thread: Entity<ThreadView>,
    panel: Option<(AnyView, Vec<Tab>, usize)>,
    layout: WorkspaceLayout,
    cx: &mut App,
) -> Entity<SplitScreen> {
    let layout = panel.as_ref().map_or(WorkspaceLayout::Chat, |_| layout);
    if let Some((panel, ..)) = &panel
        && let Ok(panel) = panel.clone().downcast::<DiffPanel>()
    {
        panel.update(cx, |panel, cx| {
            panel.full_view = (layout == WorkspaceLayout::Full).then(|| shell.downgrade());
            cx.notify();
        });
    }
    let (main, panel_focus) =
        workspace_content(&thread, panel.as_ref().map(|(view, ..)| view), layout, cx);
    shell.update(cx, |shell, cx| {
        // File and Changes panels also composite the surface itself.
        shell.main_composited = true;
        shell.main_opaque_scroll = panel.as_ref().and_then(|(view, ..)| {
            view.clone()
                .downcast::<DiffPanel>()
                .ok()
                .map(|panel| panel.read(cx).scroll.clone())
        });
        shell.main_layer = shell.main_opaque_scroll.is_some()
            || panel
                .as_ref()
                .is_some_and(|(view, ..)| view.clone().downcast::<FileView>().is_ok());
        // A lone thread draws its own px at the surface's right corners.
        shell.main_clips_right = layout == WorkspaceLayout::Chat;
        shell.set_main(main, cx);
    });
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
            layout,
            panel_focus,
        }
    })
}

fn workspace_content(
    thread: &Entity<ThreadView>,
    panel: Option<&AnyView>,
    layout: WorkspaceLayout,
    cx: &mut App,
) -> (AnyView, FocusHandle) {
    let focus = cx.focus_handle();
    let main = match panel.filter(|_| layout != WorkspaceLayout::Chat) {
        Some(panel) => cx
            .new(|_| SplitMain {
                thread: thread.clone().into(),
                panel: panel.clone(),
                drag_anchor: None,
                layout,
                focus: focus.clone(),
            })
            .into(),
        None => thread.clone().into(),
    };
    (main, focus)
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
    syntax: Vec<Vec<syntax::Mark>>,
    blame: Vec<BlameLine>,
    tree: file_tree::WorkspaceTree,
    revision: u64,
    preview: bool,
    local_controls: bool,
    /// An image or PDF ([`Self::previews`]) shown instead of text lines.
    media: Option<preview::FileMedia>,
}

impl FileView {
    /// `workspace` is the thread's cwd; the server reports it canonicalized
    /// (`/private/tmp/...`) while links carry the written path, so the crumbs start at the
    /// first segment named like the workspace folder.
    pub fn new(workspace: &str, path: String, line: Option<u32>) -> Self {
        let crumbs = Self::breadcrumbs(workspace, &path);
        let tree = file_tree::WorkspaceTree::new(workspace, &path);
        Self {
            path,
            crumbs,
            line,
            contents: None,
            scroll: ScrollHandle::new(),
            focus: None,
            go_to_line: None,
            line_count: 0,
            syntax: Vec::new(),
            blame: Vec::new(),
            tree,
            revision: 0,
            preview: true,
            local_controls: true,
            media: None,
        }
    }

    fn breadcrumbs(workspace: &str, path: &str) -> Vec<SharedString> {
        let root = workspace.trim_end_matches('/');
        let name = root.rsplit('/').next().unwrap_or(root);
        let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        let start = segments.iter().position(|part| *part == name).unwrap_or(0);
        segments[start..]
            .iter()
            .map(|part| (*part).to_owned().into())
            .collect()
    }

    pub fn name(&self) -> SharedString {
        self.crumbs.last().cloned().unwrap_or_default()
    }

    /// Hide controls that use the local workspace.
    pub fn set_local_controls(&mut self, visible: bool) {
        self.local_controls = visible;
    }

    pub fn is_preview(&self) -> bool {
        self.preview
    }

    fn pin_preview(&mut self, cx: &mut Context<Self>) {
        if self.preview {
            self.preview = false;
            cx.emit(FileTreeRequest::PinPreview);
            cx.notify();
        }
    }

    /// The tab strip icon: the Python sprite for `.py`, else the file change card's file glyph.
    pub fn tab_icon(&self) -> &'static str {
        if self.path.ends_with(".py") {
            "navigation-python"
        } else {
            "thread-file-diff"
        }
    }

    /// The host's answer; the cited line scrolls to the third row of the viewport.
    /// ponytail: a fixed top offset, not the original's centring, which was not measured.
    pub fn set_contents(&mut self, contents: Result<String, String>, cx: &mut Context<Self>) {
        self.syntax = contents
            .as_ref()
            .map_or_else(|_| Vec::new(), |text| syntax::full_file(&self.path, text));
        self.line_count = contents
            .as_ref()
            .map_or(0, |text| controls::lines(text).count() as u32);
        if self.line_count == 0 {
            self.go_to_line = None;
        }
        if let (Ok(_), Some(line)) = (&contents, self.line) {
            let top = line.saturating_sub(3) as f32 * FILE_LINE_HEIGHT;
            self.scroll.set_offset(gpui::point(px(0.), -px(top)));
        }
        self.contents = Some(contents);
        cx.notify();
    }

    fn header(&self, t: &Theme, clamped: bool, cx: &Context<Self>) -> Div {
        let last = self.crumbs.len().saturating_sub(1);
        let crumbs = self.crumbs.iter().enumerate().flat_map(|(i, crumb)| {
            let color = if i == last { t.text } else { t.text_secondary };
            (i > 0)
                .then(|| icon("thread-chevron-right", 14., t.text_tertiary).into_any_element())
                .into_iter()
                .chain(std::iter::once(
                    text(
                        crumb.clone(),
                        13.,
                        130. / 7.,
                        if i == last { 500. } else { 430. },
                        color,
                    )
                    .into_any_element(),
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
                        .gap(px(4.))
                        .px(px(8.))
                        .children(crumbs),
                ),
            )
            .when(self.local_controls, |d| {
                d.child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(
                            capsule(t, clamped)
                                .w(px(32.))
                                .id("file-search")
                                .role(gpui::Role::Button)
                                .aria_label("Search in files")
                                .focusable()
                                .tab_index(0)
                                .rounded_full()
                                .focus_visible(crate::composer::menu::trigger_focus)
                                .justify_center()
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.focus_file_search(window, cx)
                                }))
                                .on_key_down(cx.listener(
                                    |view, event: &KeyDownEvent, window, cx| {
                                        if matches!(event.keystroke.key.as_str(), "enter" | "space")
                                        {
                                            view.focus_file_search(window, cx);
                                            cx.stop_propagation();
                                        }
                                    },
                                ))
                                .child(icon("diff-jump-to-file", 16., t.text).runtime())
                                // The `backdrop-filter` button is its own cc layer: its icon
                                // rasterizes on that layer's tiles.
                                .layer(),
                        )
                        .child(self.file_tree_button(t, clamped, cx))
                        .child(
                            capsule(t, clamped).p_0().child(
                                div()
                                    .h(px(32.))
                                    .pl(px(9.))
                                    .flex()
                                    .items_center()
                                    .child(text("Open", 13., 18., 430., t.text))
                                    .child(
                                        div()
                                            .ml(px(4.))
                                            .w(px(23.))
                                            .h_full()
                                            .flex()
                                            .items_center()
                                            .pl(px(2.))
                                            .child(
                                                menu::backdrop_glyph(
                                                    include_bytes!(
                                                        "../../../assets/icons/chevron-down.svg"
                                                    ),
                                                    "navigation-open-chevron",
                                                    t.text,
                                                    128,
                                                )
                                                .size(px(14.)),
                                            ),
                                    ),
                            ),
                        ),
                )
            })
    }

    fn file_content(
        &self,
        t: &Theme,
        blame: Option<String>,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        match &self.contents {
            None => div().flex_1().into_any_element(),
            Some(Err(_)) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(text("Could not open file", 13., 20., 430., t.text_tertiary))
                .into_any_element(),
            Some(Ok(contents)) => {
                let lines: Vec<_> = controls::lines(contents).collect();
                let gutter = px(CH * (lines.len().to_string().len().max(1) + 3) as f32 + 2.);
                let foreground = rgb(if t.appearance == Appearance::Dark {
                    0xfcfcfc
                } else {
                    0x0d0d0d
                });
                let line_number = rgb(if t.appearance == Appearance::Dark {
                    0xa1a1a1
                } else {
                    0x585858
                });
                let cited = over(t.info_solid.opacity(0.12), t.surface);
                div()
                    .id("file-lines")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .font_family(MONO)
                    .text_size(px(CODE_SIZE))
                    .line_height(px(FILE_LINE_HEIGHT))
                    .font_weight(FontWeight(430.))
                    .text_color(foreground)
                    .pr(px(16.))
                    .children(lines.into_iter().enumerate().map(|(i, line)| {
                        let number = i as u32 + 1;
                        let active = Some(number) == self.line;
                        let bg = if active { cited } else { t.surface };
                        let text: SharedString = line.to_owned().into();
                        let content = StyledText::new(text).with_highlights(
                            self.syntax
                                .get(i)
                                .into_iter()
                                .flatten()
                                .map(|(range, token)| (range.clone(), token.style(t))),
                        );
                        div()
                            .id(i)
                            .h(px(FILE_LINE_HEIGHT))
                            .flex()
                            .bg(bg)
                            // The clicked line becomes the active line that blame describes.
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |view, _, _, cx| {
                                    view.line = Some(number);
                                    cx.notify();
                                }),
                            )
                            .child(
                                div()
                                    .w(gutter)
                                    .flex_none()
                                    .flex()
                                    .justify_end()
                                    .items_center()
                                    .pr(px(CH + 2.))
                                    .text_color(line_number)
                                    .child(number.to_string()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .pl(px(CH))
                                    .pr(px(CH))
                                    .whitespace_nowrap()
                                    .child(content)
                                    .when_some(blame.clone().filter(|_| active), |row, blame| {
                                        row.flex().child(
                                            div()
                                                .ml(px(CH))
                                                .text_color(t.text_tertiary)
                                                .child(blame),
                                        )
                                    }),
                            )
                    }))
                    .into_any_element()
            }
        }
    }
}

impl Render for FileView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        let blame = self.active_blame(cx);
        self.request_media(window, cx);
        let body = match self.media_view(window, cx) {
            Some(media) => media,
            None => self.file_content(&t, blame, cx),
        };
        div()
            .id("file-view")
            .relative()
            .track_focus(&focus)
            .key_context("SideTabs")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, window, cx| window.focus(&focus, cx)),
            )
            .on_click(cx.listener(|view, _, _, cx| view.pin_preview(cx)))
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
            // The panel's `contain: paint` clips the header capsules' shadows.
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .mt(px(48.))
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .child(body),
                    )
                    .when(self.tree.open, |row| row.child(self.file_tree_view(cx))),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .child(self.header(&t, panel_clamped(window, cx), cx)),
            )
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

pub(crate) fn stats_features(size: f32) -> FontFeatures {
    let mut features = vec![("tnum".into(), 1), ("cv01".into(), 1), ("cv02".into(), 1)];
    features.extend(textsys::letter_spacing(-0.025 * size).0.iter().cloned());
    FontFeatures(Arc::new(features))
}

/// `+N -M` in git colors, `size` px with `tracking-tight` (-0.025em letter spacing),
/// `tabular-nums` and `disambiguated-digits` (`cv01`, `cv02`).
pub fn stats(added: usize, removed: usize, size: f32, t: &Theme) -> Div {
    let features = stats_features(size);
    let tracked = |count, sign, color: Hsla| {
        text(states::signed_count(count, sign), size, size, 430., color)
            .font_features(features.clone())
    };
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(tracked(added, '+', t.git_added))
        .child(tracked(removed, '-', t.git_deleted))
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
                .child(controls::file_icon(&file.name, t))
                .child(
                    div()
                        .flex()
                        .overflow_hidden()
                        .child(text(
                            file.display_dir.clone(),
                            14.,
                            21.,
                            430.,
                            t.text_tertiary,
                        ))
                        .child(text(file.display_name.clone(), 14., 21., 430., t.text)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(stats(file.added, file.removed, 14., t).mx(px(4.)))
                .when(panel.viewed_identity(i).is_some(), |d| {
                    d.child(panel.viewed_button(i, t, cx))
                })
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
                        .id(("diff-open-tab", i))
                        .role(gpui::Role::Button)
                        .aria_label("Open in")
                        .focusable()
                        .tab_index(0)
                        .focus_visible(|s| s.border_2().border_color(rgb(0x3a83f7)))
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                this.request_open_tab(i, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.request_open_tab(i, cx);
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
        Kind::Context => {
            let [r, g, b] = if t.appearance == Appearance::Dark {
                [0.629856, 0.629859, 0.629858]
            } else {
                [0.344692, 0.344700, 0.344697]
            };
            (t.surface, t.surface, gpui::Rgba { r, g, b, a: 1. }.into())
        }
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

fn selected_line_colors(kind: Kind, t: &Theme) -> (Hsla, Hsla, Hsla) {
    let dark = t.appearance == Appearance::Dark;
    let (gutter, content) = match (dark, kind) {
        (true, Kind::Context) => (0x3a506b, 0x2e3a4a),
        (true, Kind::Added) => (0x36556a, 0x334e55),
        (true, Kind::Removed) => (0x4c4d64, 0x50404c),
        (false, Kind::Context) => (0xd5e6ff, 0xe1edff),
        (false, Kind::Added) => (0xc7e0f2, 0xcde4ec),
        (false, Kind::Removed) => (0xd5d8ef, 0xdfd9e8),
    };
    (
        rgb(gutter).into(),
        rgb(content).into(),
        rgb(if dark { 0x96c4ff } else { 0x20649e }).into(),
    )
}

fn range_mark(
    line: &Line,
    range: Range<usize>,
    layout: gpui::TextLayout,
    color: Hsla,
    radius: f32,
) -> gpui::AnyElement {
    let text = line.text.clone();
    gpui::canvas(
        |_, _, _| {},
        move |_, (), window, cx| {
            let Some(mut start) = layout.position_for_index(range.start) else {
                return;
            };
            let mut end = start;
            let decoration = Decoration {
                radius: Corners::all(px(radius)),
                fill: Some(color),
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
    .into_any_element()
}

fn change_marker(kind: Kind, split: bool, t: &Theme) -> Div {
    let color = if kind == Kind::Added {
        t.git_added
    } else {
        t.git_deleted
    };
    // The original's `contain: strict` marker ends two device px (at 2x) above the row box.
    let marker = div()
        .absolute()
        .left_0()
        .top_0()
        .bottom(px(1. / 64.))
        .w(px(4.));
    if kind != Kind::Removed || split {
        return marker.child(shapes::decoration(Decoration {
            fill: Some(color),
            ..Default::default()
        }));
    }
    // Keep the original Lab background mix unrounded for the gradient raster.
    let [r, g, b] = if t.appearance == Appearance::Dark {
        [0.232724, 0.120283, 0.103232]
    } else {
        [0.986266, 0.90291, 0.885396]
    };
    let colors = [gpui::Rgba { r, g, b, a: 1. }, color.into()];
    marker.child(
        gpui::canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| {
                let scale = window.scale_factor();
                let line = (CODE_LINE * scale * 64.).floor() / (scale * 64.);
                let period = px(line / (line / 2.).round());
                shapes::paint_horizontal_stripes(period, colors, bounds, window, cx);
            },
        )
        .absolute()
        .size_full(),
    )
}

fn hunk(
    h: &Hunk,
    location: (usize, usize),
    line_index: &Cell<usize>,
    panel: &DiffPanel,
    width: Pixels,
    window: &Window,
    cx: &Context<DiffPanel>,
) -> Div {
    let t = cx.theme();
    let split = panel.layout.split(&panel.files[location.0], width);
    let plain_text = panel.files[location.0].name.ends_with(".txt");
    let foreground: Hsla = rgb(match (t.appearance, plain_text) {
        (Appearance::Dark, false) => 0xfcfcfc,
        (Appearance::Dark, true) => 0xffffff,
        (_, false) => 0x0d0d0d,
        (_, true) => 0x1a1c1f,
    })
    .into();
    let digits = h
        .lines
        .iter()
        .map(|l| l.number)
        .max()
        .unwrap_or(0)
        .to_string()
        .len();
    let gutter = px(52.5625 + CH * digits.saturating_sub(4) as f32);
    let column_width = if split { (width - px(2.)) / 2. } else { width };
    let annotation_offset = px(f32::from(gutter).ceil()) - gutter;
    let annotation_width = px(f32::from(column_width - gutter).floor().max(0.));
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
               mark: Vec<gpui::AnyElement>,
               bar: Option<Kind>,
               utility: Option<gpui::AnyElement>| {
        div()
            .h(px(CODE_ROW))
            .when(panel.wrap, |d| d.h_auto().min_h(px(CODE_ROW)))
            .flex()
            .gap(px(2.))
            .child(
                deferred(
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
                        .when_some(bar, |d, kind| d.child(change_marker(kind, split, t)))
                        .text_color(number_fg)
                        .when_some(number, |d, n| d.child(n.to_string()))
                        .children(utility),
                )
                .with_priority(1)
                // Deferred only for paint order: scrolled-out rows stay inside the scroller.
                .clipped(),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .h_full()
                    .when(panel.wrap, |d| d.h_auto().min_h(px(CODE_ROW)).min_w_0())
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
    let render_line = |l: &Line, index: usize, number: u32, side: comments::Side| {
        let (mut number_bg, mut bg, mut number_fg) = line_colors(l.kind, t);
        let bar = (l.kind != Kind::Context).then_some(l.kind);
        let selected = panel.comment_selection_contains(location.0, side, number)
            || !split
                && l.kind == Kind::Context
                && panel.comment_selection_contains(location.0, comments::Side::Left, l.old_number);
        if selected {
            (number_bg, bg, number_fg) = selected_line_colors(l.kind, t);
        }
        let found = l.matches.iter().map(|range| {
            (
                range.clone(),
                HighlightStyle {
                    color: Some(t.text),
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
        let word = l
            .word_changes
            .iter()
            .filter(|_| panel.word_diffs)
            .map(|range| {
                (
                    range.clone(),
                    HighlightStyle {
                        color: Some(t.text),
                        ..Default::default()
                    },
                )
            });
        let content = StyledText::new(l.text.clone()).with_highlights(combine_highlights(
            combine_highlights(
                combine_highlights(
                    l.syntax
                        .iter()
                        .map(|(range, token)| (range.clone(), token.style(t))),
                    word,
                ),
                found,
            ),
            selection,
        ));
        let down_layout = content.layout().clone();
        panel.find_scroll.track(l, panel.active_match, &down_layout);
        let move_layout = down_layout.clone();
        let word_color = if l.kind == Kind::Added {
            t.git_added
        } else {
            t.git_deleted
        };
        let mark = l
            .word_changes
            .iter()
            .filter(|_| panel.word_diffs)
            .map(|range| {
                range_mark(
                    l,
                    range.clone(),
                    down_layout.clone(),
                    word_color.opacity(if t.appearance == Appearance::Dark {
                        0.2
                    } else {
                        0.15
                    }),
                    3.,
                )
            })
            .chain(l.matches.iter().enumerate().map(|(ordinal, range)| {
                let color = rgb(if l.match_start + ordinal == panel.active_match {
                    0xfb6a22
                } else {
                    0xffd240
                });
                range_mark(l, range.clone(), down_layout.clone(), color.into(), 2.5)
            }))
            .collect();
        let line = row(
            Some(number),
            (number_bg, number_fg),
            bg,
            content,
            mark,
            bar,
            panel.comment_gutter(location.0, side, number, cx),
        )
        .cursor_pointer()
        .group("diff-comment-line")
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                this.request_context_menu(
                    location.0,
                    Some((side, number)),
                    event.position,
                    window,
                    cx,
                );
            }),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                this.cancel_context_request();
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
        .into_any_element();
        let mut comments = panel.comment_rows(location.0, side, number, window, cx);
        if !split && l.kind == Kind::Context {
            comments.extend(panel.comment_rows(
                location.0,
                comments::Side::Left,
                l.old_number,
                window,
                cx,
            ));
        }
        let gutter_color = if selected {
            selected_line_colors(Kind::Context, t).0
        } else {
            t.surface
        };
        div()
            .flex()
            .flex_col()
            .child(line)
            .children(comments.into_iter().map(|(comment, background)| {
                div()
                    .flex()
                    .items_stretch()
                    .bg(background)
                    .child(
                        deferred(
                            div()
                                .w(gutter)
                                .flex_none()
                                .border_r_2()
                                .border_color(t.surface)
                                .bg(gutter_color),
                        )
                        .with_priority(1)
                        .clipped(),
                    )
                    .child(
                        deferred(
                            div()
                                .ml(annotation_offset)
                                .w(annotation_width)
                                .flex_none()
                                .child(comment),
                        )
                        .with_priority(0)
                        // Deferred only for paint order: the card stays inside the surface's
                        // `overflow: hidden` and the scroller, as in Chromium.
                        .clipped(),
                    )
            }))
    };
    div()
        .flex()
        .flex_col()
        .when(!panel.wrap, |d| d.min_w(minimum_width))
        .font_family(MONO)
        .font_weight(FontWeight(430.))
        .text_size(px(CODE_SIZE))
        .line_height(px(CODE_LINE))
        .text_color(foreground)
        .when_some(h.separator.clone(), |d, label| {
            d.child(panel.context_row(label, gutter, location, cx))
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
                                .min_h(px(CODE_ROW))
                                .children(old.map(|i| {
                                    render_line(
                                        &h.lines[i],
                                        base_index + i,
                                        h.lines[i].old_number,
                                        comments::Side::Left,
                                    )
                                })),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .min_h(px(CODE_ROW))
                                .children(new.map(|i| {
                                    render_line(
                                        &h.lines[i],
                                        base_index + i,
                                        h.lines[i].number,
                                        comments::Side::Right,
                                    )
                                })),
                        )
                }))
            } else {
                d.children(h.lines.iter().enumerate().map(|(i, line)| {
                    render_line(
                        line,
                        base_index + i,
                        line.number,
                        if line.kind == Kind::Removed {
                            comments::Side::Left
                        } else {
                            comments::Side::Right
                        },
                    )
                }))
            }
        })
        .map(|body| panel.track_hunk_layout(h, body, cx))
}

fn exceeds_diff_limits<'a>(lines: impl Iterator<Item = &'a str>) -> bool {
    let mut total = 0;
    for (index, line) in lines.enumerate() {
        let units = line.encode_utf16().count() + 1;
        if index >= MAX_DIFF_LINES || units > MAX_DIFF_LINE_UNITS {
            return true;
        }
        total += units;
        if total > MAX_DIFF_UNITS {
            return true;
        }
    }
    false
}

/// Hunks of a unified diff; file headers (`diff --git`, `---`, `+++`) before the first `@@`
/// are skipped.
fn parse(diff: &str, raw: Option<Kind>) -> Result<Vec<Hunk>, &'static str> {
    const TOO_LARGE: &str = "Diff too large to display";
    const INVALID: &str = "Diff failed to render";
    let mut changed_lines = 0;
    let mut changed_units = 0;
    let mut in_hunk = raw.is_some();
    let mut lines = diff.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let text = if raw.is_some() {
            line
        } else if line.starts_with("@@ ") {
            in_hunk = true;
            continue;
        } else if line.starts_with("diff --git ") {
            in_hunk = false;
            continue;
        } else if in_hunk {
            let Some(text) = line.strip_prefix('+').or_else(|| line.strip_prefix('-')) else {
                continue;
            };
            text
        } else {
            continue;
        };
        let missing_newline = raw.is_none()
            && text.ends_with('\n')
            && lines
                .peek()
                .is_some_and(|line| line.starts_with("\\ No newline at end of file"));
        let units = text.encode_utf16().count() - usize::from(missing_newline);
        changed_lines += 1;
        changed_units += units;
        if changed_lines > MAX_DIFF_LINES
            || changed_units > MAX_DIFF_UNITS
            || units > MAX_DIFF_LINE_UNITS
        {
            return Err(TOO_LARGE);
        }
    }
    if let Some(kind) = raw {
        return Ok(vec![Hunk {
            layout: Default::default(),
            old_start: if kind == Kind::Added { 0 } else { 1 },
            new_start: if kind == Kind::Removed { 0 } else { 1 },
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
                    word_changes: Vec::new(),
                    syntax: Vec::new(),
                })
                .collect(),
        }]);
    }
    let mut hunks: Vec<Hunk> = Vec::new();
    let (mut old, mut new): (u32, u32) = (1, 0);
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("@@ -") {
            let start = |s: &str| {
                let (line, count) = s.split_once(',').unwrap_or((s, "1"));
                let line = line.parse::<u32>().map_err(|_| INVALID)?;
                if count.parse::<u32>().map_err(|_| INVALID)? == 0 {
                    line.checked_add(1).ok_or(INVALID)
                } else {
                    Ok(line)
                }
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
                layout: Default::default(),
                old_start,
                new_start: new,
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
            word_changes: Vec::new(),
            syntax: Vec::new(),
        });
    }
    if changed_lines > 2_000 {
        return Ok(hunks);
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
                        let (a, b) = word::changes(&hunk.lines[old].text, &hunk.lines[index].text);
                        hunk.lines[old].word_changes = a;
                        hunk.lines[index].word_changes = b;
                    }
                }
                Kind::Context => removed = 0..0,
            }
        }
    }
    Ok(hunks)
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
    fn aggregate_changes_do_not_allow_staging() {
        assert!(!DiffSource::Uncommitted.mutable());
        assert!(DiffSource::Unstaged.mutable());
        assert!(DiffSource::Staged.mutable());
    }
    #[test]
    fn full_view_uses_workspace_width_for_auto_diff() {
        use gpui::{AppContext as _, HeadlessAppContext};
        let mut context = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            || Ok(None),
        );
        context.update(|cx| cx.set_global(Theme::dark()));
        let _window = context
            .open_window(gpui::size(px(1280.), px(832.)), |window, cx| {
                let empty: AnyView = cx.new(|_| crate::EmptyMain).into();
                let shell = cx.new(|_| Shell::new(empty.clone(), empty));
                let mut panel = DiffPanel::from_patch(
                    "/repo",
                    "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n",
                );
                panel.standalone = false;
                panel.layout = DiffLayout::Auto;
                panel.full_view = Some(shell.downgrade());
                assert!(
                    panel
                        .layout
                        .split(&panel.files[0], panel.available_width(window, cx))
                );
                panel.full_view = None;
                assert!(
                    !panel
                        .layout
                        .split(&panel.files[0], panel.available_width(window, cx))
                );
                shell
            })
            .unwrap();
    }

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
    fn view_changes_preserves_exact_path_identity_across_refresh() {
        use gpui::{AppContext as _, HeadlessAppContext};
        let start: ThreadStartResponse =
            fixture::response("sessions/subtract.jsonl", "thread/start", 0);
        let read: ThreadReadResponse = fixture::response("readback.jsonl", "thread/read", 1);
        let requested = read
            .thread
            .turns
            .iter()
            .flat_map(|turn| &turn.items)
            .find_map(|item| match item {
                ThreadItem::FileChange { changes, .. } => changes.first(),
                _ => None,
            })
            .unwrap()
            .path
            .clone();
        let mut app = HeadlessAppContext::with_platform(
            textsys::text_system(),
            Arc::new(theme::Assets),
            gpui_platform::current_headless_renderer,
        );
        app.update(|cx| {
            let panel =
                cx.new(|_| DiffPanel::last_turn(&start.cwd, &start.cwd, &read.thread.turns));
            panel.update(cx, |panel, cx| {
                panel.select_source(DiffSource::Uncommitted, cx);
                panel.query = Some("subtract".into());
                panel.filter.text = "unrelated".into();
                panel.view_changes(&requested, cx);
                assert_eq!(panel.source, DiffSource::LastTurn);
                assert_eq!(panel.query.as_deref(), Some(""));
                assert!(panel.filter.text.is_empty());
                assert!(panel.files[0].open);
                panel.apply(
                    &Event::Notification {
                        method: "item/completed".into(),
                        params: serde_json::json!({
                            "threadId": read.thread.id,
                            "turnId": read.thread.turns.last().unwrap().id,
                            "item": {"type": "fileChange"}
                        }),
                    },
                    &read.thread.cwd,
                    &read.thread.turns,
                    cx,
                );
                assert_ne!(panel.files[0].path_key, requested);
                assert!(!panel.files[0].open);
                assert_eq!((panel.files[0].added, panel.files[0].removed), (4, 0));
                assert_eq!(panel.files[0].hunks.len(), 1);
                let canonical = panel.files[0].path_key.clone();
                panel.view_changes(&canonical, cx);
                assert!(panel.files[0].open);
                panel.set_file_open(0, false);
                panel.replace_files(
                    DiffPanel::last_turn(&start.cwd, &read.thread.cwd, &read.thread.turns).files,
                );
                assert!(!panel.files[0].open);
                panel.set_all_files_open(true);
                panel.replace_files(
                    DiffPanel::last_turn(&start.cwd, &read.thread.cwd, &read.thread.turns).files,
                );
                assert!(panel.files[0].open);
            });
        });
    }

    #[test]
    fn replacement_lines_keep_their_word_spans() {
        let hunks = parse(
            "@@ -1,3 +1,3 @@\n-a = 1\n-b = 2\n+a = 3\n+b = 4\n same",
            None,
        )
        .unwrap();
        let lines = &hunks[0].lines;
        assert_eq!(
            lines
                .iter()
                .map(|line| line.word_changes.clone())
                .collect::<Vec<_>>(),
            vec![vec![4..5], vec![4..5], vec![4..5], vec![4..5], vec![]]
        );
    }
}
