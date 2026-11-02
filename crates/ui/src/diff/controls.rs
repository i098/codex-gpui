use super::*;
use std::borrow::Cow;

pub(super) fn file_icon(file_name: &str, t: &Theme) -> crate::icon::Icon {
    let extension = std::path::Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    let dark = t.appearance == Appearance::Dark;
    let (name, color) = if extension.eq_ignore_ascii_case("py") {
        ("diff-file-python", if dark { 0x69b1ff } else { 0x1a85d4 })
    } else if [
        "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tif", "tiff",
    ]
    .iter()
    .any(|suffix| extension.eq_ignore_ascii_case(suffix))
    {
        ("diff-file-image", if dark { 0xff678d } else { 0xd32a61 })
    } else if extension.eq_ignore_ascii_case("txt") {
        ("diff-file-text", if dark { 0xadadb1 } else { 0x84848a })
    } else {
        ("diff-file-default", if dark { 0xadadb1 } else { 0x84848a })
    };
    icon(name, 16., rgb(color).into())
}

fn header_text(text: &str) -> Cow<'_, str> {
    let mut space = true;
    let changed = text.bytes().any(|byte| {
        let needs_change = matches!(byte, b'\t' | b'\n' | b'\r' | 12) || (byte == b' ' && space);
        space = byte == b' ';
        needs_change
    }) || text.ends_with(' ');
    if !changed {
        return Cow::Borrowed(text);
    }
    let mut display = String::with_capacity(text.len());
    for word in text
        .split([' ', '\t', '\n', '\r', '\u{c}'])
        .filter(|word| !word.is_empty())
    {
        if !display.is_empty() {
            display.push(' ');
        }
        display.push_str(word);
    }
    Cow::Owned(display)
}

pub(super) fn display_parts(
    path: &str,
    dir: &SharedString,
    name: &SharedString,
) -> (SharedString, SharedString) {
    match header_text(path) {
        Cow::Borrowed(_) => (dir.clone(), name.clone()),
        Cow::Owned(mut text) => {
            let split = text.rfind('/').map_or(0, |index| index + 1);
            let name = text.split_off(split);
            (text.into(), name.into())
        }
    }
}

