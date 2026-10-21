use super::*;
use gpui::{Anchor, Point, Role, StyledImage, anchored, point};
use std::path::{Component, Path, PathBuf};

const SOURCE_WIDTH: f32 = 200.;
pub(super) const SHADOW_SIDE_PADDING: f32 = 20.;
const ROW: f32 = 3657. / 128.;
const SOURCES: [DiffSource; 6] = [
    DiffSource::LastTurn,
    DiffSource::Uncommitted,
    DiffSource::Unstaged,
    DiffSource::Staged,
    DiffSource::Commits,
    DiffSource::Branch,
];

const SOURCE_CHECK: &[u8] = br#"<svg width="16" height="16" viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg"><path d="M12.7216 2.99666C12.8863 2.75824 13.2125 2.69837 13.4511 2.86287C13.6895 3.0274 13.75 3.35377 13.5858 3.59236L7.20011 12.8472C6.97922 13.1685 6.52057 13.2111 6.24405 12.9361L2.45987 9.18123C2.25451 8.97704 2.25305 8.64475 2.45694 8.43904C2.66115 8.23343 2.99433 8.23205 3.20011 8.43611L6.62296 11.8326L12.7216 2.99666Z" fill="currentColor"/></svg>"#;
const SOURCE_SUBMENU: &[u8] = br#"<svg width="16" height="16" viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg"><path d="M5.96236 11.6289C5.75734 11.8339 5.75734 12.166 5.96236 12.3711C6.16739 12.5761 6.49953 12.5761 6.70455 12.3711L10.4692 8.60641C10.8039 8.27129 10.8039 7.72863 10.4692 7.39351L6.70455 3.62887C6.49952 3.42384 6.16739 3.42384 5.96236 3.62887C5.75734 3.83389 5.75734 4.16603 5.96236 4.37105L9.59127 7.99996L5.96236 11.6289Z" fill="currentColor"/></svg>"#;

#[derive(Clone, Copy)]
enum GlyphOpacity {
    Svg(u8),
    Compositor(u8),
}

fn source_glyph(
    data: impl Into<std::borrow::Cow<'static, [u8]>>,
    id: &'static str,
    color: Hsla,
    opacity: GlyphOpacity,
    compositor_opacity: u8,
) -> impl IntoElement {
    let data = data.into();
    gpui::canvas(
        |_, _, _| {},
        move |bounds, (), window, cx| {
            let paint = |window: &mut Window| {
                let origin = matches!(opacity, GlyphOpacity::Svg(alpha) if alpha != 255)
                    .then_some(bounds.origin);
                if !shapes::paint_icon(data.as_ref(), id, bounds, false, color, origin, window, cx)
                {
                    window
                        .paint_svg(
                            bounds,
                            id.into(),
                            Some(data.as_ref()),
                            gpui::TransformationMatrix::default(),
                            color,
                            cx,
                        )
                        .ok();
                }
            };
            let svg = |window: &mut Window| match opacity {
                GlyphOpacity::Svg(255) | GlyphOpacity::Compositor(255) => paint(window),
                GlyphOpacity::Svg(alpha) => {
                    let ink = shapes::icon_paint_bounds(data.as_ref(), id, bounds, false, window)
                        .unwrap_or(bounds);
                    shapes::with_layer(alpha, ink, window, paint);
                }
                GlyphOpacity::Compositor(alpha) => {
                    shapes::with_composited_opacity(alpha, bounds, window, cx, paint);
                }
            };
            if compositor_opacity == 255 {
                svg(window);
            } else {
                shapes::with_composited_opacity(compositor_opacity, bounds, window, cx, svg);
            }
        },
    )
    .size(px(16.))
    .flex_none()
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Header,
    Body,
    Source,
    Options,
    Branches,
    Commits,
    Repositories,
    OpenWith,
}

