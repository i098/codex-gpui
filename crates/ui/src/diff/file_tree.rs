use super::*;
use crate::composer::Composer;
use gpui::{Focusable as _, ScrollStrategy, Task, UniformListScrollHandle, uniform_list};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub enum FileTreeRequest {
    Directory(String),
    Search { query: String, generation: u64 },
    Open(String),
    PinPreview,
}

impl gpui::EventEmitter<FileTreeRequest> for FileView {}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum EntryKind {
    Directory,
    File,
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    path: String,
    #[serde(rename = "type")]
    kind: EntryKind,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listing {
    directory_path: String,
    entries: Vec<Entry>,
}

#[derive(Clone)]
struct Row {
    name: SharedString,
    match_name: SharedString,
    path: SharedString,
    kind: EntryKind,
    depth: usize,
}

impl Row {
    fn new(name: String, path: String, kind: EntryKind) -> Self {
        Self {
            match_name: name.to_lowercase().into(),
            name: name.into(),
            path: path.into(),
            kind,
            depth: 0,
        }
    }
}

struct Directory {
    entries: Vec<Row>,
    loaded: Instant,
}

fn relative_path(root: &str, path: &str) -> Option<SharedString> {
    let relative = Path::new(path).strip_prefix(root).ok()?.to_str()?;
    (!relative.is_empty()).then(|| relative.to_owned().into())
}

pub(super) struct WorkspaceTree {
    pub(super) open: bool,
    root: String,
    directories: BTreeMap<String, Directory>,
    pending: BTreeSet<String>,
    expanded: BTreeSet<String>,
    search_directories: BTreeMap<String, Directory>,
    search_expanded: BTreeSet<String>,
    rows: Vec<Row>,
    selected: Option<SharedString>,
    search_selected: Option<SharedString>,
    hovered: Option<SharedString>,
    input: Option<Entity<Composer>>,
    focus: Option<FocusHandle>,
    query: String,
    searching: bool,
    search_task: Option<Task<()>>,
    search_generation: u64,
    directory_errors: BTreeMap<String, SharedString>,
    search_error: Option<SharedString>,
    scroll: UniformListScrollHandle,
    typeahead: String,
    typed_at: Option<Instant>,
}

impl WorkspaceTree {
    pub(super) fn new(root: &str, path: &str) -> Self {
        Self {
            open: false,
            root: Path::new(root)
                .components()
                .as_path()
                .to_string_lossy()
                .into_owned(),
            directories: BTreeMap::new(),
            pending: BTreeSet::new(),
            expanded: BTreeSet::new(),
            search_directories: BTreeMap::new(),
            search_expanded: BTreeSet::new(),
            rows: Vec::new(),
            selected: relative_path(root, path),
            search_selected: None,
            hovered: None,
            input: None,
            focus: None,
            query: String::new(),
            searching: false,
            search_task: None,
            search_generation: 0,
            directory_errors: BTreeMap::new(),
            search_error: None,
            scroll: UniformListScrollHandle::new(),
            typeahead: String::new(),
            typed_at: None,
        }
    }

    fn collect_rows(&self, path: &str, depth: usize, rows: &mut Vec<Row>) {
        let (directories, expanded) = if self.query.trim().is_empty() {
            (&self.directories, &self.expanded)
        } else {
            (&self.search_directories, &self.search_expanded)
        };
        let Some(directory) = directories.get(path) else {
            return;
        };
        for entry in &directory.entries {
            let mut row = entry.clone();
            row.depth = depth;
            rows.push(row);
            if entry.kind == EntryKind::Directory && expanded.contains(entry.path.as_ref()) {
                self.collect_rows(entry.path.as_ref(), depth + 1, rows);
            }
        }
    }

    fn refresh_rows(&mut self) {
        if self.query.trim().is_empty() {
            self.search_selected = None;
        }
        let mut rows = std::mem::take(&mut self.rows);
        rows.clear();
        self.collect_rows("", 0, &mut rows);
        self.rows = rows;
    }

