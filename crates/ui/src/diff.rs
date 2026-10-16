//! Side panel of a thread route: the split layout and titlebar tab strip that refs/diff and
//! refs/terminal share, and the Changes panel with the unified diff of the last turn's file
//! changes. All numbers are the spec's CSS px.

use crate::{
    MONO, SANS, Shell, composer::shadow, fixture, icon::icon, layout_color, over,
    screens::thread::view::ThreadView, text,
};
use bridge::{
    Event,
    types::{ThreadItem, ThreadReadResponse, ThreadStartResponse, Turn},
};
use gpui::{
    Action, AnyView, App, AppContext as _, ClickEvent, ClipboardItem, Context, Corners, Div,
    Entity, FocusHandle, Font, FontFeatures, FontWeight, HighlightStyle, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Render, Rgba, ScrollHandle,
    SharedString, Stateful, StatefulInteractiveElement, Styled, StyledText, TextRun, Window, black,
    combine_highlights, deferred, div, font, prelude::FluentBuilder as _, px, rgb, white,
};
use shapes::Decoration;
use std::{cell::Cell, ops::Range, sync::Arc};
use theme::{ActiveTheme as _, Appearance, Theme};

#[path = "diff/menu.rs"]
mod menu;

/// Panel width the capture had stored (`app-shell:right-panel-width:v3`) before clamping.
const STORED_WIDTH: f32 = 664.17;
/// Room the chat keeps beside the panel (`unified` layout), also the panel's minimum.
const MIN_CHAT: f32 = 320.;
/// `--diffs-font-size`, `--diffs-line-height`.
const CODE_SIZE: f32 = 12.;
const CODE_LINE: f32 = 21.6;
/// Menlo advance at 12 px (0.602em); `--diffs-min-number-column-width` is 4ch.
const CH: f32 = 7.224;

/// Panel width for the current window, as the original clamps its stored width.
/// ponytail: assumes the sidebar is open; a collapsed sidebar only leaves the chat wider.
pub fn panel_width(window: &Window) -> Pixels {
    // Main panel inner width: window minus sidebar, the main panel's 0.5 px hairline and 4 px
    // inset.
    let main = window.viewport_size().width - Theme::SIDEBAR_WIDTH - px(4.5);
    px(STORED_WIDTH).min((main - px(MIN_CHAT)).max(px(MIN_CHAT)))
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
pub fn panel_clamped(window: &Window) -> bool {
    panel_width(window) < px(STORED_WIDTH)
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
}

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
            let hunks = match change.kind["type"].as_str() {
                Some(kind @ ("add" | "delete")) => vec![Hunk {
                    separator: None,
                    lines: change
                        .diff
                        .lines()
                        .enumerate()
                        .map(|(i, text)| Line {
                            kind: if kind == "add" {
                                Kind::Added
                            } else {
                                Kind::Removed
                            },
                            number: i as u32 + 1,
                            text: text.to_owned().into(),
                            matches: Vec::new(),
                        })
                        .collect(),
                }],
                _ => parse(&change.diff),
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
        }
    }

    pub fn search(&self) -> Option<(&str, usize)> {
        self.query.as_deref().map(|query| (query, self.matches))
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
                diff.split("diff --git ")
                    .skip(1)
                    .filter_map(|part| {
                        let path = part
                            .lines()
                            .find_map(|line| line.strip_prefix("+++ b/"))
                            .or_else(|| {
                                part.lines().find_map(|line| line.strip_prefix("--- a/"))
                            })?;
                        let split = path.rfind('/').map_or(0, |i| i + 1);
                        let hunks = parse(part);
                        let added = hunks
                            .iter()
                            .flat_map(|h| &h.lines)
                            .filter(|l| l.kind == Kind::Added)
                            .count();
                        let removed = hunks
                            .iter()
                            .flat_map(|h| &h.lines)
                            .filter(|l| l.kind == Kind::Removed)
                            .count();
                        Some(FileDiff {
                            dir: path[..split].to_owned().into(),
                            name: path[split..].to_owned().into(),
                            added,
                            removed,
                            hunks,
                            open: true,
                        })
                    })
                    .collect()
            }
            "item/started" | "item/completed"
                if params["item"]["type"] == "fileChange"
                    && self.aggregated_turn.as_deref() != Some(&turn.id) =>
            {
                Self::last_turn(workspace, workspace, turns).files
            }
            _ => return,
        };
        self.replace_files(files);
        cx.notify();
    }

    fn replace_files(&mut self, mut files: Vec<FileDiff>) {
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
        self.menu = None;
        self.search_changed();
    }

    fn search_changed(&mut self) {
        let query = self.query.as_deref().unwrap_or("");
        self.matches = 0;
        for file in &mut self.files {
            let before = self.matches;
            for line in file.hunks.iter_mut().flat_map(|h| &mut h.lines) {
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
        if key.modifiers.platform && key.key == "f" {
            self.query.get_or_insert_default();
            self.selected_query = true;
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

    fn header(&self, t: &Theme, wide: bool, clamped: bool) -> Div {
        let added = self.files.iter().map(|f| f.added).sum();
        let removed = self.files.iter().map(|f| f.removed).sum();
        let tool = |name| round_icon(name, 28., t);
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
                            .relative()
                            .h(px(28.))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .pl(px(12.))
                            .pr(px(6.))
                            .child(lens())
                            .child(text("Last Turn", 13., 20., 430., t.text))
                            .child(icon("diff-chevron-down", 12., t.text)),
                    )
                    .child(stats(added, removed, 14., t).mr(px(4.))),
            )
            .child(
                capsule(t, clamped)
                    .child(tool("diff-more"))
                    .child(tool("diff-jump-to-file"))
                    // `@container review-header (min-width: 625px)`
                    .when(wide, |d| {
                        d.child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .child(tool("diff-refresh"))
                                .child(lensed("diff-word-wrap"))
                                .child(tool("diff-load-full-files"))
                                .child(tool("diff-word-diffs")),
                        )
                    })
                    .child(lensed("diff-show-files")),
            )
    }
}