pub(super) fn problem_body(message: &'static str, t: &Theme) -> Div {
    if !matches!(
        message,
        "Binary file not shown" | "File renamed without changes" | "No content"
    ) {
        return text(message, 13., 20., 400., t.text_tertiary).p(px(12.));
    }
    div()
        .relative()
        .p(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .child(shapes::decoration(Decoration {
            fill: Some(t.surface_tertiary),
            ..Default::default()
        }))
        .child(text(message, 13., 130. / 7., 430., t.text.opacity(0.65)))
}

#[derive(Default)]
pub(super) struct TextField {
    pub(super) text: String,
    pub(super) selected: bool,
}

impl TextField {
    pub(super) fn paste(&mut self, text: &str) {
        if self.selected {
            self.text.clear();
        }
        self.text
            .extend(text.chars().filter(|c| !matches!(c, '\r' | '\n')));
        self.selected = false;
    }

    pub(super) fn key(&mut self, event: &KeyDownEvent) -> bool {
        let key = &event.keystroke;
        match key.key.as_str() {
            "a" if key.modifiers.platform => self.selected = true,
            "backspace" => {
                if self.selected {
                    self.text.clear();
                } else {
                    self.text.pop();
                }
                self.selected = false;
            }
            _ if !key.modifiers.platform && !key.modifiers.control => {
                let Some(text) = &key.key_char else {
                    return false;
                };
                if text.chars().any(char::is_control) {
                    return false;
                }
                self.paste(text);
            }
            _ => return false,
        }
        true
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum DiffLayout {
    #[default]
    Auto,
    Unified,
    Split,
}

impl DiffLayout {
    pub(super) fn next(self) -> Self {
        match self {
            Self::Auto => Self::Unified,
            Self::Unified => Self::Split,
            Self::Split => Self::Auto,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto diff: switch to unified diff",
            Self::Unified => "Unified diff: switch to split diff",
            Self::Split => "Split diff: switch to auto diff",
        }
    }

    pub(super) fn icon(self) -> &'static str {
        match self {
            Self::Auto => "diff-word-diffs",
            Self::Unified => "diff-unified",
            Self::Split => "diff-split",
        }
    }

    pub(super) fn split(self, file: &FileDiff, width: Pixels) -> bool {
        self == Self::Split
            || self == Self::Auto && width >= px(800.) && file.added > 0 && file.removed > 0
    }
}

pub(super) fn split_rows(
    lines: &[Line],
) -> impl Iterator<Item = (Option<usize>, Option<usize>)> + '_ {
    let mut next = 0;
    let mut old = 0..0;
    let mut new = 0..0;
    std::iter::from_fn(move || {
        if old.is_empty() && new.is_empty() {
            let line = lines.get(next)?;
            if line.kind == Kind::Context {
                let index = next;
                next += 1;
                return Some((Some(index), Some(index)));
            }
            let start = next;
            while lines.get(next).is_some_and(|l| l.kind == Kind::Removed) {
                next += 1;
            }
            old = start..next;
            let start = next;
            while lines.get(next).is_some_and(|l| l.kind == Kind::Added) {
                next += 1;
            }
            new = start..next;
        }
        Some((old.next(), new.next()))
    })
}

impl FileDiff {
    pub(super) fn patch(path: &str, diff: &str) -> Self {
        let split = path.rfind('/').map_or(0, |i| i + 1);
        let (hunks, problem) = match parse(diff, None) {
            Ok(hunks) => {
                let problem = hunks
                    .iter()
                    .all(|hunk| hunk.lines.is_empty())
                    .then(|| patch::empty_state(diff));
                (hunks, problem)
            }
            Err(problem) => (Vec::new(), Some(problem)),
        };

        let mut added = 0;
        let mut removed = 0;
        for line in hunks.iter().flat_map(|h| &h.lines) {
            added += usize::from(line.kind == Kind::Added);
            removed += usize::from(line.kind == Kind::Removed);
        }
        if problem.is_some() {
            (added, removed) = patch::counts(diff, None);
        }
        let dir: SharedString = path[..split].to_owned().into();
        let name: SharedString = path[split..].to_owned().into();
        let (mut display_dir, mut display_name) = display_parts(path, &dir, &name);
        if let Some(old) = diff
            .lines()
            .take_while(|line| !line.starts_with("@@ "))
            .find_map(|line| line.strip_prefix("rename from "))
            .and_then(super::patch::unquote)
        {
            let label = format!("{old} -> {path}");
            display_name = match header_text(&label) {
                Cow::Borrowed(_) => label.into(),
                Cow::Owned(label) => label.into(),
            };
            display_dir = "".into();
        }
        let mut file = Self {
            dir,
            name,
            path_key: path.to_owned(),
            display_dir,
            display_name,
            added,
            removed,
            changed_bytes: diff.len(),
            hunks,
            open: true,
            problem,
            raw: diff.into(),
            raw_kind: None,
            line_info_separators: true,
            full: None,
            full_error: None,
            full_loading: false,
            original_hunks: None,
            imports: None,
            pending_expansion: None,
            preview: None,
        };
        syntax::apply(&file.name, &mut file.hunks);
        file
    }
}

pub(super) fn files(diff: &str) -> Vec<FileDiff> {
    let mut files = Vec::new();
    let mut start = 0;
    let ends = diff
        .match_indices("\ndiff --git ")
        .map(|(offset, _)| offset + 1)
        .chain(std::iter::once(diff.len()));
    for end in ends {
        let part = &diff[start..end];
        if let Some(path) = super::patch::file_path(part) {
            files.push(FileDiff::patch(&path, part));
        }
        start = end;
    }
    files
}

impl DiffPanel {
    pub(super) fn file_placeholder(
        &self,
        index: usize,
        file: &FileDiff,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Div {
        let too_large = file.problem == Some("Diff too large to display");
        let label = if too_large {
            "This file is too large to display here."
        } else {
            file.problem.unwrap_or("No content")
        };
        let empty = matches!(
            label,
            "No content" | "Binary file not shown" | "File renamed without changes"
        );
        if empty {
            return problem_body(label, t);
        }
        div()
            .flex()
            .items_center()
            .px(px(12.))
            .py(px(if too_large { 8. } else { 16. }))
            .when(too_large, |d| d.bg(t.surface_tertiary))
            .child(
                text(
                    label,
                    if too_large { 14. } else { 13. },
                    if too_large { 21. } else { 18.571_428 },
                    if too_large { 430. } else { 400. },
                    if too_large {
                        t.text_tertiary
                    } else {
                        t.text_secondary
                    },
                )
                .when(too_large, |d| d.flex_1().min_w_0().truncate()),
            )
            .when(too_large, |d| {
                d.child(
                    div()
                        .id(("diff-too-large-open", index))
                        .role(gpui::Role::Button)
                        .aria_label("Open in editor")
                        .focusable()
                        .cursor_pointer()
                        .ml(px(8.))
                        .h(px(28.))
                        .px(px(6.))
                        .border_1()
                        .border_color(gpui::black().opacity(0.))
                        .flex()
                        .items_center()
                        .rounded(px(12.5))
                        .hover(|d| d.bg(t.text_alpha_10))
                        .on_click(cx.listener(move |this, _, _, cx| this.request_open(index, cx)))
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                this.request_open(index, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .child(text("Open in editor", 13., 18., 430., t.text_tertiary)),
                )
            })
    }

    pub fn from_patch(workspace: &str, diff: &str) -> Self {
        let mut panel = Self::last_turn(workspace, workspace, &[]);
        panel.replace_files(files(diff));
        panel.standalone = true;
        panel.rich_preview = false;
        panel
    }

    pub fn cloud(workspace: &str, diff: &str) -> Self {
        let mut panel = Self::last_turn(workspace, workspace, &[]);
        panel.source = DiffSource::Cloud;
        panel.files = files(diff);
        panel
    }

    pub fn set_cloud_patch(&mut self, diff: &str, cx: &mut Context<Self>) {
        self.replace_files(files(diff));
        cx.notify();
    }

    pub fn layout(&self) -> DiffLayout {
        self.layout
    }

    pub(super) fn layout_button(&self, t: &Theme, cx: &Context<Self>) -> Stateful<Div> {
        round_icon(self.layout.icon(), 28., t)
            .id("diff-layout")
            .role(gpui::Role::Button)
            .aria_label(self.layout.label())
            .focusable()
            .tab_index(0)
            .focus_visible(|s| s.border_2().border_color(rgb(0x3a83f7)))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.set_layout(this.layout.next(), cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.set_layout(this.layout.next(), cx);
                    cx.stop_propagation();
                }
            }))
    }

    pub(super) fn standalone_header(&self, t: &Theme, cx: &Context<Self>) -> Div {
        div()
            .h(px(44.))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .pl(px(24.))
            .pr(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(text(
                        format!(
                            "{} file{} changed",
                            self.files.len(),
                            if self.files.len() == 1 { "" } else { "s" }
                        ),
                        13.,
                        18.5714,
                        430.,
                        t.text,
                    ))
                    .child(stats(
                        self.files.iter().map(|f| f.added).sum(),
                        self.files.iter().map(|f| f.removed).sum(),
                        13.,
                        t,
                    )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(self.layout_choices(t, cx))
                    .child(self.rich_preview_toggle(t, cx)),
            )
    }

    /// The original's `Toggle rich preview` ghost button: active fill while previews show.
    fn rich_preview_toggle(&self, t: &Theme, cx: &Context<Self>) -> Stateful<Div> {
        let on = self.rich_preview;
        div()
            .id("diff-rich-preview-toggle")
            .role(gpui::Role::Button)
            .aria_label("Toggle rich preview")
            .aria_toggled(if on {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .focusable()
            .tab_index(0)
            .cursor_pointer()
            .relative()
            .size(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .when(on, |d| {
                d.layer().child(shapes::decoration(shapes::Decoration {
                    radius: gpui::Corners::all(px(12.5)),
                    corner: shapes::Corner::Squircle,
                    fill: Some(t.text_alpha_5),
                    ..Default::default()
                }))
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_rich_preview(cx)))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.toggle_rich_preview(cx);
                    cx.stop_propagation();
                }
            }))
            // The original swaps icons: an image while previews show, braces (source) otherwise.
            .child(if on {
                icon("diff-rich-preview", 16., t.text)
            } else {
                icon("import-category-braces", 16., t.text_tertiary)
            })
    }

    fn layout_choices(&self, t: &Theme, cx: &Context<Self>) -> Div {
        div().flex().items_center().gap(px(2.)).children(
            [
                (DiffLayout::Unified, "Switch to unified diff"),
                (DiffLayout::Split, "Switch to split diff"),
                (DiffLayout::Auto, "Auto"),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (layout, label))| {
                div()
                    .id(("diff-layout-choice", i))
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .aria_selected(self.layout == layout)
                    .focusable()
                    .tab_index(0)
                    .cursor_pointer()
                    .relative()
                    .w(px(34.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(self.layout == layout, |d| {
                        d.layer().child(shapes::decoration(shapes::Decoration {
                            radius: gpui::Corners::all(px(12.5)),
                            corner: shapes::Corner::Squircle,
                            fill: Some(t.text_alpha_5),
                            ..Default::default()
                        }))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_layout(layout, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.set_layout(layout, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(icon(
                        layout.icon(),
                        16.,
                        if self.layout == layout {
                            t.text
                        } else {
                            t.text_tertiary
                        },
                    ))
            }),
        )
    }

    pub(super) fn standalone_file_header(
        &self,
        index: usize,
        file: &FileDiff,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(("diff-file", index))
            .role(gpui::Role::Button)
            .aria_label("Toggle file diff")
            .aria_expanded(file.open)
            .focusable()
            .tab_index(0)
            .h(px(36.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(22.))
            .pr(px(16.))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.set_file_open(index, !this.files[index].open);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(
                text(format!("{}{}", file.dir, file.name), 14., 21., 430., t.text)
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden(),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(stats(file.added, file.removed, 14., t))
                    .child(div().size(px(28.)))
                    .child(
                        div()
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon(
                                if file.open {
                                    "diff-file-toggle"
                                } else {
                                    "thread-chevron-right"
                                },
                                14.,
                                t.text_tertiary,
                            )),
                    ),
            )
    }

    pub(super) fn file_tree(
        &mut self,
        width: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = cx.theme().clone();
        // The original draws each count as SVG `<text>` centred on `dominant-baseline="middle"`,
        // which sits 1.5 px above our centred line box.
        let count = |value, sign, color| {
            text(states::signed_count(value, sign), 12., 18., 400., color)
                .relative()
                .top(px(-1.5))
        };
        let focus = self
            .filter_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        let mut rows = Vec::new();
        let query = self.filter.text.to_lowercase();
        let mut files: Vec<_> = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| {
                let path = format!("{}{}", file.dir, file.name);
                path.to_lowercase().contains(&query)
            })
            .collect();
        files.sort_by(|(_, a), (_, b)| (&a.dir, &a.name).cmp(&(&b.dir, &b.name)));
        let mut last_dir = "";
        for (i, file) in files {
            if !file.dir.is_empty() && file.dir.as_ref() != last_dir {
                rows.push(
                    div()
                        .h(px(26.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .child(icon("diff-chevron-down", 12., t.text_tertiary))
                        .child(text(
                            file.dir.trim_end_matches('/').to_owned(),
                            13.,
                            20.,
                            400.,
                            t.text,
                        ))
                        .into_any_element(),
                );
                last_dir = &file.dir;
            }
            rows.push(
                div()
                    .id(("diff-tree-file", i))
                    .role(gpui::Role::Button)
                    .aria_label(format!("{}{}", file.dir, file.name))
                    .aria_selected(self.active_file == i)
                    .h(px(29.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .pl(px(3.))
                    .pr(px(4.))
                    .rounded(px(6.))
                    .focusable()
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(|d| d.bg(t.list_hover))
                    .when(self.active_file == i, |d| d.bg(t.text_alpha_5))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_file_open(i, true);
                        this.active_file = i;
                        this.scroll
                            .scroll_to_top_of_item(if this.capped() { 0 } else { i });
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            this.set_file_open(i, true);
                            this.active_file = i;
                            this.scroll
                                .scroll_to_top_of_item(if this.capped() { 0 } else { i });
                            cx.stop_propagation();
                            cx.notify();
                        }
                    }))
                    .child(file_icon(&file.name, &t))
                    .child(
                        text(
                            file.name.clone(),
                            13.,
                            20.,
                            400.,
                            if self.active_file == i {
                                t.text
                            } else {
                                t.text_tertiary
                            },
                        )
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden(),
                    )
                    .when(file.added > 0 || file.removed > 0, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .when(file.added > 0, |d| {
                                    d.child(count(file.added, '+', t.git_added))
                                })
                                .when(file.removed > 0, |d| {
                                    d.child(count(file.removed, '-', t.git_deleted))
                                }),
                        )
                    })
                    .into_any_element(),
            );
        }
        let empty = rows.is_empty();
        div()
            .w(px(250.).min(width * 0.6))
            .h_full()
            .flex_none()
            .border_l_1()
            .border_color(t.border)
            .flex()
            .flex_col()
            .px(px(8.))
            .child(
                div()
                    .id("diff-file-filter")
                    .track_focus(&focus)
                    .role(gpui::Role::TextInput)
                    .aria_label("Filter changed files")
                    .aria_placeholder("Filter files…")
                    .aria_value(self.filter.text.clone())
                    .mt(px(4.))
                    .h(px(28.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .rounded(px(12.5))
                    .border_1()
                    .border_color(if focus.is_focused(window) {
                        t.info_solid
                    } else {
                        t.border
                    })
                    .bg(t.text.opacity(0.03))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_, _, window, cx| {
                            window.focus(&focus, cx);
                            cx.stop_propagation();
                        }),
                    )
                    .child(icon("search", 14., t.text_tertiary))
                    .child(text(
                        if self.filter.text.is_empty() {
                            "Filter files…"
                        } else {
                            &self.filter.text
                        }
                        .to_owned(),
                        14.,
                        18.,
                        430.,
                        if self.filter.text.is_empty() {
                            t.text_tertiary
                        } else {
                            t.text
                        },
                    )),
            )
            .child(
                div()
                    .id("diff-file-tree")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .mt(px(1.))
                    .children(rows)
                    .when(empty, |d| {
                        d.child(
                            text("No matching files", 13., 20., 400., t.text_tertiary).p(px(8.)),
                        )
                    }),
            )
    }

    pub(super) fn boundary(&self, t: &Theme, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        if !self.files.is_empty() {
            return None;
        }
        if self.standalone {
            return Some(
                div()
                    .flex_1()
                    .p(px(16.))
                    .child(text("No diff available", 14., 20., 400., t.text_error))
                    .into_any_element(),
            );
        }
        let (title, description) = if self.source != DiffSource::LastTurn && self.pending {
            ("Loading diff", "Refreshing changes")
        } else if self.source != DiffSource::LastTurn && self.error.is_some() {
            (
                "Couldn't load changes",
                "Refresh to try loading the changes again",
            )
        } else {
            match self.source {
                DiffSource::Staged => ("No staged changes", "Accept edits to stage them"),
                DiffSource::Unstaged => ("No unstaged changes", "Code changes will appear here"),
                DiffSource::LastTurn => (
                    "No file changes yet",
                    self.last_turn_state.map_or(
                        "Changes in this project will appear here.",
                        LastTurnState::description,
                    ),
                ),
                _ => (
                    "No file changes yet",
                    "Changes in this project will appear here.",
                ),
            }
        };
        Some(
            div()
                .id("diff-boundary")
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .px(px(24.))
                .role(gpui::Role::Status)
                .aria_label(title)
                .child(icon("thread-file-diff", 40., t.text_tertiary))
                .child(text(title, 16., 24., 500., t.text))
                .child(text(description, 13., 20., 400., t.text_tertiary))
                .when(
                    self.source == DiffSource::LastTurn && self.last_turn_state.is_some(),
                    |d| {
                        d.child(
                            div()
                                .id("diff-view-branch")
                                .role(gpui::Role::Button)
                                .aria_label("View branch diff")
                                .focusable()
                                .cursor_pointer()
                                .px(px(12.))
                                .py(px(6.))
                                .rounded(px(14.))
                                .bg(t.text_alpha_10)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.select_source(DiffSource::Branch, cx)
                                }))
                                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        this.select_source(DiffSource::Branch, cx);
                                        cx.stop_propagation();
                                    }
                                }))
                                .child(text("View branch diff", 13., 20., 430., t.text)),
                        )
                    },
                )
                .when(
                    self.error.is_some() && !self.pending && self.source != DiffSource::LastTurn,
                    |d| {
                        d.child(text(
                            self.error.clone().unwrap_or_default(),
                            12.,
                            18.,
                            400.,
                            t.text_tertiary,
                        ))
                        .child(self.retry_button(t, cx))
                    },
                )
                .into_any_element(),
        )
    }

    fn retry_button(&self, t: &Theme, cx: &Context<Self>) -> Stateful<Div> {
        div()
            .id("diff-retry")
            .role(gpui::Role::Button)
            .aria_label("Retry")
            .focusable()
            .tab_index(0)
            .cursor_pointer()
            .px(px(12.))
            .py(px(6.))
            .rounded(px(8.))
            .border_1()
            .border_color(t.border)
            .on_click(cx.listener(|_, _, _, cx| cx.emit(DiffRequest::Refresh)))
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.emit(DiffRequest::Refresh);
                    cx.stop_propagation();
                }
            }))
            .child(text("Retry", 13., 20., 500., t.text))
    }

    pub(super) fn repository_status(
        &self,
        t: &Theme,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if self.source == DiffSource::LastTurn
            || self.files.is_empty()
            || (!self.pending && self.error.is_none())
        {
            return None;
        }
        let title = if self.pending {
            "Refreshing changes"
        } else {
            "Couldn't load changes"
        };
        Some(
            div()
                .id("diff-read-status")
                .role(gpui::Role::Status)
                .aria_label(title)
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .py(px(8.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(text(title, 13., 20., 500., t.text))
                        .children(
                            self.error
                                .as_ref()
                                .map(|error| text(error.clone(), 12., 18., 400., t.text_tertiary)),
                        ),
                )
                .when(!self.pending, |d| d.child(self.retry_button(t, cx)))
                .into_any_element(),
        )
    }
}
pub(super) fn lines(contents: &str) -> impl Iterator<Item = &str> {
    contents
        .split('\n')
        .flat_map(|line| line.strip_suffix('\r').unwrap_or(line).split('\r'))
}