pub(super) struct Menu {
    file: usize,
    pub(super) kind: Kind,
    position: Point<Pixels>,
    active: Option<usize>,
    pub(super) selected: String,
    return_focus: Option<FocusHandle>,
    query: controls::TextField,
    choices: Vec<(usize, i32)>,
    parent: Option<Kind>,
    options_start: usize,
}

impl DiffPanel {
    pub(super) fn refresh_picker(&mut self) {
        let Some(menu) = &mut self.menu else { return };
        menu.choices.clear();
        let Some(repository) = &self.repository else {
            return;
        };
        match menu.kind {
            Kind::Branches => {
                let query = menu.query.text.trim();
                if query.is_empty() {
                    menu.choices
                        .extend((0..repository.branches.len()).map(|index| (index, 0)));
                } else {
                    let pattern = crate::screens::sidebar::MatchPattern::new(query);
                    menu.choices
                        .extend(repository.branches.iter().enumerate().filter_map(
                            |(index, name)| {
                                let score = pattern.score(name);
                                (score > 0).then_some((index, score))
                            },
                        ));
                    menu.choices
                        .sort_unstable_by_key(|(index, score)| (std::cmp::Reverse(*score), *index));
                }
            }
            Kind::Commits => menu
                .choices
                .extend((0..repository.commits.len()).map(|index| (index, 0))),
            _ => {}
        }
    }