impl Render for DiffPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme().clone();
        let wide = panel_width(window) - px(1.) >= px(625.);
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        let ordinal = Cell::new(0);
        let line_index = Cell::new(0);
        let files = self.files.iter().enumerate().map(|(i, file)| {
            div()
                .flex()
                .flex_col()
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                        this.open_menu(i, menu::Kind::Body, event.position, window, cx);
                    }),
                )
                .child(
                    file_header(i, file, &t, cx).on_click(cx.listener(move |this, _, _, cx| {
                        this.files[i].open ^= true;
                        this.selection = None;
                        cx.notify();
                    })),
                )
                .map(|d| {
                    if file.open {
                        d.children(
                            file.hunks
                                .iter()
                                .map(|h| hunk(h, &t, &ordinal, &line_index, self, cx)),
                        )
                    } else {
                        line_index.set(
                            line_index.get()
                                + file.hunks.iter().map(|h| h.lines.len()).sum::<usize>(),
                        );
                        d
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
            .on_action(cx.listener(|this, _: &crate::Copy, _, cx| {
                this.copy_selection(cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.selecting = false;
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .relative()
                    .flex_none()
                    .overflow_hidden()
                    .pb(px(1.))
                    // Skia blends it like Chromium (a GPUI quad rounds apart).
                    .child(
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
                    .child(self.header(&t, wide, panel_clamped(window))),
            )
            .child(
                div()
                    .id("diff-files")
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .pb(px(12.))
                    .children(files),
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

impl SplitScreen {
    /// Titlebar row over a side panel (`Task tabs`): every open tab in a 240 px slot (the
    /// selected one on its surface, the others `text-tertiary`, surface and `×` on hover),
    /// `New tab`, and the `Enter full view` / `Hide tabs` buttons. The caller places it from
    /// the panel's left edge to the window's right edge, 44 px tall.
    fn tab_strip(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> Div {
        /// Label box width and its `text-fade-truncate` distance.
        const LABEL_WIDTH: f32 = 181.;
        const FADE: f32 = 16.;
        let layout = layout_color(t);
        let fill = match t.appearance {
            Appearance::Dark => t.ghost_hover,
            Appearance::Light => t.surface_elevated_secondary,
        };
        let tab = |ix: usize, (tab_icon, label): &Tab, cx: &mut Context<Self>| {
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
                LABEL_WIDTH
            } else {
                LABEL_WIDTH + 20.
            };
            let fade = width > px(label_width - FADE);
            div()
                .id(("side-tab", ix))
                .relative()
                .w(px(238.))
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
                            .w(px(240.))
                            .h(px(34.))
                            .layer()
                            .child(
                                div()
                                    .absolute()
                                    .left(px(1.))
                                    .top_0()
                                    .w(px(238.))
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
                    d.child(
                        div()
                            .id("side-panel-close-tab")
                            .absolute()
                            .top(px(6.))
                            .right(px(7.))
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(dispatch_action(CloseSideTab(ix)))
                            .child(icon("diff-tab-close", 14., t.text_tertiary)),
                    )
                })
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
                // The tab's box clips its shadow (`overflow` on the 240 px tab slot).
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .w(px(240.))
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
                    .child(button("diff-hide-tabs", t.text, Some(t.text_alpha_5))),
            )
    }
}

/// Click handler dispatching `action`.
fn dispatch_action(action: impl Action) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx)
}

/// Main panel of a thread with a side panel open: the thread, then the panel behind its 1 px
/// `border-l`, ending at the main panel's right edge.
struct SplitMain {
    thread: AnyView,
    panel: AnyView,
}

impl Render for SplitMain {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        div()
            .size_full()
            .flex()
            .child(div().flex_1().min_w_0().child(self.thread.clone()))
            .child(
                div()
                    .w(panel_width(window))
                    .flex_none()
                    .border_l_1()
                    // The panel content's `[contain:layout_paint]`.
                    .snap_origin()
                    .border_color(over(t.border, t.surface))
                    .child(self.panel.clone()),
            )
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
}

impl Render for SplitScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The panel ends at the main panel's right edge, 4 px
        // (`--app-shell-page-inline-end-inset`) from the window's.
        let left = (!self.tabs.is_empty())
            .then(|| window.viewport_size().width - px(4.) - panel_width(window));
        // ponytail: like refs/thread, the header only follows the open-sidebar titlebar layout.
        let open = self.shell.read(cx).sidebar_open;
        let sidebar_width = self.shell.read(cx).sidebar_width;
        div()
            .size_full()
            .relative()
            .font_family(SANS)
            .child(self.shell.clone())
            .when(open, |d| {
                d.child(
                    self.thread
                        .update(cx, |view, cx| view.titlebar_header(cx))
                        .left(sidebar_width)
                        .when_some(left, |h, left| h.w(left - sidebar_width)),
                )
            })
            // Without a panel the shell's `New tab` group follows the header behind a divider.
            .when(open && left.is_none(), |d| {
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
    let main = match &panel {
        Some((panel, ..)) => cx
            .new(|_| SplitMain {
                thread: thread.clone().into(),
                panel: panel.clone(),
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
                let lines: Vec<_> = contents.lines().collect();
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
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .border_b_1()
                    .border_color(t.border)
                    .child(self.header(&t, panel_clamped(window))),
            )
            .child(body)
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

fn file_header(i: usize, file: &FileDiff, t: &Theme, cx: &Context<DiffPanel>) -> Stateful<Div> {
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
                .child(
                    small("diff-open-in")
                        .id(("diff-open-editor", i))
                        .role(gpui::Role::Button)
                        .aria_label("Open in editor")
                        .aria_disabled(true)
                        .on_click(|_, _, cx| cx.stop_propagation()),
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
            keyword: c(0xf8a6c8, 0xab4f7a),
            name: c(0x63a8f8, 0x1f4e94),
            string: c(0x83d197, 0x3a843f),
            comment: c(0xb9b9b9, 0x4f4f4f),
            literal: c(0xf1a275, 0xac4f23),
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
                let color = if KEYWORDS.contains(&word) {
                    Some(s.keyword)
                } else if matches!(word, "True" | "False" | "None") {
                    Some(s.literal)
                } else if matches!(prev, "def" | "class") {
                    Some(s.name)
                } else {
                    None
                };
                prev = word;
                color
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

fn hunk(
    h: &Hunk,
    t: &Theme,
    ordinal: &Cell<usize>,
    line_index: &Cell<usize>,
    panel: &DiffPanel,
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
    let gutter = px(CH * digits.max(4) as f32 + 16.);
    let (base, mixer) = match t.appearance {
        Appearance::Dark => (black(), 0.15),
        Appearance::Light => (white(), 0.09),
    };
    let row = |number: Option<u32>, number_bg: Hsla, bg: Hsla, content: StyledText| {
        div()
            .h(px(CODE_LINE))
            .when(panel.wrap, |d| d.h_auto().min_h(px(CODE_LINE)))
            .flex()
            .child(
                div()
                    .w(gutter)
                    .flex_none()
                    .h_full()
                    .when(panel.wrap, |d| d.h_auto())
                    .flex()
                    .justify_end()
                    .items_center()
                    .when(panel.wrap, |d| d.items_start())
                    .pr(px(8.))
                    .bg(number_bg)
                    .text_color(t.text_tertiary)
                    .when_some(number, |d, n| d.child(n.to_string())),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .when(panel.wrap, |d| d.h_auto().min_h(px(CODE_LINE)).min_w_0())
                    .flex()
                    .items_center()
                    .when(panel.wrap, |d| d.flex_col().items_stretch())
                    .pl(px(8.))
                    .bg(bg)
                    .whitespace_nowrap()
                    .when(panel.wrap, |d| d.whitespace_normal())
                    .child(content),
            )
    };
    let separator = mix(t.surface, t.info_solid, 0.06);
    div()
        .flex()
        .flex_col()
        .font_family(MONO)
        .text_size(px(CODE_SIZE))
        .line_height(px(CODE_LINE))
        .text_color(t.text)
        .when_some(h.separator.clone(), |d, label| {
            let len = label.len();
            d.child(row(
                None,
                separator,
                separator,
                StyledText::new(label).with_highlights([(
                    0..len,
                    HighlightStyle {
                        color: Some(t.text_tertiary),
                        ..Default::default()
                    },
                )]),
            ))
        })
        .children(h.lines.iter().map(|l| {
            let (number_bg, bg) = match l.kind {
                Kind::Context => (t.surface, t.surface),
                Kind::Added => (mix(base, t.git_added, mixer), over(t.diff_added, t.surface)),
                Kind::Removed => (
                    mix(base, t.git_deleted, mixer),
                    over(t.diff_deleted, t.surface),
                ),
            };
            let found = l.matches.iter().map(|range| {
                let index = ordinal.get();
                ordinal.set(index + 1);
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
            let index = line_index.get();
            line_index.set(index + 1);
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
            let content = StyledText::new(l.text.clone()).with_highlights(combine_highlights(
                combine_highlights(highlight(&l.text, &syntax), found),
                selection,
            ));
            let down_layout = content.layout().clone();
            let move_layout = down_layout.clone();
            row(Some(l.number), number_bg, bg, content)
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
        }))
}

/// Hunks of a unified diff; file headers (`diff --git`, `---`, `+++`) before the first `@@`
/// are skipped.
fn parse(diff: &str) -> Vec<Hunk> {
    let mut hunks: Vec<Hunk> = Vec::new();
    // `old` starts at 1 so the lines before the first hunk count as skipped.
    let (mut old, mut new): (u32, u32) = (1, 0);
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("@@ -") {
            let start = |s: &str| {
                s.split(',')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0)
            };
            let mut ranges = rest.split(' ');
            let old_start: u32 = start(ranges.next().unwrap_or(""));
            let skipped = old_start.saturating_sub(old);
            old = old_start;
            new = start(ranges.next().unwrap_or("").trim_start_matches('+'));
            let plural = if skipped == 1 { "" } else { "s" };
            hunks.push(Hunk {
                separator: (skipped > 0)
                    .then(|| format!("{skipped} unmodified line{plural}").into()),
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
            _ => continue, // `\ No newline at end of file`
        };
        if kind != Kind::Removed {
            new += 1;
        }
        if kind != Kind::Added {
            old += 1;
        }
        hunk.lines.push(Line {
            kind,
            number,
            text: line.get(1..).unwrap_or("").to_string().into(),
            matches: Vec::new(),
        });
    }
    hunks
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
    fn python_tokens() {
        let s = Syntax::of(&Theme::dark());
        let spans: Vec<_> = highlight("def subtract(a, b): # x", &s)
            .into_iter()
            .map(|(r, h)| (r, h.color))
            .collect();
        assert_eq!(
            spans,
            [
                (0..3, Some(s.keyword)),
                (4..12, Some(s.name)),
                (20..23, Some(s.comment))
            ]
        );
    }
}