fn line_number(value: &str, count: u32) -> Option<u32> {
    let value = value.trim();
    let digits = value.strip_prefix('-').unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: i64 = value.parse().ok()?;
    if number == 0 || number.unsigned_abs() > 9_007_199_254_740_991 {
        return None;
    }
    let count = i64::from(count.max(1));
    Some(
        (if number < 0 {
            count + 1 + number
        } else {
            number
        })
        .clamp(1, count) as u32,
    )
}

impl FileView {
    pub(super) fn line_key(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke;
        if key.modifiers.platform && key.key == "l" {
            if self.line_count == 0 {
                return;
            }
            self.go_to_line.get_or_insert_default().selected = true;
        } else if let Some(query) = &mut self.go_to_line {
            match key.key.as_str() {
                "escape" => self.go_to_line = None,
                "enter" => {
                    if let Some(line) = line_number(&query.text, self.line_count) {
                        self.line = Some(line);
                        self.scroll.set_offset(gpui::point(
                            px(0.),
                            -px(line.saturating_sub(3) as f32 * FILE_LINE_HEIGHT),
                        ));
                        self.go_to_line = None;
                    }
                }
                _ if !query.key(event) => return,
                _ => {}
            }
        } else {
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn line_bar(&self, cx: &Context<Self>) -> Div {
        let t = cx.theme();
        let value = self
            .go_to_line
            .as_ref()
            .map_or("", |field| field.text.as_str());
        let invalid = !value.is_empty() && line_number(value, self.line_count).is_none();
        div()
            .absolute()
            .top(px(60.))
            .right(px(12.))
            .w(px(240.))
            .h(px(32.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .rounded(px(8.))
            .bg(t.surface_secondary)
            .border_1()
            .border_color(if invalid { t.git_deleted } else { t.border })
            .child(text("Line", 12., 20., 400., t.text_tertiary))
            .child(
                div()
                    .id("file-go-to-line")
                    .role(gpui::Role::TextInput)
                    .aria_label("Go to line")
                    .aria_value(value.to_owned())
                    .aria_placeholder(format!("1–{}", self.line_count.max(1)))
                    .aria_description(if invalid {
                        "Enter a valid whole line number"
                    } else {
                        "Enter a line number; negative numbers count from the end"
                    })
                    .flex_1()
                    .min_w_0()
                    .cursor_text()
                    .child(text(
                        if value.is_empty() {
                            format!("1–{}", self.line_count.max(1))
                        } else {
                            value.to_owned()
                        },
                        13.,
                        20.,
                        400.,
                        if value.is_empty() {
                            t.text_tertiary
                        } else {
                            t.text
                        },
                    )),
            )
            .child(
                round_icon("close", 20., t)
                    .id("file-go-to-line-close")
                    .role(gpui::Role::Button)
                    .aria_label("Close go to line")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.go_to_line = None;
                        cx.notify();
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_navigation_counts_from_the_end_and_clamps() {
        assert_eq!(line_number("-1", 12), Some(12));
        assert_eq!(line_number("-2", 12), Some(11));
        assert_eq!(line_number(" -99 ", 12), Some(1));
        assert_eq!(line_number("99", 12), Some(12));
        for invalid in ["0", "-0", "1.5", "+2", "", "9007199254740992"] {
            assert_eq!(line_number(invalid, 12), None, "{invalid}");
        }
        assert_eq!(
            lines("a\r\nb\rc\n").collect::<Vec<_>>(),
            ["a", "b", "c", ""]
        );
    }

    #[test]
    fn split_pairs_replacements_without_losing_one_sided_lines() {
        let hunks = parse(
            "@@ -10,4 +20,5 @@\n keep\n-old1\n-old2\n+new1\n+new2\n+new3\n tail\n",
            None,
        )
        .unwrap();
        let lines = &hunks[0].lines;
        assert_eq!(
            split_rows(lines).collect::<Vec<_>>(),
            [
                (Some(0), Some(0)),
                (Some(1), Some(3)),
                (Some(2), Some(4)),
                (None, Some(5)),
                (Some(6), Some(6)),
            ]
        );
        assert_eq!((lines[0].old_number, lines[0].number), (10, 20));
        assert_eq!((lines[6].old_number, lines[6].number), (13, 24));
    }

    #[test]
    fn auto_layout_uses_inline_for_narrow_or_one_sided_changes() {
        let mut file = FileDiff::patch("a.py", "@@ -1 +1 @@\n-old\n+new\n");
        assert!(DiffLayout::Auto.split(&file, px(1280.)));
        assert!(!DiffLayout::Auto.split(&file, px(767.)));
        assert!(!DiffLayout::Auto.split(&file, px(600.)));
        file.removed = 0;
        assert!(!DiffLayout::Auto.split(&file, px(1280.)));
        assert!(DiffLayout::Split.split(&file, px(600.)));
        assert!(!DiffLayout::Unified.split(&file, px(1280.)));
    }

    #[test]
    fn file_size_limits_match_changed_lines_and_utf16_units() {
        let line = "a".repeat(1_048_576);
        assert!(parse(&line, Some(Kind::Added)).is_ok());
        assert!(parse(&format!("{line}a"), Some(Kind::Added)).is_err());
        let unicode = "\u{1f600}".repeat(524_288);
        assert!(parse(&unicode, Some(Kind::Added)).is_ok());
        assert!(parse(&format!("{unicode}a"), Some(Kind::Added)).is_err());
        assert!(parse(&"a\n".repeat(15_000), Some(Kind::Added)).is_ok());
        assert!(parse(&"a\n".repeat(15_001), Some(Kind::Added)).is_err());
        let total = format!("{}\n", "a".repeat(524_287)).repeat(6);
        assert!(parse(&total, Some(Kind::Added)).is_ok());
        assert!(parse(&format!("{total}a"), Some(Kind::Added)).is_err());
        let context = format!(" {}\n", "a".repeat(1_048_577)).repeat(3);
        assert!(parse(&format!("@@ -1,4 +1,4 @@\n{context}-old\n+new\n"), None).is_ok());
        assert!(
            parse(
                &format!("@@ -0,0 +1 @@\n+{line}\n\\ No newline at end of file\n"),
                None,
            )
            .is_ok()
        );
    }

    #[test]
    fn oversized_files_keep_change_counts_and_large_diff_navigation() {
        let patch = format!(
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -0,0 +1,15001 @@\n{}",
            "+line\n".repeat(15_001),
        );
        let panel = DiffPanel::from_patch("/repo", &patch);
        assert_eq!(panel.files[0].problem, Some("Diff too large to display"));
        assert_eq!((panel.files[0].added, panel.files[0].removed), (15_001, 0));
        assert!(panel.capped());
    }

    #[test]
    fn invalid_hunk_numbers_never_reach_the_renderer() {
        assert!(parse("@@ -4294967295 +1 @@\n-old\n+new\n", None).is_err());
        assert!(parse("@@ -nope +1 @@\n+new\n", None).is_err());
    }

    #[test]
    fn file_delimiters_inside_added_lines_stay_in_the_file() {
        let patch = concat!(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -0,0 +1,2 @@\n",
            "+const PATCH: &str = \"diff --git a/fake b/fake\";\n+after\n",
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -0,0 +1 @@\n+second\n",
        );
        for patch in [patch.to_owned(), patch.replace('\n', "\r\n")] {
            let parsed = files(&patch);
            assert_eq!(
                parsed
                    .iter()
                    .map(|file| file.name.as_ref())
                    .collect::<Vec<_>>(),
                ["a.rs", "b.rs"]
            );
            assert_eq!(
                parsed[0].hunks[0]
                    .lines
                    .iter()
                    .map(|line| line.text.as_ref())
                    .collect::<Vec<_>>(),
                ["const PATCH: &str = \"diff --git a/fake b/fake\";", "after"]
            );
            assert_eq!(parsed[1].hunks[0].lines[0].text.as_ref(), "second");
        }
    }

    #[test]
    fn changes_preserve_decoded_and_metadata_only_file_paths() {
        let patch = concat!(
            "diff --git \"a/dir space/\\303\\251.txt\" \"b/dir space/\\303\\251.txt\"\n",
            "--- \"a/dir space/\\303\\251.txt\"\n+++ \"b/dir space/\\303\\251.txt\"\n",
            "@@ -1 +1 @@\n-old\n+new\n",
            "diff --git a/old.txt b/new name.txt\nsimilarity index 100%\nrename from old.txt\nrename to new name.txt\n",
            "diff --git a/image.bin b/image.bin\nBinary files a/image.bin and b/image.bin differ\n",
            "diff --git a/empty.txt b/empty.txt\nnew file mode 100644\nindex 0000000..e69de29\n",
            "diff --git a/file.txt a/file.txt\n--- a/file.txt\n+++ a/file.txt\n@@ -1 +1 @@\n-before\n+after\n",
        );
        let panel = DiffPanel::from_patch("/repo", patch);
        assert_eq!(
            panel
                .files
                .iter()
                .map(|file| format!("{}{}", file.dir, file.name))
                .collect::<Vec<_>>(),
            [
                "dir space/é.txt",
                "new name.txt",
                "image.bin",
                "empty.txt",
                "a/file.txt"
            ],
        );
        assert_eq!((panel.files[0].added, panel.files[0].removed), (1, 1));
        assert_eq!(panel.files[0].hunks[0].lines[1].text.as_ref(), "new");
        assert_eq!(
            panel.files[1].display_name.as_ref(),
            "old.txt -> new name.txt"
        );
        assert_eq!(panel.files[1].problem, Some("File renamed without changes"));
        assert_eq!(panel.files[2].problem, Some("Binary file not shown"));
        assert_eq!(panel.files[3].problem, Some("No content"));
        assert!(panel.files[1..4].iter().all(|file| file.hunks.is_empty()));
    }

    #[test]
    fn displayed_path_whitespace_does_not_change_the_action_path() {
        let file = FileDiff::patch("dir space/é\tname.txt", "@@ -1 +1 @@\n-before\n+after\n");
        assert_eq!(file.dir.as_ref(), "dir space/");
        assert_eq!(file.name.as_ref(), "é\tname.txt");
        assert_eq!(file.display_dir.as_ref(), "dir space/");
        assert_eq!(file.display_name.as_ref(), "é name.txt");
    }

    #[test]
    fn hunk_text_cannot_redirect_a_changes_file() {
        let panel = DiffPanel::from_patch(
            "/repo",
            "diff --git a/right.txt b/right.txt\n@@ -0,0 +1 @@\n+++ b/wrong.txt\n",
        );
        assert_eq!(panel.files[0].name.as_ref(), "right.txt");
        assert_eq!(
            panel.files[0].hunks[0].lines[0].text.as_ref(),
            "++ b/wrong.txt"
        );
    }

    #[test]
    fn active_file_survives_reordering_and_falls_back_after_removal() {
        let patch = "@@ -0,0 +1 @@\n+line\n";
        let mut panel = DiffPanel::from_patch("", "");
        panel.files = vec![
            FileDiff::patch("a.rs", patch),
            FileDiff::patch("b.rs", patch),
        ];
        panel.active_file = 1;
        panel.replace_files(vec![
            FileDiff::patch("b.rs", patch),
            FileDiff::patch("a.rs", patch),
        ]);
        assert_eq!(panel.files[panel.active_file].name.as_ref(), "b.rs");
        panel.replace_files(vec![FileDiff::patch("c.rs", patch)]);
        assert_eq!(panel.files[panel.active_file].name.as_ref(), "c.rs");
    }
}