    fn picker_status(&self, kind: Kind) -> Option<&'static str> {
        if !matches!(kind, Kind::Branches | Kind::Commits) {
            return None;
        }
        let branches = kind == Kind::Branches;
        if self.pending {
            return Some(if branches {
                "Loading branches…"
            } else {
                "Loading commits…"
            });
        }
        if self.error.is_some() {
            return Some(if branches {
                "Unable to load branches"
            } else {
                "Unable to load commits"
            });
        }
        if self
            .menu
            .as_ref()
            .is_none_or(|menu| menu.choices.is_empty())
        {
            return Some(if branches {
                if self
                    .menu
                    .as_ref()
                    .is_some_and(|menu| !menu.query.text.is_empty())
                {
                    "No matching branches"
                } else {
                    "No branches"
                }
            } else {
                "No commits on branch"
            });
        }
        None
    }

    pub(super) fn paste_picker(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        if let Some(menu) = &mut self.menu
            && menu.kind == Kind::Branches
        {
            menu.query.paste(text);
            menu.active = None;
            self.refresh_picker();
            cx.notify();
            return true;
        }
        false
    }

    pub(super) fn select_picker_text(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(menu) = &mut self.menu
            && menu.kind == Kind::Branches
        {
            menu.query.selected = true;
            cx.notify();
            return true;
        }
        false
    }
    fn picker_selected(&self, kind: Kind, index: usize) -> bool {
        if kind == Kind::Repositories {
            return self.source != DiffSource::LastTurn
                && self.repositories.get(index).is_some_and(|repository| {
                    self.selected_repository
                        .as_deref()
                        .or_else(|| self.repository.as_ref().map(|r| r.root.as_str()))
                        == Some(repository.root.as_str())
                });
        }
        if self.picker_status(kind).is_some() {
            return false;
        }
        let Some(index) = self
            .menu
            .as_ref()
            .and_then(|menu| menu.choices.get(index))
            .map(|choice| choice.0)
        else {
            return false;
        };
        self.repository
            .as_ref()
            .is_some_and(|repository| match kind {
                Kind::Branches => repository.branches.get(index) == self.base_branch.as_ref(),
                Kind::Commits => repository
                    .commits
                    .get(index)
                    .is_some_and(|commit| Some(&commit.id) == self.commit.as_ref()),
                _ => false,
            })
    }

    pub fn menu_items(&self) -> impl ExactSizeIterator<Item = (&str, bool)> + Clone {
        self.menu_items_for(self.menu.as_ref().map(|m| m.kind))
    }

    fn menu_items_for(
        &self,
        kind: Option<Kind>,
    ) -> impl ExactSizeIterator<Item = (&str, bool)> + Clone {
        let repository = self.repository.as_ref();
        let count = match kind {
            None => 0,
            Some(Kind::Source) => SOURCES.len(),
            Some(Kind::Options) => 10 - self.menu.as_ref().map_or(0, |menu| menu.options_start),
            Some(kind @ (Kind::Branches | Kind::Commits)) => {
                if self.picker_status(kind).is_some() {
                    if self.error.is_some() && !self.pending {
                        2
                    } else {
                        1
                    }
                } else {
                    self.menu.as_ref().map_or(0, |menu| menu.choices.len())
                }
            }
            Some(Kind::Repositories) => self.repositories.len(),
            Some(Kind::Header) => {
                if matches!(self.source, DiffSource::Unstaged | DiffSource::Staged) {
                    5
                } else {
                    3
                }
            }
            Some(Kind::Body) => self.context.items.len(),
            Some(Kind::OpenWith) => self.context.target_count(),
        };
        (0..count).map(move |index| {
            let ready = !self.pending && self.error.is_none();
            match kind.unwrap() {
                Kind::Source => (SOURCES[index].label(), !self.pending),
                Kind::Options => {
                    match index + self.menu.as_ref().map_or(0, |menu| menu.options_start) {
                        0 => ("Refresh", !self.pending),
                        1 => ("Word wrap", true),
                        2 => (self.layout.label(), true),
                        3 => (
                            if self.all_expanded() {
                                "Collapse all diffs"
                            } else {
                                "Expand all diffs"
                            },
                            true,
                        ),
                        4 => ("Load full files", true),
                        5 => ("Rich preview", true),
                        6 => ("Word diffs", true),
                        7 => ("Hide white space", true),
                        8 => ("Hide imports", true),
                        _ => ("Copy git apply command", self.can_copy_patch()),
                    }
                }
                Kind::Branches | Kind::Commits => {
                    if let Some(status) = self.picker_status(kind.unwrap()) {
                        if index == 1 {
                            ("Retry", true)
                        } else {
                            (status, false)
                        }
                    } else {
                        let selected = self.menu.as_ref().unwrap().choices[index].0;
                        if kind == Some(Kind::Branches) {
                            (repository.unwrap().branches[selected].as_str(), true)
                        } else {
                            (repository.unwrap().commits[selected].label.as_str(), true)
                        }
                    }
                }
                Kind::Repositories => (self.repositories[index].name.as_str(), !self.pending),
                Kind::Header => match index {
                    0 => ("Copy path", true),
                    1 => ("Open in editor", true),
                    2 => (
                        if self.files[self.menu.as_ref().unwrap().file].open {
                            "Collapse file"
                        } else {
                            "Expand file"
                        },
                        true,
                    ),
                    3 => (
                        if self.source == DiffSource::Staged {
                            "Unstage file"
                        } else {
                            "Stage file"
                        },
                        ready,
                    ),
                    _ => ("Revert file", ready),
                },
                Kind::Body => {
                    let item = &self.context.items[index];
                    (item.label.as_ref(), item.enabled)
                }
                Kind::OpenWith => (self.context.target_label(index), true),
            }
        })
    }

    pub fn wraps_lines(&self) -> bool {
        self.wrap
    }

    pub fn shows_word_diffs(&self) -> bool {
        self.word_diffs
    }

    pub(super) fn toggle_all_files(&mut self, cx: &mut Context<Self>) {
        let open = !self.all_expanded();
        self.files.iter_mut().for_each(|file| file.open = open);
        self.selection = None;
        cx.notify();
    }

    pub(super) fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        self.wrap = !self.wrap;
        cx.emit(DiffRequest::Preference(
            "host.wrapCodeDiff.2",
            self.wrap.into(),
        ));
    }

    pub(super) fn open_menu(
        &mut self,
        file: usize,
        kind: Kind,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_context_request();
        let position = if matches!(kind, Kind::Source | Kind::Repositories) {
            let extra = if kind == Kind::Source {
                self.repository_label().map_or(px(0.), |label| {
                    let line = window.text_system().layout_line(
                        label,
                        px(13.),
                        &[TextRun {
                            len: label.len(),
                            font: Font {
                                weight: FontWeight(430.),
                                ..font(SANS)
                            },
                            color: cx.theme().text,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        }],
                        None,
                    );
                    (line.width + px(32.)).min(px(230.))
                })
            } else {
                px(0.)
            };
            point(
                window.viewport_size().width - panel_width(window, cx) + px(7.) + extra,
                px(83.),
            )
        } else {
            position
        };
        self.selecting = false;
        let mut selected = self.selected_text();
        // The original native menu trims selection; keyboard Copy preserves it.
        selected.truncate(selected.trim_end().len());
        selected.drain(..selected.len() - selected.trim_start().len());
        self.menu = Some(Menu {
            file,
            kind,
            position,
            active: None,
            selected,
            return_focus: window.focused(cx),
            query: controls::TextField::default(),
            choices: Vec::new(),
            parent: None,
            options_start: if (if self.standalone {
                window.viewport_size().width
            } else {
                panel_width(window, cx) - px(1.)
            }) >= px(625.)
            {
                4
            } else if self.source == DiffSource::Cloud {
                1
            } else {
                0
            },
        });
        self.refresh_picker();
        if let Some(focus) = &self.focus {
            window.focus(focus, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = &self.menu else { return };
        if menu.kind == Kind::Branches
            && !matches!(
                event.keystroke.key.as_str(),
                "escape" | "enter" | "up" | "down" | "home" | "end" | "tab"
            )
        {
            if self.menu.as_mut().unwrap().query.key(event) {
                self.refresh_picker();
                self.menu.as_mut().unwrap().active = None;
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }
        match event.keystroke.key.as_str() {
            "left" | "escape" if menu.parent.is_some() => {
                let parent = menu.parent.unwrap();
                let active = if parent == Kind::Source {
                    Some(4)
                } else {
                    self.context.open_with_index()
                };
                let menu = self.menu.as_mut().unwrap();
                menu.kind = parent;
                menu.parent = None;
                menu.active = active;
            }
            "right" if menu.kind == Kind::Source && menu.active == Some(4) => {
                self.activate_menu(4, window, cx);
            }
            "right"
                if menu.kind == Kind::Body
                    && menu.active.is_some()
                    && menu.active == self.context.open_with_index() =>
            {
                self.open_targets_menu(cx);
            }
            "escape" => self.close_menu(window, cx),
            "enter" | "space" => {
                if let Some(index) = menu.active {
                    self.activate_menu(index, window, cx);
                }
            }
            "down" | "up" | "home" | "end" => {
                let forward = matches!(event.keystroke.key.as_str(), "down" | "home");
                let mut index = match event.keystroke.key.as_str() {
                    "home" | "end" => None,
                    _ => menu.active,
                };
                let count = self.menu_items().len();
                for _ in 0..count {
                    let next = match index {
                        None => {
                            if forward {
                                0
                            } else {
                                count - 1
                            }
                        }
                        Some(i) => {
                            if forward {
                                (i + 1) % count
                            } else {
                                (i + count - 1) % count
                            }
                        }
                    };
                    index = Some(next);
                    if self.menu_items().nth(next).is_some_and(|item| item.1) {
                        self.menu.as_mut().unwrap().active = index;
                        break;
                    }
                }
            }
            "tab" => {}
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn choose_source(
        &mut self,
        index: usize,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = SOURCES[index];
        if source == DiffSource::Commits {
            self.open_menu(0, Kind::Commits, position, window, cx);
            self.menu.as_mut().unwrap().parent = Some(Kind::Source);
            if !self.pending {
                cx.emit(DiffRequest::Refresh);
            }
        } else {
            self.select_source(source, cx);
        }
    }

    /// The directory a displayed file path is relative to.
    pub(super) fn file_root(&self) -> &str {
        if self.source == DiffSource::LastTurn {
            if self.aggregated_turn.is_some() {
                &self.cwd
            } else {
                &self.workspace
            }
        } else {
            self.repository.as_ref().map_or(
                self.selected_repository
                    .as_deref()
                    .unwrap_or(self.workspace.as_str()),
                |r| r.root.as_str(),
            )
        }
    }

    pub(super) fn file_path(&self, index: usize) -> Option<String> {
        let file = self.files.get(index)?;
        Some(absolute_path(
            self.file_root(),
            &format!("{}{}", file.dir, file.name),
        ))
    }

    pub(super) fn request_open(&self, file: usize, cx: &mut Context<Self>) {
        if let Some(path) = self.file_path(file) {
            cx.emit(DiffRequest::Open(path));
        }
    }

    pub(super) fn request_open_tab(&self, file: usize, cx: &mut Context<Self>) {
        if let Some(path) = self.file_path(file) {
            cx.emit(DiffRequest::OpenTab { path, line: None });
        }
    }

    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_context_request();
        if let Some(menu) = self.menu.take()
            && let Some(focus) = menu.return_focus
        {
            window.focus(&focus, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn open_targets_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.menu {
            menu.kind = Kind::OpenWith;
            menu.parent = Some(Kind::Body);
            menu.active = None;
            cx.notify();
        }
    }

    fn activate_menu(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .menu
            .as_ref()
            .is_some_and(|menu| matches!(menu.kind, Kind::Branches | Kind::Commits))
            && self.error.is_some()
            && !self.pending
            && index == 1
        {
            cx.emit(DiffRequest::Refresh);
            return;
        }
        let Some((_, true)) = self.menu_items().nth(index) else {
            return;
        };
        if self
            .menu
            .as_ref()
            .is_some_and(|menu| menu.kind == Kind::Body)
            && self.context.items[index].action == context::Action::OpenWith
        {
            self.open_targets_menu(cx);
            cx.stop_propagation();
            return;
        }
        let Some(menu) = self.menu.take() else { return };
        if let Some(focus) = menu.return_focus {
            window.focus(&focus, cx);
        }
        if menu.kind == Kind::Body {
            self.activate_context(index, menu.selected, window, cx);
            return;
        }
        if menu.kind == Kind::OpenWith {
            self.activate_target(index, cx);
            return;
        }
        if matches!(menu.kind, Kind::Branches | Kind::Commits) {
            if let Some(repository) = &self.repository {
                if menu.kind == Kind::Branches {
                    self.base_branch = repository.branches.get(menu.choices[index].0).cloned();
                } else {
                    self.commit = repository
                        .commits
                        .get(menu.choices[index].0)
                        .map(|c| c.id.clone());
                }
                self.select_source(
                    if menu.kind == Kind::Branches {
                        DiffSource::Branch
                    } else {
                        DiffSource::Commits
                    },
                    cx,
                );
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if menu.kind == Kind::Repositories {
            self.select_repository(index, cx);
            return;
        }
        if menu.kind == Kind::Options {
            match index + menu.options_start {
                0 => cx.emit(DiffRequest::Refresh),
                1 => self.toggle_wrap(cx),
                2 => self.set_layout(self.layout.next(), cx),
                3 => self.toggle_all_files(cx),
                4 => self.toggle_full_files(cx),
                5 => self.toggle_rich_preview(cx),
                6 => {
                    self.word_diffs = !self.word_diffs;
                    cx.emit(DiffRequest::Preference(
                        "host.wordDiffsEnabled.2",
                        self.word_diffs.into(),
                    ));
                }
                7 => self.toggle_whitespace(cx),
                8 => self.toggle_imports(cx),
                _ => cx.emit(DiffRequest::CopyGitApply),
            }
            self.selection = None;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if menu.kind == Kind::Source {
            self.choose_source(index, menu.position, window, cx);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let file = &mut self.files[menu.file];
        match index {
            0 => cx.write_to_clipboard(ClipboardItem::new_string(format!(
                "{}{}",
                file.dir, file.name
            ))),
            1 => self.request_open(menu.file, cx),
            2 => {
                file.open = !file.open;
                self.selection = None;
            }
            3 if self.source == DiffSource::Staged => {
                cx.emit(DiffRequest::Unstage(format!("{}{}", file.dir, file.name)))
            }
            3 => cx.emit(DiffRequest::Stage(format!("{}{}", file.dir, file.name))),
            _ => self.request_revert(menu.file, None, cx),
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn options_popup(&self, window: &Window, cx: &Context<Self>) -> Stateful<Div> {
        self.popup(
            Kind::Options,
            self.menu.as_ref().and_then(|menu| menu.active),
            window,
            cx,
        )
    }

    fn popup(
        &self,
        kind: Kind,
        active: Option<usize>,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let t = cx.theme();
        let source = kind == Kind::Source;
        let options = kind == Kind::Options;
        let option_start = self.menu.as_ref().map_or(0, |menu| menu.options_start);
        let filter = (kind == Kind::Branches).then(|| {
            let query = &self.menu.as_ref().unwrap().query;
            div()
                .id("diff-branch-filter")
                .role(Role::TextInput)
                .aria_label("Search branches")
                .aria_placeholder("Search branches")
                .aria_value(query.text.clone())
                .h(px(32.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon("diff-jump-to-file", 14., t.text_tertiary))
                .child(
                    text(
                        if query.text.is_empty() {
                            "Search branches"
                        } else {
                            &query.text
                        },
                        13.,
                        20.,
                        430.,
                        if query.text.is_empty() {
                            t.text_tertiary
                        } else {
                            t.text
                        },
                    )
                    .min_w_0()
                    .truncate(),
                )
                .into_any_element()
        });
        let opaque = matches!(
            kind,
            Kind::Source | Kind::Options | Kind::Branches | Kind::Commits | Kind::Repositories
        );
        let popup_id = if kind == Kind::Commits {
            "diff-commits-menu"
        } else {
            "diff-menu"
        };
        let row_id = if kind == Kind::Commits {
            "diff-commits-item"
        } else {
            "diff-menu-item"
        };
        let rows = self
            .menu_items_for(Some(kind))
            .enumerate()
            .map(|(i, (label, enabled))| {
                let option = options.then_some(i + option_start);
                let checked = option.and_then(|index| self.option_checked(index));
                let context_icon = match kind {
                    Kind::Body => self.context.items[i].icon.clone(),
                    Kind::OpenWith => self.context.target_icon(i),
                    _ => None,
                };
                let row = div()
                    .id((row_id, i))
                    .role(Role::MenuItem)
                    .aria_label(label.to_owned())
                    .when_some(checked, |d, checked| {
                        d.role(Role::MenuItemCheckBox).aria_toggled(if checked {
                            gpui::Toggled::True
                        } else {
                            gpui::Toggled::False
                        })
                    })
                    .aria_disabled(!enabled)
                    .when(active == Some(i), |d| {
                        d.aria_active_descendant().bg(t.menu_item_hover)
                    })
                    .h(px(if opaque { ROW } else { 29. }))
                    .px(px(8.))
                    .rounded(px(if source || options { 15. } else { 12. }))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(enabled, |d| {
                        d.cursor_pointer().on_hover(cx.listener(
                            move |this, hovered: &bool, window, cx| {
                                if !*hovered {
                                    return;
                                }
                                if source && i == 4 {
                                    if let Some(position) =
                                        this.menu.as_ref().map(|menu| menu.position)
                                    {
                                        this.choose_source(i, position, window, cx);
                                    }
                                } else if kind == Kind::Body
                                    && this.context.open_with_index() == Some(i)
                                {
                                    this.open_targets_menu(cx);
                                } else if let Some(menu) = &mut this.menu {
                                    menu.kind = kind;
                                    menu.parent = None;
                                    menu.active = Some(i);
                                    cx.notify();
                                }
                            },
                        ))
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if kind == Kind::Body
                            && this
                                .menu
                                .as_ref()
                                .is_some_and(|menu| menu.kind == Kind::OpenWith)
                        {
                            if let Some(menu) = &mut this.menu {
                                menu.kind = Kind::Body;
                                menu.parent = None;
                            }
                            this.activate_menu(i, window, cx);
                        } else if source
                            && this.menu.as_ref().is_some_and(|m| m.kind != Kind::Source)
                        {
                            let position = this.menu.as_ref().unwrap().position;
                            this.close_menu(window, cx);
                            this.choose_source(i, position, window, cx);
                        } else {
                            this.activate_menu(i, window, cx);
                        }
                    }))
                    .when_some(option, |d, index| {
                        let name = self.option_icon(index);
                        let data = cx
                            .asset_source()
                            .load(&format!("icons/{name}.svg"))
                            .expect("read bundled option icon")
                            .expect("bundled option icon exists");
                        let opacity = if matches!(index, 0 | 3 | 9) && active != Some(i) {
                            191
                        } else {
                            255
                        };
                        d.child(source_glyph(
                            data,
                            name,
                            t.text,
                            if index == 0 {
                                GlyphOpacity::Compositor(opacity)
                            } else {
                                GlyphOpacity::Svg(opacity)
                            },
                            if enabled { 255 } else { 128 },
                        ))
                    })
                    .when_some(context_icon, |d, image| {
                        d.child(
                            gpui::img(image)
                                .size(px(16.))
                                .object_fit(gpui::ObjectFit::Contain)
                                .flex_none(),
                        )
                    })
                    .child(
                        text(
                            label.to_owned(),
                            13.,
                            if opaque { ROW - 10. } else { 20. },
                            if opaque { 430. } else { 400. },
                            if enabled || options {
                                t.text
                            } else {
                                t.text_tertiary
                            },
                        )
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .when(options && !enabled, |d| d.opacity(0.5)),
                    )
                    .when(source && SOURCES[i] == self.source, |d| {
                        d.child(source_glyph(
                            SOURCE_CHECK,
                            "diff-source-check",
                            t.text,
                            GlyphOpacity::Svg(191),
                            255,
                        ))
                    })
                    .when(checked.is_some(), |d| {
                        d.child(
                            div()
                                .size(px(16.))
                                .flex_none()
                                .children(
                                    checked
                                        .filter(|checked| *checked)
                                        .map(|_| icon("diff-option-check", 16., t.text)),
                                )
                                .when(!enabled, |d| d.opacity(0.5)),
                        )
                    })
                    .when(
                        matches!(kind, Kind::Branches | Kind::Commits | Kind::Repositories)
                            && self.picker_selected(kind, i),
                        |d| {
                            d.child(source_glyph(
                                SOURCE_CHECK,
                                "diff-picker-check",
                                t.text,
                                GlyphOpacity::Svg(191),
                                255,
                            ))
                        },
                    )
                    .when(
                        source && i == 4
                            || kind == Kind::Body && self.context.open_with_index() == Some(i),
                        |d| {
                            d.child(source_glyph(
                                SOURCE_SUBMENU,
                                "diff-source-submenu",
                                t.text_tertiary,
                                GlyphOpacity::Svg(255),
                                255,
                            ))
                        },
                    );
                div()
                    .when(
                        source && matches!(i, 1 | 4)
                            || options && i > 0 && option == Some(4)
                            || kind == Kind::Body && self.context.items[i].separator,
                        |d| {
                            d.child(
                                div()
                                    .h(px(9.))
                                    .px(px(8.))
                                    .py(px(4.))
                                    .child(div().h(px(1.)).bg(t.border)),
                            )
                        },
                    )
                    .child(row)
            });
        let popup = if opaque {
            let width = if source {
                px(SOURCE_WIDTH)
            } else if options {
                self.menu_items_for(Some(kind))
                    .enumerate()
                    .fold(px(160.), |width, (index, (label, _))| {
                        let line = window.text_system().layout_line(
                            label,
                            px(13.),
                            &[TextRun {
                                len: label.len(),
                                font: Font {
                                    weight: FontWeight(430.),
                                    ..font(SANS)
                                },
                                color: t.text,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            }],
                            None,
                        );
                        width.max(
                            line.width
                                + px(46.
                                    + if self.option_checked(index + option_start).is_some() {
                                        22.
                                    } else {
                                        0.
                                    }),
                        )
                    })
                    .min(px(320.))
            } else {
                px(320.)
            };
            let surface = div()
                .relative()
                .m(px(1.))
                .w(width)
                .child(shapes::decoration(Decoration {
                    radius: Corners::all(px(20.)),
                    corner: shapes::Corner::Squircle,
                    fill: Some(t.menu_background),
                    shadows: vec![
                        shadow(t.border, 0., 0., 0.5, false),
                        shadow(gpui::rgba(0x0000001f).into(), 8., 16., -4., false),
                    ],
                    ..Default::default()
                }))
                .child(crate::rounded_clip(
                    px(20.),
                    shapes::Corner::Squircle,
                    div()
                        .id(if kind == Kind::Commits {
                            "diff-commits-scroll"
                        } else {
                            "diff-menu-scroll"
                        })
                        .max_h(px(360.))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .p(px(4.))
                        .children(filter)
                        .children(rows),
                ));
            div()
                .ml(px(-SHADOW_SIDE_PADDING))
                .mt(px(-12.))
                .pl(px(SHADOW_SIDE_PADDING))
                .pt(px(12.))
                .layer()
                .child(surface)
        } else {
            crate::composer::popover(t, 220., |d| d.children(rows))
                .ml_0()
                .pl_0()
        };
        popup
            .id(popup_id)
            .role(Role::Menu)
            .aria_label(match kind {
                Kind::Source => "Changes source",
                Kind::Options => "Changes options",
                Kind::Branches => "Select branch",
                Kind::Commits => "Select commit",
                Kind::Repositories => "Select repository",
                _ => "File actions",
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
    }

    pub(super) fn menu_overlay(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let menu = self.menu.as_ref().expect("open diff menu");
        let parent = match (menu.kind, menu.parent) {
            (Kind::Commits, Some(Kind::Source)) => Some((Kind::Source, 4)),
            (Kind::OpenWith, Some(Kind::Body)) => self
                .context
                .open_with_index()
                .map(|index| (Kind::Body, index)),
            _ => None,
        };
        let position = match parent {
            Some((Kind::Source, _)) => {
                menu.position + point(px(SOURCE_WIDTH), px(4. + 4. * ROW + 18.))
            }
            Some((_, index)) => menu.position + point(px(220.), px(4. + index as f32 * 29.)),
            None => menu.position,
        };
        deferred(
            anchored().position(point(px(0.), px(0.))).child(
                div()
                    .w(window.viewport_size().width)
                    .h(window.viewport_size().height)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.close_menu(window, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, window, cx| this.close_menu(window, cx)),
                    )
                    .when_some(parent, |d, (kind, index)| {
                        d.child(
                            anchored()
                                .position(menu.position)
                                .anchor(Anchor::TopLeft)
                                .snap_to_window_with_margin(px(8.))
                                .child(self.popup(kind, Some(index), window, cx)),
                        )
                    })
                    .when(menu.kind != Kind::Options, |d| {
                        d.child(
                            anchored()
                                .position(position)
                                .anchor(if menu.kind == Kind::Header {
                                    Anchor::TopRight
                                } else {
                                    Anchor::TopLeft
                                })
                                .snap_to_window_with_margin(px(8.))
                                .child(self.popup(menu.kind, menu.active, window, cx)),
                        )
                    }),
            ),
        )
        .with_priority(4)
    }
}

fn absolute_path(workspace: &str, relative: &str) -> String {
    let mut path = PathBuf::new();
    for component in Path::new(workspace).join(relative).components() {
        match component {
            Component::ParentDir => {
                path.pop();
            }
            Component::CurDir => {}
            other => path.push(other.as_os_str()),
        }
    }
    path.to_string_lossy().into_owned()
}