    fn expanded(&self, path: &str) -> bool {
        if self.query.trim().is_empty() {
            self.expanded.contains(path)
        } else {
            self.search_expanded.contains(path)
        }
    }

    fn selected(&self) -> Option<&SharedString> {
        if self.query.trim().is_empty() {
            self.selected.as_ref()
        } else {
            self.search_selected.as_ref()
        }
    }

    fn select(&mut self, path: Option<SharedString>) {
        if self.query.trim().is_empty() {
            self.selected = path;
        } else {
            self.search_selected = path;
        }
    }

    fn error(&self) -> Option<&SharedString> {
        if self.query.trim().is_empty() {
            self.directory_errors.get("")
        } else {
            self.search_error.as_ref()
        }
    }
}

impl FileView {
    pub fn workspace_root(&self) -> &str {
        &self.tree.root
    }

    pub fn file_tree_open(&self) -> bool {
        self.tree.open
    }

    pub fn visible_file_paths(&self) -> impl Iterator<Item = &str> {
        self.tree.rows.iter().map(|row| row.path.as_ref())
    }

    pub fn file_revision(&self) -> u64 {
        self.revision
    }

    pub fn navigate_file(&mut self, path: String, line: Option<u32>, cx: &mut Context<Self>) {
        self.revision = self.revision.wrapping_add(1);
        self.crumbs = Self::breadcrumbs(&self.tree.root, &path);
        self.tree.select(relative_path(&self.tree.root, &path));
        self.path = path;
        self.line = line;
        self.contents = None;
        self.go_to_line = None;
        self.line_count = 0;
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }

    pub fn set_file_contents(
        &mut self,
        revision: u64,
        contents: Result<String, String>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.revision != revision {
            return false;
        }
        self.set_contents(contents, cx);
        true
    }

    pub(super) fn file_tree_button(
        &self,
        theme: &Theme,
        clamped: bool,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let (surface, color) = if self.tree.open {
            let accent = Rgba::from(theme.accent);
            let base = Rgba::from(theme.surface);
            let fill = Rgba {
                r: base.r * 0.9 + accent.r * 0.1,
                g: base.g * 0.9 + accent.g * 0.1,
                b: base.b * 0.9 + accent.b * 0.1,
                a: 1.,
            }
            .into();
            let (color, shadows) = match theme.appearance {
                Appearance::Dark => (
                    Rgba {
                        r: accent.r * 0.61 + 0.39,
                        g: accent.g * 0.61 + 0.39,
                        b: accent.b * 0.61 + 0.39,
                        a: 1.,
                    }
                    .into(),
                    vec![
                        shadow(theme.border, 0., 0., 0.5, false),
                        shadow(theme.border, 0., 0., 0.5, false),
                        shadow(black().opacity(0.12), 8., 16., -4., false),
                    ],
                ),
                Appearance::Light => (
                    theme.text_accent,
                    vec![
                        shadow(theme.text.opacity(0.047), 0., 0., 1., false),
                        shadow(black().opacity(0.05), 4., 16., 0., false),
                    ],
                ),
            };
            (
                div().relative().size(px(32.)).child(shapes::backdrop_blur(
                    px(24.),
                    shapes::Fill::Color,
                    clamped,
                    Decoration {
                        radius: Corners::all(px(16.)),
                        fill: Some(fill),
                        shadows,
                        ..Default::default()
                    },
                )),
                color,
            )
        } else {
            (capsule(theme, clamped).w(px(32.)), theme.text)
        };
        surface
            .id("file-tree")
            .role(gpui::Role::Button)
            .aria_label("Toggle file tree")
            .aria_toggled(if self.tree.open {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .focusable()
            .tab_index(0)
            .rounded_full()
            .focus_visible(crate::composer::menu::trigger_focus)
            .flex()
            .items_center()
            .justify_center()
            .on_click(cx.listener(|view, _, window, cx| view.toggle_file_tree(window, cx)))
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    view.toggle_file_tree(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(icon("navigation-files", 18., color))
    }

    pub fn focus_file_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tree.open {
            self.toggle_file_tree(window, cx);
        }
        if let Some(input) = &self.tree.input {
            window.focus(&input.read(cx).focus_handle(cx), cx);
        }
    }

    fn clear_file_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = &self.tree.input {
            input.update(cx, |input, cx| input.set_text("", cx));
        }
        self.focus_file_search(window, cx);
    }

    pub fn focus_file_tree(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.open
            && let Some(focus) = &self.tree.focus
        {
            window.focus(focus, cx);
        }
    }

    pub fn toggle_file_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pin_preview(cx);
        self.tree.open = !self.tree.open;
        if self.tree.open {
            if self.tree.input.is_none() {
                let input = cx
                    .new(|cx| Composer::embedded_field("Filter files…", "", px(14.), px(18.), cx));
                cx.observe(&input, |view, input, cx| {
                    let query = input.read(cx).field_text();
                    if query != view.tree.query {
                        view.tree.query = query.to_owned();
                        view.search_tree(cx);
                    }
                })
                .detach();
                self.tree.input = Some(input);
                self.tree.focus = Some(cx.focus_handle());
            }
            self.request_tree_directory("", cx);
            self.tree.refresh_rows();
            if !self.tree.query.trim().is_empty() {
                self.search_tree(cx);
            }
        } else {
            self.tree.search_task = None;
            self.tree.search_generation = self.tree.search_generation.wrapping_add(1);
            self.tree.searching = false;
            if self
                .tree
                .focus
                .as_ref()
                .is_some_and(|focus| focus.contains_focused(window, cx))
                && let Some(focus) = &self.focus
            {
                window.focus(focus, cx);
            }
        }
        cx.notify();
    }

