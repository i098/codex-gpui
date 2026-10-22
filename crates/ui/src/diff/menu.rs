use super::*;
use gpui::{Anchor, Point, Role, anchored, point};
use std::path::{Component, Path, PathBuf};

const SOURCE_WIDTH: f32 = 200.;
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

pub(super) fn source_glyph(
    data: &'static [u8],
    id: &'static str,
    color: Hsla,
    opacity: u8,
) -> impl IntoElement + Styled {
    gpui::canvas(
        |_, _, _| {},
        move |bounds, (), window, cx| {
            let paint = |window: &mut Window| {
                if !shapes::paint_icon(data, id, bounds, false, color, None, window, cx) {
                    window
                        .paint_svg(
                            bounds,
                            id.into(),
                            Some(data),
                            gpui::TransformationMatrix::default(),
                            color,
                            cx,
                        )
                        .ok();
                }
            };
            if opacity == 255 {
                paint(window);
            } else {
                shapes::with_composited_opacity(opacity, bounds, window, cx, paint);
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
}

pub(super) struct Menu {
    file: usize,
    pub(super) kind: Kind,
    position: Point<Pixels>,
    active: Option<usize>,
    selected: String,
    return_focus: Option<FocusHandle>,
}

impl DiffPanel {
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
            Some(Kind::Options) => 4,
            Some(Kind::Branches) => repository.map_or(0, |r| r.branches.len()),
            Some(Kind::Commits) => repository.map_or(0, |r| r.commits.len()),
            Some(Kind::Header) => {
                if matches!(self.source, DiffSource::Unstaged | DiffSource::Staged) {
                    5
                } else {
                    3
                }
            }
            Some(Kind::Body) => 4,
        };
        (0..count).map(move |index| {
            let ready = !self.pending && self.error.is_none();
            match kind.unwrap() {
                Kind::Source => (SOURCES[index].label(), !self.pending),
                Kind::Options => match index {
                    0 => ("Expand all diffs", true),
                    1 => ("Collapse all diffs", true),
                    2 => ("Word wrap", true),
                    _ => (self.layout.label(), true),
                },
                Kind::Branches => (repository.unwrap().branches[index].as_str(), !self.pending),
                Kind::Commits => (
                    repository.unwrap().commits[index].label.as_str(),
                    !self.pending,
                ),
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
                Kind::Body => match index {
                    0 => (
                        "Copy selection",
                        !self.menu.as_ref().unwrap().selected.is_empty(),
                    ),
                    1 => ("Copy path", true),
                    2 => ("Copy relative path", true),
                    _ => ("Toggle line wrap", true),
                },
            }
        })
    }

    pub fn wraps_lines(&self) -> bool {
        self.wrap
    }

    pub(super) fn open_menu(
        &mut self,
        file: usize,
        kind: Kind,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = if kind == Kind::Source {
            point(
                window.viewport_size().width - panel_width(window, cx) + px(7.),
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
        });
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
        match event.keystroke.key.as_str() {
            "left" | "escape" if menu.kind == Kind::Commits => {
                let menu = self.menu.as_mut().unwrap();
                menu.kind = Kind::Source;
                menu.active = Some(4);
            }
            "right" if menu.kind == Kind::Source && menu.active == Some(4) => {
                self.activate_menu(4, window, cx);
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
            if !self.pending {
                cx.emit(DiffRequest::Refresh);
            }
        } else {
            self.select_source(source, cx);
            if source == DiffSource::Branch {
                self.open_menu(0, Kind::Branches, position, window, cx);
            }
        }
    }

    /// The directory a displayed file path is relative to.
    pub(super) fn file_root(&self) -> &str {
        if self.source == DiffSource::LastTurn {
            &self.workspace
        } else {
            self.repository
                .as_ref()
                .map_or(self.workspace.as_str(), |r| r.root.as_str())
        }
    }

    pub(super) fn request_open(&self, file: usize, cx: &mut Context<Self>) {
        let Some(file) = self.files.get(file) else {
            return;
        };
        let relative = format!("{}{}", file.dir, file.name);
        cx.emit(DiffRequest::Open(absolute_path(
            self.file_root(),
            &relative,
        )));
    }

    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.menu.take()
            && let Some(focus) = menu.return_focus
        {
            window.focus(&focus, cx);
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn activate_menu(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, true)) = self.menu_items().nth(index) else {
            return;
        };
        let Some(menu) = self.menu.take() else { return };
        if let Some(focus) = menu.return_focus {
            window.focus(&focus, cx);
        }
        if matches!(menu.kind, Kind::Branches | Kind::Commits) {
            if let Some(repository) = &self.repository {
                if menu.kind == Kind::Branches {
                    self.base_branch = repository.branches.get(index).cloned();
                } else {
                    self.commit = repository.commits.get(index).map(|c| c.id.clone());
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
        if menu.kind == Kind::Options {
            match index {
                0 => self.set_all_files_open(true),
                1 => self.set_all_files_open(false),
                2 => self.wrap = !self.wrap,
                _ => self.layout = self.layout.next(),
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
        let root = self.file_root().to_owned();
        let label = match (menu.kind, index) {
            (Kind::Header, 0) => "Copy path",
            (Kind::Header, 1) => "Open in editor",
            (Kind::Header, 2) => "Collapse file",
            (Kind::Header, 3) => {
                if self.source == DiffSource::Staged {
                    "Unstage file"
                } else {
                    "Stage file"
                }
            }
            (Kind::Header, _) => "Revert file",
            (_, 0) => "Copy selection",
            (_, 1) => "Copy path",
            (_, 2) => "Copy relative path",
            _ => "Toggle line wrap",
        };
        let file = &mut self.files[menu.file];
        match label {
            "Stage file" => cx.emit(DiffRequest::Stage(format!("{}{}", file.dir, file.name))),
            "Unstage file" => cx.emit(DiffRequest::Unstage(format!("{}{}", file.dir, file.name))),
            "Revert file" => self.request_revert(menu.file, None, cx),
            "Open in editor" => self.request_open(menu.file, cx),
            "Copy selection" => cx.write_to_clipboard(ClipboardItem::new_string(menu.selected)),
            "Copy path" | "Copy relative path" => {
                let relative = format!("{}{}", file.dir, file.name);
                let path = if menu.kind == Kind::Header || label == "Copy relative path" {
                    relative
                } else {
                    absolute_path(&root, &relative)
                };
                cx.write_to_clipboard(ClipboardItem::new_string(path));
            }
            "Collapse file" | "Expand file" => {
                let open = !file.open;
                self.set_file_open(menu.file, open);
                self.selection = None;
            }
            "Toggle line wrap" => self.wrap = !self.wrap,
            _ => unreachable!(),
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn popup(&self, kind: Kind, active: Option<usize>, cx: &Context<Self>) -> Stateful<Div> {
        let t = cx.theme();
        let source = kind == Kind::Source;
        let opaque = matches!(kind, Kind::Source | Kind::Branches | Kind::Commits);
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
                let row = div()
                    .id((row_id, i))
                    .role(Role::MenuItem)
                    .aria_label(label.to_owned())
                    .aria_disabled(!enabled)
                    .when(active == Some(i), |d| {
                        d.aria_active_descendant().bg(t.menu_item_hover)
                    })
                    .h(px(if opaque { ROW } else { 29. }))
                    .px(px(8.))
                    .rounded(px(if source { 15. } else { 12. }))
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
                                } else if let Some(menu) = &mut this.menu {
                                    menu.kind = kind;
                                    menu.active = Some(i);
                                    cx.notify();
                                }
                            },
                        ))
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if source && this.menu.as_ref().is_some_and(|m| m.kind != Kind::Source) {
                            let position = this.menu.as_ref().unwrap().position;
                            this.close_menu(window, cx);
                            this.choose_source(i, position, window, cx);
                        } else {
                            this.activate_menu(i, window, cx);
                        }
                    }))
                    .child(
                        text(
                            label.to_owned(),
                            13.,
                            if opaque { ROW - 10. } else { 20. },
                            if opaque { 430. } else { 400. },
                            if enabled { t.text } else { t.text_tertiary },
                        )
                        .flex_1(),
                    )
                    .when(source && SOURCES[i] == self.source, |d| {
                        d.child(source_glyph(SOURCE_CHECK, "diff-source-check", t.text, 191))
                    })
                    .when(source && i == 4, |d| {
                        d.child(source_glyph(
                            SOURCE_SUBMENU,
                            "diff-source-submenu",
                            t.text_tertiary,
                            255,
                        ))
                    });
                div()
                    .when(source && matches!(i, 1 | 4), |d| {
                        d.child(
                            div()
                                .h(px(9.))
                                .px(px(8.))
                                .py(px(4.))
                                .child(div().h(px(1.)).bg(t.border)),
                        )
                    })
                    .child(row)
            });
        let popup = if opaque {
            let width = if source { SOURCE_WIDTH } else { 320. };
            let surface = div()
                .relative()
                .m(px(1.))
                .w(px(width))
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
                        .children(rows),
                ));
            div()
                .ml(px(-20.))
                .mt(px(-12.))
                .pl(px(20.))
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
                _ => "File actions",
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
    }

    pub(super) fn menu_overlay(&self, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let menu = self.menu.as_ref().expect("open diff menu");
        let parent = menu.kind == Kind::Commits;
        let position = if parent {
            menu.position + point(px(SOURCE_WIDTH), px(4. + 4. * ROW + 18.))
        } else {
            menu.position
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
                    .when(parent, |d| {
                        d.child(
                            anchored()
                                .position(menu.position)
                                .anchor(Anchor::TopLeft)
                                .snap_to_window_with_margin(px(8.))
                                .child(self.popup(Kind::Source, Some(4), cx)),
                        )
                    })
                    .child(
                        anchored()
                            .position(position)
                            .anchor(if menu.kind == Kind::Header {
                                Anchor::TopRight
                            } else {
                                Anchor::TopLeft
                            })
                            .snap_to_window_with_margin(px(8.))
                            .child(self.popup(menu.kind, menu.active, cx)),
                    ),
            ),
        )
        .with_priority(4)
    }
}

pub(super) fn absolute_path(workspace: &str, relative: &str) -> String {
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