    fn request_tree_directory(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.tree.pending.contains(path)
            || self
                .tree
                .directories
                .get(path)
                .is_some_and(|directory| directory.loaded.elapsed() < Duration::from_secs(5))
        {
            return;
        }
        self.tree.pending.insert(path.to_owned());
        self.tree.directory_errors.remove(path);
        cx.emit(FileTreeRequest::Directory(path.to_owned()));
    }

    pub fn set_tree_directory(
        &mut self,
        path: &str,
        result: Result<Value, String>,
        cx: &mut Context<Self>,
    ) {
        self.tree.pending.remove(path);
        let listing = result.and_then(|value| {
            serde_json::from_value::<Listing>(value).map_err(|error| error.to_string())
        });
        match listing {
            Ok(listing) if listing.directory_path == path => {
                self.tree.directories.insert(
                    path.to_owned(),
                    Directory {
                        entries: listing
                            .entries
                            .into_iter()
                            .map(|entry| Row::new(entry.name, entry.path, entry.kind))
                            .collect(),
                        loaded: Instant::now(),
                    },
                );
                self.tree.directory_errors.remove(path);
                self.tree.refresh_rows();
            }
            Ok(_) => {
                self.tree.directory_errors.insert(
                    path.to_owned(),
                    "The directory response did not match the requested folder".into(),
                );
            }
            Err(error) => {
                self.tree
                    .directory_errors
                    .insert(path.to_owned(), error.into());
            }
        }
        cx.notify();
    }

    fn search_tree(&mut self, cx: &mut Context<Self>) {
        self.tree.search_task = None;
        self.tree.search_generation = self.tree.search_generation.wrapping_add(1);
        self.tree.search_error = None;
        self.tree.searching = !self.tree.query.trim().is_empty();
        if !self.tree.searching {
            self.tree.refresh_rows();
            cx.notify();
            return;
        }
        let query = self.tree.query.clone();
        let generation = self.tree.search_generation;
        self.tree.search_task = Some(cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            view.update(cx, |view, cx| {
                if view.tree.open && view.tree.query == query {
                    cx.emit(FileTreeRequest::Search { query, generation });
                }
            })
            .ok();
        }));
        cx.notify();
    }

    pub fn set_tree_search(
        &mut self,
        generation: u64,
        query: &str,
        result: Result<Vec<String>, String>,
        cx: &mut Context<Self>,
    ) {
        if self.tree.search_generation != generation || self.tree.query != query || !self.tree.open
        {
            return;
        }
        self.tree.searching = false;
        self.tree.search_directories.clear();
        self.tree.search_expanded.clear();
        match result {
            Ok(paths) => {
                let now = Instant::now();
                let mut seen = BTreeSet::<SharedString>::new();
                for path in paths {
                    let parts: Vec<_> = path.split('/').filter(|part| !part.is_empty()).collect();
                    let mut parent = String::new();
                    for (index, part) in parts.iter().enumerate() {
                        let entry_path = if parent.is_empty() {
                            (*part).to_owned()
                        } else {
                            format!("{parent}/{part}")
                        };
                        let kind = if index + 1 == parts.len() {
                            EntryKind::File
                        } else {
                            EntryKind::Directory
                        };
                        let directory =
                            self.tree
                                .search_directories
                                .entry(parent)
                                .or_insert_with(|| Directory {
                                    entries: Vec::new(),
                                    loaded: now,
                                });
                        if !seen.contains(entry_path.as_str()) {
                            let row = Row::new((*part).to_owned(), entry_path.clone(), kind);
                            seen.insert(row.path.clone());
                            directory.entries.push(row);
                        }
                        if kind == EntryKind::Directory {
                            self.tree.search_expanded.insert(entry_path.clone());
                        }
                        parent = entry_path;
                    }
                }
                for directory in self.tree.search_directories.values_mut() {
                    directory.entries.sort_by(|a, b| {
                        (a.kind == EntryKind::File)
                            .cmp(&(b.kind == EntryKind::File))
                            .then_with(|| a.name.cmp(&b.name))
                    });
                }
                self.tree.search_error = None;
            }
            Err(error) => self.tree.search_error = Some(error.into()),
        }
        self.tree.refresh_rows();
        cx.notify();
    }

    fn activate_tree_path(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.searching || self.tree.error().is_some() {
            return;
        }
        let Some(row) = self
            .tree
            .rows
            .iter()
            .find(|row| row.path.as_ref() == path)
            .cloned()
        else {
            return;
        };
        self.tree.select(Some(row.path.clone()));
        self.focus_file_tree(window, cx);
        if row.kind == EntryKind::Directory {
            let expanded = if self.tree.query.trim().is_empty() {
                &mut self.tree.expanded
            } else {
                &mut self.tree.search_expanded
            };
            if !expanded.remove(path) {
                expanded.insert(path.to_owned());
            }
            if self.tree.query.trim().is_empty() && self.tree.expanded.contains(path) {
                self.request_tree_directory(path, cx);
            }
            self.tree.refresh_rows();
        } else {
            cx.emit(FileTreeRequest::Open(path.to_owned()));
        }
        cx.notify();
    }

    fn tree_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .tree
            .focus
            .as_ref()
            .is_some_and(|focus| focus.is_focused(window))
            || self.tree.rows.is_empty()
            || self.go_to_line.is_some()
            || self.tree.searching
            || self.tree.error().is_some()
        {
            return;
        }
        let Some(index) = self.tree_key_index(event, window, cx) else {
            return;
        };
        self.tree.select(Some(self.tree.rows[index].path.clone()));
        self.tree
            .scroll
            .scroll_to_item(index, ScrollStrategy::Center);
        if self.tree.rows[index].kind == EntryKind::File {
            cx.emit(FileTreeRequest::Open(
                self.tree.rows[index].path.to_string(),
            ));
        }
        cx.notify();
        cx.stop_propagation();
    }

    fn tree_key_index(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let selected = self
            .tree
            .selected()
            .and_then(|path| self.tree.rows.iter().position(|row| &row.path == path));
        let current = selected.unwrap_or(0);
        Some(match event.keystroke.key.as_str() {
            "up" => current.saturating_sub(1),
            "down" => selected.map_or(0, |index| (index + 1).min(self.tree.rows.len() - 1)),
            "home" => 0,
            "end" => self.tree.rows.len() - 1,
            "enter" | "space" => {
                let path = self.tree.rows[current].path.clone();
                self.activate_tree_path(path.as_ref(), window, cx);
                cx.stop_propagation();
                return None;
            }
            "left" | "right" => return self.tree_horizontal(event, current, window, cx),
            _ => return self.tree_typeahead(event, cx),
        })
    }

    fn tree_horizontal(
        &mut self,
        event: &KeyDownEvent,
        current: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let row = &self.tree.rows[current];
        let right = event.keystroke.key == "right";
        if row.kind == EntryKind::Directory && right != self.tree.expanded(row.path.as_ref()) {
            let path = row.path.clone();
            self.activate_tree_path(path.as_ref(), window, cx);
            cx.stop_propagation();
            return None;
        }
        if right {
            return if row.kind == EntryKind::Directory {
                Some((current + 1).min(self.tree.rows.len() - 1))
            } else {
                self.tree_typeahead(event, cx)
            };
        }
        let parent = row.path.rsplit_once('/').map(|(parent, _)| parent);
        let index = parent.and_then(|parent| {
            self.tree
                .rows
                .iter()
                .position(|row| row.path.as_ref() == parent)
        });
        if index.is_none() {
            cx.stop_propagation();
        }
        index
    }

    fn tree_typeahead(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> Option<usize> {
        let character = event.keystroke.key_char.as_deref().filter(|text| {
            !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.platform
                && text.chars().all(|c| !c.is_control())
        })?;
        if self
            .tree
            .typed_at
            .is_none_or(|time| time.elapsed() > Duration::from_secs(1))
        {
            self.tree.typeahead.clear();
        }
        self.tree.typeahead.push_str(character);
        self.tree.typed_at = Some(Instant::now());
        let query = self.tree.typeahead.to_lowercase();
        let index = self
            .tree
            .rows
            .iter()
            .position(|row| row.match_name.starts_with(&query));
        if index.is_none() {
            cx.stop_propagation();
        }
        index
    }

    fn tree_icon(&self, row: &Row, theme: &Theme, selected: bool) -> gpui::AnyElement {
        let (name, color) = match row.kind {
            EntryKind::Directory => {
                let color = if selected {
                    theme.text
                } else {
                    rgb(0x84848a).into()
                };
                return Self::chevron(self.tree.expanded(row.path.as_ref()), color);
            }
            EntryKind::File if row.name.ends_with(".py") => (
                "diff-file-python",
                rgb(if theme.appearance == Appearance::Dark {
                    0x69b1ff
                } else {
                    0x1a85d4
                })
                .into(),
            ),
            EntryKind::File => ("thread-file-diff", theme.text_tertiary),
        };
        icon(name, 16., color).runtime().into_any_element()
    }

    /// The original nudges its one chevron: `translate(0, 1px)` open, and closed
    /// `rotate(-90deg) translate(-0.5px, 0.5px)`, which lands 0.5px right and down.
    fn chevron(expanded: bool, color: Hsla) -> gpui::AnyElement {
        if expanded {
            return icon("navigation-chevron-down", 12., color)
                .runtime()
                .relative()
                .top(px(1.))
                .into_any_element();
        }
        // Chromium's inexact `rotate(-90deg)` turns the svg box clip into an anti-aliased
        // path clip, which changes AAA's fringe px; a hairline radius gives Skia that clip.
        crate::rounded_clip(
            px(0.01),
            shapes::Corner::Round,
            icon("navigation-chevron-right", 12., color)
                .runtime()
                .relative()
                .left(px(0.5))
                .top(px(0.5)),
        )
        .into_any_element()
    }

    fn tree_rows(
        &self,
        range: Range<usize>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<Stateful<Div>> {
        let theme = cx.theme();
        let active_fill = match theme.appearance {
            Appearance::Dark => gpui::white().opacity(0.05),
            Appearance::Light => theme.button_secondary,
        };
        let focused = self
            .tree
            .focus
            .as_ref()
            .is_some_and(|focus| focus.is_focused(window));
        range
            .map(|index| {
                let row = &self.tree.rows[index];
                let path = row.path.clone();
                let directory = row.kind == EntryKind::Directory;
                let selected = self.tree.selected() == Some(&row.path);
                let hovered_path = row.path.clone();
                let fill = if selected {
                    active_fill
                } else if self.tree.hovered.as_ref() == Some(&row.path) {
                    theme.list_hover
                } else {
                    theme.surface
                };
                div()
                    .id(row.path.clone())
                    .role(gpui::Role::TreeItem)
                    .aria_label(row.name.clone())
                    .aria_level(row.depth + 1)
                    .aria_selected(selected)
                    .when(directory, |row_element| {
                        row_element.aria_expanded(self.tree.expanded(row.path.as_ref()))
                    })
                    .h(px(28.))
                    .w_full()
                    .relative()
                    .px(px(3.))
                    .pl(px(3. + row.depth as f32 * 12.5))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .overflow_hidden()
                    .child(shapes::decoration(Decoration {
                        radius: Corners::all(px(6.)),
                        fill: Some(fill),
                        ..Default::default()
                    }))
                    // The original's focus outline is on a `::before` box over the row fill.
                    .when(selected && focused, |row_element| {
                        row_element.child(shapes::decoration(Decoration {
                            radius: Corners::all(px(6.)),
                            border: Some((px(1.), rgb(0x009fff).into())),
                            ..Default::default()
                        }))
                    })
                    .children((0..row.depth).map(|depth| {
                        div()
                            .absolute()
                            .left(px(10.25 + depth as f32 * 12.5))
                            .top_0()
                            .h_full()
                            .w(px(1.))
                            // The original's guide: `transform: translateX(-0.25px)`.
                            .child(shapes::translated_decoration(
                                Decoration {
                                    fill: Some(rgb(0x84848a).opacity(0.1875).into()),
                                    ..Default::default()
                                },
                                gpui::point(px(-0.25), px(0.)),
                            ))
                    }))
                    .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                        view.tree.hovered = hovered.then(|| hovered_path.clone());
                        cx.notify();
                    }))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.activate_tree_path(path.as_ref(), window, cx)
                    }))
                    .child(
                        div()
                            .w(px(16.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(self.tree_icon(row, theme, selected)),
                    )
                    .child(text(row.name.clone(), 13., 28., 400., theme.text))
            })
            .collect()
    }

    pub(super) fn file_tree_view(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let mut content = div().flex_1().min_h_0();
        let error = self.tree.error();
        if let Some(error) = error {
            content = content.child(div().px(px(12.)).py(px(8.)).child(text(
                error.clone(),
                12.,
                18.,
                430.,
                theme.text_error,
            )));
        } else if self.tree.searching {
            content = content.child(div().p(px(8.)).child(text(
                "Searching files…",
                14.,
                20.,
                430.,
                theme.text_tertiary,
            )));
        } else if !self.tree.directories.contains_key("") && self.tree.query.trim().is_empty() {
            content = content.child(div().p(px(8.)).child(text(
                "Loading directory entries…",
                14.,
                20.,
                430.,
                theme.text_tertiary,
            )));
        } else if self.tree.rows.is_empty() {
            content = content.child(div().p(px(8.)).child(text(
                if self.tree.query.trim().is_empty() {
                    "No files in this folder"
                } else {
                    "No matching files"
                },
                14.,
                20.,
                430.,
                theme.text_tertiary,
            )));
        } else {
            let view = cx.entity();
            content = content.px(px(4.)).child(
                uniform_list(
                    "workspace-file-tree-rows",
                    self.tree.rows.len(),
                    move |range, window, cx| {
                        view.update(cx, |view, cx| view.tree_rows(range, window, cx))
                    },
                )
                .size_full()
                .max_h(px(self.tree.rows.len() as f32 * 28.))
                // Match the original virtual rows' isolated transform layer.
                .layer()
                .track_scroll(&self.tree.scroll),
            );
        }
        div()
            .w(px(250.))
            .max_w(gpui::relative(0.6))
            .h_full()
            .flex_none()
            .border_l_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("workspace-file-tree")
                    .role(gpui::Role::Tree)
                    .aria_label("Workspace files")
                    .track_focus(self.tree.focus.as_ref().expect("open tree focus"))
                    .key_context("FileTree")
                    .size_full()
                    .flex()
                    .flex_col()
                    .on_key_down(cx.listener(Self::tree_key))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, window, cx| {
                            view.focus_file_tree(window, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div().px(px(4.)).pt(px(8.)).pb(px(1.)).flex_none().child(
                            div()
                                .relative()
                                .h(px(28.))
                                .p(px(1.))
                                .child(shapes::decoration(Decoration {
                                    radius: Corners::all(px(12.5)),
                                    corner: shapes::Corner::Squircle,
                                    fill: Some(theme.primary_soft_alpha),
                                    border: Some((px(1.), theme.border)),
                                    ..Default::default()
                                }))
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|view, _, window, cx| {
                                        view.focus_file_search(window, cx);
                                        cx.stop_propagation();
                                    }),
                                )
                                .child(div().ml(px(8.)).flex_none().child(
                                    icon("navigation-search", 16., theme.text_tertiary).runtime(),
                                ))
                                .child(div().flex_1().min_w_0().pr(px(6.)).child(
                                    self.tree.input.as_ref().expect("open tree input").clone(),
                                ))
                                .when(!self.tree.query.is_empty(), |field| {
                                    field.child(
                                        div()
                                            .id("clear-file-filter")
                                            .role(gpui::Role::Button)
                                            .aria_label("Clear file filter")
                                            .focusable()
                                            .tab_index(0)
                                            .focus_visible(crate::composer::menu::trigger_focus)
                                            .size(px(24.))
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(10.))
                                            .hover(|button| button.bg(theme.list_hover))
                                            .on_click(cx.listener(|view, _, window, cx| {
                                                view.clear_file_filter(window, cx);
                                            }))
                                            .on_key_down(cx.listener(
                                                |view, event: &KeyDownEvent, window, cx| {
                                                    if matches!(
                                                        event.keystroke.key.as_str(),
                                                        "enter" | "space"
                                                    ) {
                                                        view.clear_file_filter(window, cx);
                                                        cx.stop_propagation();
                                                    }
                                                },
                                            ))
                                            .child(
                                                icon("diff-tab-close", 14., theme.text_tertiary)
                                                    .runtime(),
                                            ),
                                    )
                                }),
                        ),
                    )
                    .child(content),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> gpui::HeadlessAppContext {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        gpui::HeadlessAppContext::new(textsys::text_system())
    }

    #[test]
    fn searching_preserves_browse_selection_without_selecting_search_results() {
        let mut tree = WorkspaceTree::new("/workspace", "/workspace/original.rs");
        tree.select(Some("src".into()));
        tree.query = ".rs".into();
        assert!(tree.selected().is_none());
        tree.select(Some("src/other.rs".into()));
        assert_eq!(
            tree.selected().map(|path| path.as_ref()),
            Some("src/other.rs")
        );
        tree.query.clear();
        tree.refresh_rows();
        assert_eq!(tree.selected().map(|path| path.as_ref()), Some("src"));
        tree.query = ".rs".into();
        assert!(tree.selected().is_none());
    }

    #[test]
    fn unix_relative_paths_preserve_literal_backslashes() {
        assert_eq!(
            relative_path("/workspace", r"/workspace/back\slash/note.txt").as_deref(),
            Some(r"back\slash/note.txt")
        );
    }

    #[test]
    fn failed_search_clears_previous_visible_results() {
        let mut cx = context();
        let file = cx.new(|_| FileView::new("/workspace", "/workspace/a.rs".into(), None));
        file.update(&mut cx, |file, cx| {
            file.tree.open = true;
            file.tree.query = "a".into();
            file.tree.search_generation = 1;
            file.set_tree_search(1, "a", Ok(vec!["a.rs".into()]), cx);
            file.tree.query = "b".into();
            file.tree.search_generation = 2;
            file.set_tree_search(2, "b", Err("search failed".into()), cx);
            assert!(file.visible_file_paths().next().is_none());
            assert_eq!(
                file.tree.error().map(|error| error.as_ref()),
                Some("search failed")
            );
        });
    }

    #[test]
    fn child_directory_failure_preserves_the_workspace_tree() {
        let mut cx = context();
        let file = cx.new(|_| FileView::new("/workspace", "/workspace/safe.rs".into(), None));
        file.update(&mut cx, |file, cx| {
            file.set_tree_directory(
                "",
                Ok(serde_json::json!({
                    "directoryPath": "",
                    "entries": [
                        { "name": "gone", "path": "gone", "type": "directory" },
                        { "name": "safe.rs", "path": "safe.rs", "type": "file" }
                    ]
                })),
                cx,
            );
            file.tree.expanded.insert("gone".into());
            file.set_tree_directory("gone", Err("folder disappeared".into()), cx);
            assert_eq!(
                file.visible_file_paths().collect::<Vec<_>>(),
                ["gone", "safe.rs"]
            );
            assert!(file.tree.error().is_none());
        });
    }

    #[test]
    fn stale_file_reads_do_not_replace_the_current_file() {
        let mut cx = context();
        let file = cx.new(|_| FileView::new("/workspace", "/workspace/a.rs".into(), None));
        file.update(&mut cx, |file, cx| {
            let stale = file.file_revision();
            file.navigate_file("/workspace/b.rs".into(), None, cx);
            assert!(file.set_file_contents(file.file_revision(), Ok("current file".into()), cx));
            assert!(!file.set_file_contents(stale, Ok("stale file".into()), cx));
            assert_eq!(file.path, "/workspace/b.rs");
            assert_eq!(
                file.contents.as_ref().unwrap().as_ref().unwrap(),
                "current file"
            );
        });
    }

    #[test]
    fn outdated_search_results_do_not_replace_current_results() {
        let mut cx = context();
        let file = cx.new(|_| FileView::new("/workspace", "/workspace/a.rs".into(), None));
        file.update(&mut cx, |file, cx| {
            file.tree.open = true;
            file.tree.query = "f".into();
            file.tree.search_generation = 2;
            file.set_tree_search(2, "f", Ok(vec!["fresh.rs".into()]), cx);
            file.set_tree_search(1, "f", Ok(vec!["former.rs".into()]), cx);
            assert_eq!(file.tree.rows.len(), 1);
            assert_eq!(file.tree.rows[0].path.as_ref(), "fresh.rs");
            file.tree.open = false;
            file.set_tree_search(2, "f", Ok(vec!["hidden.rs".into()]), cx);
            assert_eq!(file.tree.rows[0].path.as_ref(), "fresh.rs");
        });
    }
}
