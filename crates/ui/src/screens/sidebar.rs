//! `sidebar` parity screen (Home with chat search open) and the Home sidebar every chat route
//! shows: header, Projects, and Recents from `thread/list`. Numbers are CSS px from
//! refs/home and refs/sidebar.

use crate::{
    Shell, blit, fixture, home_sidebar_header, icon::icon, sidebar_empty, sidebar_section, text,
};
use bridge::types::{Thread, ThreadListResponse};
use gpui::{
    Action, AnyElement, AnyView, App, AppContext as _, Context, Corners, Div, EventEmitter,
    FocusHandle, Focusable, FontWeight, HighlightStyle, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Pixels, Render, Rgba, ScrollHandle, SharedString,
    Stateful, StatefulInteractiveElement, Styled, StyledText, TextRun, Window, div, point,
    prelude::FluentBuilder as _, px, relative, rgba,
};
use std::{ops::Range, path::Path};
use theme::{ActiveTheme as _, Appearance, Theme};

/// `.sidebar-item`: `--radius-lg` (12.5 px) with `corner-shape: superellipse(1.5)`.
const RADIUS_ROW: Pixels = px(12.5);

/// Recents row label: the thread name, else its preview cut to 80 chars with `…`.
pub(crate) fn title(thread: &Thread) -> SharedString {
    if let Some(name) = &thread.name {
        return name.clone().into();
    }
    let preview = thread.preview.trim();
    if preview.chars().count() <= 80 {
        preview.to_owned().into()
    } else {
        format!("{}…", preview.chars().take(79).collect::<String>()).into()
    }
}

pub struct HomeSidebar {
    /// `(thread id, row label)` in `thread/list` order.
    threads: Vec<(String, SharedString)>,
    selected: Option<String>,
    /// `Projects`, `Recents` collapsed by a click on their header.
    collapsed: [bool; 2],
    /// Threads whose rows carry the unread dot; any lights the header's bell.
    pub unread: Vec<String>,
    /// The sections' scroller (Projects and Recents under the header).
    pub scroll: ScrollHandle,
}

/// Emitted when a Recents row is clicked, with the thread id.
pub struct OpenThread(pub String);

impl EventEmitter<OpenThread> for HomeSidebar {}

/// A Recents row's `Archive chat`: `thread/archive` on that thread (its id).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = sidebar, no_json)]
pub struct ArchiveThread(pub String);

impl HomeSidebar {
    /// Recents from a live `thread/list`; `selected` is the open thread, highlighted.
    pub fn new(threads: &[Thread], selected: Option<&str>) -> Self {
        Self {
            threads: threads.iter().map(|t| (t.id.clone(), title(t))).collect(),
            selected: selected.map(str::to_owned),
            collapsed: [false; 2],
            unread: Vec::new(),
            scroll: ScrollHandle::new(),
        }
    }

    pub fn set_threads(&mut self, threads: &[Thread], cx: &mut Context<Self>) {
        self.threads = threads.iter().map(|t| (t.id.clone(), title(t))).collect();
        cx.notify();
    }

    /// Puts `thread` at the top of Recents (`thread/started`), keeping its current label
    /// while the thread has no title yet.
    pub fn add_thread(&mut self, thread: &Thread, cx: &mut Context<Self>) {
        let old = self.threads.iter().position(|(id, _)| *id == thread.id);
        let old = old.map(|i| self.threads.remove(i).1);
        let label = match (title(thread), old) {
            (label, Some(old)) if label.is_empty() => old,
            (label, _) => label,
        };
        self.threads.insert(0, (thread.id.clone(), label));
        cx.notify();
    }

    pub fn select(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        self.selected = id.map(str::to_owned);
        cx.notify();
    }
}

/// The Home sidebar with Recents from the fixture `thread/list`; `selected_thread` is the id of
/// the open thread, highlighted in Recents.
pub fn home_sidebar(cx: &mut App, selected_thread: Option<&str>) -> AnyView {
    let list: ThreadListResponse = fixture::response("readback.jsonl", "thread/list", 0);
    cx.new(|_| HomeSidebar::new(&list.data, selected_thread))
        .into()
}

impl Render for HomeSidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let recents = if self.threads.is_empty() {
            sidebar_empty("No chats", cx).into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(1.))
                .children(self.threads.iter().map(|(id, label)| {
                    let open = id.clone();
                    let unread = self.unread.contains(id);
                    thread_row(
                        id,
                        label.clone(),
                        self.selected.as_ref() == Some(id),
                        unread,
                        t,
                    )
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(OpenThread(open.clone()))))
                }))
                .into_any_element()
        };
        // A header click collapses or expands its section (`group/nav-section`).
        let section = |i: usize, title: &'static str, body: AnyElement, cx: &mut Context<Self>| {
            let body = if self.collapsed[i] {
                div().into_any_element()
            } else {
                body
            };
            sidebar_section(title, body, cx).relative().child(
                div()
                    .id(title)
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(30.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.collapsed[i] ^= true;
                        cx.notify();
                    })),
            )
        };
        let projects = section(
            0,
            "Projects",
            sidebar_empty("No projects", cx).into_any_element(),
            cx,
        );
        let recents = section(1, "Recents", recents, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(home_sidebar_header(!self.unread.is_empty(), cx))
            .child(
                // `overflow-y-auto`: the sections scroll under the header and clip at its
                // bottom edge. The extent ends 16 px past Recents: its wrapper's `pb-4`
                // (cancelled in layout by `-mb-4`) reaches past the scroller's own 8 px
                // `pb-row-x`, and Chromium's scrollable overflow takes the farther one.
                div()
                    .id("sidebar-sections")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .pt(px(4.))
                    .pb(px(16.))
                    .child(projects)
                    .child(recents),
            )
    }
}

/// One Recents row: 30 px, `primary-ghost-hover` fill when hovered or selected. The label is
/// clipped at the row's end with the marquee's 16 px right fade (`_overflowingViewport`); on
/// hover `Pin chat` and `Archive chat` appear at the end. An `unread` row ends in the blue
/// dot, which takes 28 px off the label.
fn thread_row(
    id: &str,
    label: SharedString,
    selected: bool,
    unread: bool,
    t: &Theme,
) -> Stateful<Div> {
    let clip = if unread { 28. } else { 0. };
    let group = SharedString::from(format!("thread-row-{id}"));
    let fill = div()
        .absolute()
        .inset_0()
        .child(shapes::decoration(shapes::Decoration {
            radius: Corners::all(RADIUS_ROW),
            corner: shapes::Corner::Squircle,
            fill: Some(t.ghost_hover),
            ..Default::default()
        }));
    let action = |id: &'static str, name: &'static str| {
        div()
            .id(id)
            .size(px(20.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(crate::motion::ButtonVisual {
                glyph: icon(name, 14., t.text_tertiary),
                radius: px(6.),
                hover: t.ghost_hover,
                foreground: t.text.opacity(0.85),
                selected: false,
            })
    };
    div()
        .id(group.clone())
        .group(group.clone())
        .relative()
        .h(px(30.))
        .child(if selected {
            fill
        } else {
            fill.opacity(0.)
                .group_hover(group.clone(), |s| s.opacity(1.))
        })
        .child(
            // `_clipViewport _overflowingViewport`: the marquee's CSS mask (8 px left fade over
            // the track's padding, 16 px right fade), a masked layer in the sidebar's layer.
            div()
                .absolute()
                .left_0()
                .top(px(5.))
                .w(px(264. - clip))
                .h(px(20.))
                .pl(px(8.))
                .overflow_hidden()
                .layer()
                .child(text(label, 14., 20., 430., t.text_default_nav))
                .child(shapes::x_fade(px(8.), px(16.))),
        )
        .when(unread, |d| {
            d.child(
                div()
                    .absolute()
                    .top(px(11.))
                    .right(px(14.))
                    .size(px(8.))
                    .group_hover(group.clone(), |s| s.opacity(0.))
                    .child(shapes::decoration(shapes::Decoration {
                        radius: Corners::all(Theme::RADIUS_FULL),
                        fill: Some(t.info_solid),
                        ..Default::default()
                    })),
            )
        })
        .child(
            div()
                .absolute()
                .top_0()
                .right(px(2.))
                .h_full()
                .w(px(52.))
                .pr(px(6.))
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .opacity(0.)
                .group_hover(group, |s| s.opacity(1.))
                .child(action("pin-chat", "sidebar-pin"))
                .child({
                    let id = id.to_owned();
                    action("archive-chat", "sidebar-archive").on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(ArchiveThread(id.clone())), cx)
                    })
                }),
        )
}

/// One command menu row: optional 16 px leading icon, label, then a description, the chat's
/// project, its key equivalent, or an external-link arrow.
#[derive(Default)]
struct Item {
    icon: Option<&'static str>,
    label: SharedString,
    /// Text the query also matches: a command's description, a chat's first message.
    keywords: SharedString,
    /// Chat rows: workspace folder name; `Some` marks the row as a chat.
    project: Option<SharedString>,
    keys: Option<SharedString>,
    /// Chat rows: the thread id.
    thread: Option<String>,
    description: Option<&'static str>,
    external: bool,
}

impl Item {
    fn command(icon: Option<&'static str>, label: &'static str, keywords: &'static str) -> Self {
        Self {
            icon,
            label: label.into(),
            keywords: keywords.into(),
            ..Default::default()
        }
    }

    fn keys(self, keys: &'static str) -> Self {
        Self {
            keys: Some(keys.into()),
            ..self
        }
    }

    fn is_chat(&self) -> bool {
        self.project.is_some()
    }

    /// Chats match the query as a substring of their title or first message, commands with
    /// [`fuzzy_match`] over title and description.
    fn matches(&self, query: &str) -> bool {
        if self.is_chat() {
            let query = query.to_lowercase();
            [&self.label, &self.keywords]
                .iter()
                .any(|s| s.to_lowercase().contains(&query))
        } else {
            fuzzy_match(&format!("{} {}", self.label, self.keywords), query)
        }
    }
}

/// The command menu's filter (a word-start matcher, `*query`): the query splits into
/// contiguous fragments; the first may start anywhere, each next one at a word start.
fn fuzzy_match(text: &str, query: &str) -> bool {
    fn word_start(t: &[char], i: usize) -> bool {
        i == 0 || !t[i - 1].is_alphanumeric() || (t[i].is_uppercase() && t[i - 1].is_lowercase())
    }
    fn rest(t: &[char], q: &[char], from: usize, first: bool) -> bool {
        if q.is_empty() {
            return true;
        }
        (from..t.len()).any(|i| {
            (first || word_start(t, i))
                && (1..=q.len()).any(|k| {
                    t.len() >= i + k
                        && t[i..i + k]
                            .iter()
                            .zip(&q[..k])
                            .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
                        && rest(t, &q[k..], i + k, false)
                })
        })
    }
    let t: Vec<char> = text.chars().collect();
    let q: Vec<char> = query.trim().chars().collect();
    rest(&t, &q, 0, true)
}

/// Which chars of `text` the original highlights for `query`: all for an empty query, else
/// the first case-insensitive occurrence, else (`fuzzy`) a greedy in-order char match.
fn highlight(text: &str, query: &str, fuzzy: bool) -> Vec<bool> {
    let lower: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let q: Vec<char> = query.trim().chars().flat_map(char::to_lowercase).collect();
    // ponytail: assumes lowercasing keeps the char count (true for the fixture's ASCII).
    if q.is_empty() || lower.len() != text.chars().count() {
        return vec![q.is_empty(); text.chars().count()];
    }
    if let Some(start) = lower.windows(q.len()).position(|w| w == q.as_slice()) {
        return (0..lower.len())
            .map(|i| (start..start + q.len()).contains(&i))
            .collect();
    }
    let mut next = 0;
    lower
        .iter()
        .map(|c| {
            let hit = fuzzy && next < q.len() && *c == q[next];
            next += usize::from(hit);
            hit
        })
        .collect()
}

/// [`text`] with the chars `marks` flags in `on` and the others in `off`.
fn marked(
    s: SharedString,
    marks: &[bool],
    size: f32,
    line_height: f32,
    on: Hsla,
    off: Hsla,
) -> Div {
    // One run per stretch of equal marks.
    let mut runs: Vec<(Range<usize>, bool)> = Vec::new();
    for ((i, c), &m) in s.char_indices().zip(marks) {
        match runs.last_mut() {
            Some((r, prev)) if *prev == m => r.end = i + c.len_utf8(),
            _ => runs.push((i..i + c.len_utf8(), m)),
        }
    }
    // `HighlightStyle::color` blends over the base color and keeps the base alpha, so each run
    // sets the opaque color and its alpha through `fade_out`.
    let style = |c: Hsla| HighlightStyle {
        color: Some(Hsla { a: 1., ..c }),
        fade_out: Some(1. - c.a),
        ..Default::default()
    };
    div()
        .text_size(px(size))
        .line_height(px(line_height))
        .font_weight(FontWeight(430.))
        .text_color(Hsla { a: 1., ..off })
        .whitespace_nowrap()
        .child(
            StyledText::new(s).with_highlights(
                runs.into_iter()
                    .map(|(r, m)| (r, style(if m { on } else { off }))),
            ),
        )
}

/// `CommandMenuDialog` in chat-search mode (sidebar `Search`): the query input over `Chats`
/// (from `thread/list`), `Quick actions`, `Settings` and `App`, filtered by the query. The
/// selected row carries the `primary-ghost-hover` fill, the others `opacity: .75`.
/// ponytail: the original also lists ~50 more commands (Chat, Navigation, Panels, ...) after
/// `Settings`; add them with the commands they run.
pub struct CommandMenu {
    groups: Vec<(&'static str, Vec<Item>)>,
    query: String,
    selected: usize,
    focus: FocusHandle,
}

/// What the command menu asks its owner to do.
pub enum CommandMenuEvent {
    /// Escape or a click outside the dialog.
    Dismiss,
    /// A chat row: open the thread.
    OpenThread(String),
    /// A command row, by its label (`New chat`, `General`, `Manage scheduled tasks`, ...).
    Command(SharedString),
}

impl EventEmitter<CommandMenuEvent> for CommandMenu {}

impl Focusable for CommandMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl CommandMenu {
    /// Over the fixture's `thread/list` (refs/sidebar).
    pub fn new(query: &str, cx: &mut Context<Self>) -> Self {
        let list: ThreadListResponse = fixture::response("readback.jsonl", "thread/list", 0);
        Self::with_threads(&list.data, query, cx)
    }

    /// `Chats` from `threads` (a live `thread/list`), in that order.
    pub fn with_threads(threads: &[Thread], query: &str, cx: &mut Context<Self>) -> Self {
        let chats = threads
            .iter()
            .map(|t| Item {
                label: title(t),
                keywords: t.preview.clone().into(),
                project: Some(
                    Path::new(&t.cwd)
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
                        .into(),
                ),
                thread: Some(t.id.clone()),
                ..Default::default()
            })
            .collect();
        // Settings pages; `true` = trailing external-link arrow.
        let settings = [
            ("settings-general", "General", false),
            ("bell", "Notifications", false),
            ("settings-import", "Import", false),
            ("settings-profile", "Profile", true),
            ("settings-profile", "Account", false),
            ("settings-profile", "Parental controls", false),
            ("settings-profile", "Trusted contact", false),
            ("settings-appearance", "Appearance", false),
            ("settings-voice", "Voice", false),
            ("settings-storage", "Storage", false),
            ("settings-mini-pets", "Mini & Pets", false),
            ("settings-computer-history", "Computer history", false),
            ("settings-appshots", "Appshots", false),
            ("settings-git", "Git", false),
            ("settings-connections", "Connections", false),
            ("settings-codex-cloud", "Cloud preferences", false),
            ("settings-codex-cloud", "Codex Cloud", false),
            ("settings-legacy-codex-cloud", "Legacy Codex Cloud", false),
            ("settings-code-review", "Code Review", false),
            ("sidebar-cmd-environments", "Environments", false),
            ("settings-worktrees", "Worktrees", false),
            ("settings-configuration", "Configuration", false),
            ("settings-profile", "Work preferences", false),
            ("settings-personalization", "Personalization", false),
            ("settings-keyboard-shortcuts", "Keyboard shortcuts", false),
            ("settings-usage", "Usage & billing", false),
            ("sidebar-cmd-analytics", "Analytics", false),
            ("sidebar-cmd-browser", "Browser", false),
            ("settings-computer-use", "Computer use", false),
            ("sidebar-cmd-mcp-servers", "MCP servers", false),
            ("settings-hooks", "Hooks", false),
            ("sidebar-cmd-plugins", "Plugins", false),
            ("settings-data-controls", "Data controls", false),
            ("settings-archived-chats", "Archived chats", false),
            ("settings-cloud-computer", "Cloud computer", false),
            ("settings-passwords", "Passwords", false),
            ("sidebar-cmd-account-external", "Account", true),
        ]
        .map(|(icon, label, external)| Item {
            external,
            ..Item::command(Some(icon), label, "")
        });
        let create_page = "Start a private page outside a Space";
        Self {
            groups: vec![
                ("Chats", chats),
                (
                    "Quick actions",
                    vec![
                        Item::command(Some("sidebar-cmd-new-chat"), "New chat", "").keys("⌘N"),
                        Item::command(Some("sidebar-cmd-folder"), "Open folder", "").keys("⌘O"),
                    ],
                ),
                ("Settings", settings.into()),
                (
                    "App",
                    vec![
                        Item::command(
                            Some("settings-profile"),
                            "Show Mini",
                            "Show Mini from anywhere",
                        )
                        .keys("⌥Space"),
                        Item::command(
                            None,
                            "Clear all unreads",
                            "Mark all chats and scheduled task updates as read",
                        )
                        .keys("⇧Esc"),
                        Item::command(
                            Some("sidebar-cmd-manage-scheduled-tasks"),
                            "Manage scheduled tasks",
                            "Create or manage scheduled tasks from the current page",
                        ),
                        Item::command(Some("sidebar-cmd-log-out"), "Log out", "Sign out of Codex"),
                        Item::command(
                            Some("sidebar-cmd-feedback"),
                            "Feedback",
                            "Send product feedback to the ChatGPT team",
                        ),
                        Item {
                            description: Some(create_page),
                            ..Item::command(
                                Some("sidebar-cmd-create-page"),
                                "Create page",
                                create_page,
                            )
                        },
                    ],
                ),
            ],
            query: query.to_owned(),
            selected: 0,
            focus: cx.focus_handle(),
        }
    }

    /// Groups with the items matching the query; empty groups are dropped.
    fn visible(&self) -> Vec<(&'static str, Vec<&Item>)> {
        self.groups
            .iter()
            .map(|(heading, items)| {
                let items = items
                    .iter()
                    .filter(|i| self.query.trim().is_empty() || i.matches(&self.query))
                    .collect::<Vec<_>>();
                (*heading, items)
            })
            .filter(|(_, items)| !items.is_empty())
            .collect()
    }

    fn on_key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        match k.key.as_str() {
            "down" => self.selected += 1,
            "up" => self.selected = self.selected.saturating_sub(1),
            "backspace" => {
                self.query.pop();
                self.selected = 0;
            }
            "escape" => return cx.emit(CommandMenuEvent::Dismiss),
            "enter" => return self.confirm(cx),
            _ => match &k.key_char {
                Some(c) if !k.modifiers.platform && !k.modifiers.control => {
                    self.query.push_str(c);
                    self.selected = 0;
                }
                _ => return,
            },
        }
        let count = self
            .visible()
            .iter()
            .map(|(_, items)| items.len())
            .sum::<usize>();
        self.selected = self.selected.min(count.saturating_sub(1));
        cx.notify();
    }

    /// Runs the selected row.
    fn confirm(&mut self, cx: &mut Context<Self>) {
        let event = self
            .visible()
            .into_iter()
            .flat_map(|(_, items)| items)
            .nth(self.selected)
            .map(|item| match &item.thread {
                Some(id) => CommandMenuEvent::OpenThread(id.clone()),
                None => CommandMenuEvent::Command(item.label.clone()),
            });
        if let Some(event) = event {
            cx.emit(event);
        }
    }
}

impl Render for CommandMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let viewport = window.viewport_size();
        let (w, h) = (f32::from(viewport.width), f32::from(viewport.height));
        // `.command-menu-dialog` / `.global-command-menu-dialog` sizing.
        let list_height = (0.9 * h - 64.).clamp(120., 440.);
        let width = 520f32.min(0.92 * w);
        let top = ((h - list_height - 64.) / 2.).max(16.);
        let surface = t.surface_elevated_secondary;
        // `border-color: var(--color-border)` in dark, transparent in light (still a 1 px
        // border: Blink paints the fill in its bleed-avoidance clip layer).
        let border_color = if t.appearance == Appearance::Dark {
            t.border
        } else {
            gpui::transparent_black()
        };
        let border = Some((px(1.), border_color));
        let query = self.query.trim();
        // The open dialog is its own composited layer (Radix's enter animation): it rasterizes
        // into transparent tiles that start at its ink overflow, i.e. the shadow's (3σ of the
        // 32 px blur, less the 8 px spread, minus the 16 px offset on top). Its ops blend in the
        // layer, which then composites once.
        let (ink_x, ink_y) = (40., 24.);
        // A row's text column: the dialog less its 1 px border and `p-1`, the row's `px-2`, and
        // the 16 px icon slot with its 8 px gap.
        let text_width = width - 2. * 5. - 2. * 8. - 16. - 8.;
        let mut index = 0;
        let groups = self.visible().into_iter().map(|(heading, items)| {
            // While a query is typed the original also searches ChatGPT conversations and
            // shows its pending spinner next to `Chats`.
            // ponytail: that remote search is not wired, so the spinner stays.
            let searching = heading == "Chats" && !query.is_empty();
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                // `pt-2` over a 13 px / 18.57 line; 26.5 keeps the rows on the original's
                // device-pixel-snapped edges.
                .child(
                    div()
                        .h(px(26.5))
                        .pt(px(8.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(text(heading, 13., 18.5, 430., t.text_tertiary))
                        .when(searching, |d| {
                            d.child(icon("sidebar-cmd-spinner", 12., t.text_tertiary))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .children(items.into_iter().map(|item| {
                            let selected = index == self.selected;
                            index += 1;
                            // `Chats` is the first group, so a chat's ordinal is `index`.
                            menu_row(item, query, index, selected, text_width, t, window).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.selected = index - 1;
                                    this.confirm(cx);
                                }),
                            )
                        })),
                )
        });
        let input = if query.is_empty() {
            text("Search chats", 14., 21., 430., t.text_tertiary)
        } else {
            text(self.query.clone(), 14., 21., 430., t.text)
        };
        div()
            .absolute()
            .inset_0()
            .bg(rgba(OVERLAY))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(CommandMenuEvent::Dismiss)),
            )
            .child(
                div()
                    .id("command-menu")
                    .track_focus(&self.focus)
                    .on_key_down(cx.listener(Self::on_key))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .absolute()
                    .left(px((w - width) / 2. - ink_x))
                    .top(px(top - ink_y))
                    .pl(px(ink_x))
                    .pt(px(ink_y))
                    .layer()
                    .child(
                        div()
                            .relative()
                            .w(px(width))
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(px(20.)),
                                fill: Some(surface),
                                border,
                                shadows: vec![shapes::Shadow {
                                    color: rgba(0x00000030).into(),
                                    offset: point(px(0.), px(16.)),
                                    blur: px(32.),
                                    spread: px(-8.),
                                    inset: false,
                                }],
                                ..Default::default()
                            }))
                            // 1 px border + `p-1`.
                            .child(
                                div()
                                    .relative()
                                    .p(px(5.))
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .overflow_hidden()
                                    .child(div().h(px(33.)).px(px(10.)).py(px(6.)).child(input))
                                    .child(
                                        div()
                                            .id("command-menu-list")
                                            .max_h(px(list_height))
                                            .overflow_y_scroll()
                                            // Its contents end 0.1 px past the box, so Chromium
                                            // scrolls and composites them in their own
                                            // transparent layer, over the dialog's (cc layers
                                            // are flat): a row's opacity restores onto it.
                                            .top_layer()
                                            .flex()
                                            .flex_col()
                                            .children(groups),
                                    ),
                            ),
                    ),
            )
    }
}

/// `#0002` `--color-dialog-overlay` over the window behind the command menu.
const OVERLAY: u32 = 0x0000_0022;

/// `color` inside nested CSS `opacity` layers (outermost last), composited on opaque `bottom`
/// the way Skia does: each layer scales the 8-bit premultiplied pixel by
/// `(v * (round(opacity * 255) + 1)) >> 8`.
fn layered(color: Hsla, opacities: &[f32], bottom: Hsla) -> Hsla {
    let (c, b) = (Rgba::from(color), Rgba::from(bottom));
    let mut a = (c.a * 255.).round() as u32;
    let premul = |v: f32| (v * 255. * a as f32 / 255.).round() as u32;
    let mut rgb = [premul(c.r), premul(c.g), premul(c.b)];
    for o in opacities {
        let scale = (o * 255.).round() as u32 + 1;
        a = (a * scale) >> 8;
        rgb = rgb.map(|v| (v * scale) >> 8);
    }
    let blend = |v: u32, b: f32| (v as f32 + (b * 255. * (255 - a) as f32 / 255.).round()) / 255.;
    Rgba {
        r: blend(rgb[0], b.r),
        g: blend(rgb[1], b.g),
        b: blend(rgb[2], b.b),
        a: 1.,
    }
    .into()
}

/// A command menu row. CSS dims unselected rows with `opacity: .75` and the `kbd` with `.8`:
/// the glyphs blend through those layers (element `opacity`); fills are precomputed with
/// [`layered`] and icons take the alpha in their color (Chromium folds a lone draw's opacity).
/// Query matches show in the text color, the rest of a filtered title in the description
/// color; a chat matching the query shows its first message under the title. `text_width`:
/// the width of the row's text column.
fn menu_row(
    item: &Item,
    query: &str,
    ordinal: usize,
    selected: bool,
    text_width: f32,
    t: &Theme,
    window: &Window,
) -> Stateful<Div> {
    let surface = t.surface_elevated_secondary;
    let (bottom, row_opacity) = if selected {
        (blit(t.ghost_hover, surface), 1.)
    } else {
        (surface, 0.75)
    };
    let (fg, dim) = (t.text, t.text_tertiary);
    let snippet = (item.is_chat() && !query.is_empty())
        .then(|| item.keywords.clone())
        .filter(|s| s.to_lowercase().contains(&query.to_lowercase()));
    let label = marked(
        item.label.clone(),
        &highlight(&item.label, query, !item.is_chat()),
        14.,
        21.,
        fg,
        dim,
    )
    .opacity(row_opacity);
    let line = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .map(|d| match item.description {
            Some(description) => d
                .child(label.flex_none().max_w(relative(0.6)).truncate())
                .child(
                    text(description, 13., 130. / 7., 430., dim)
                        .opacity(row_opacity)
                        .flex_1()
                        .min_w_0()
                        .truncate(),
                ),
            None => d.child(label.flex_1().min_w_0().truncate()),
        })
        .when_some(item.project.clone(), |d, project| {
            d.child(
                div()
                    .w(px(96.))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .child(text(project, 13., 130. / 7., 430., dim).opacity(row_opacity)),
            )
        })
        .when_some(
            item.keys
                .clone()
                .or_else(|| item.is_chat().then(|| format!("⌃{ordinal}").into())),
            |d, keys| {
                let kbd_bg = layered(t.text_alpha_10, &[0.8, row_opacity], bottom);
                // `kbd` sits on the 21 px line's baseline, 3 px below its top.
                d.child(
                    div().flex_none().h(px(21.)).pt(px(3.)).child(
                        div()
                            .relative()
                            .h(px(16.))
                            .px(px(6.))
                            .flex()
                            .items_center()
                            .child(shapes::decoration(shapes::Decoration {
                                radius: Corners::all(px(10.)),
                                fill: Some(kbd_bg),
                                ..Default::default()
                            }))
                            .child(
                                div()
                                    .relative()
                                    .opacity(row_opacity)
                                    .child(text(keys, 12., 12., 430., fg).opacity(0.8)),
                            ),
                    ),
                )
            },
        )
        .when(item.external, |d| {
            d.child(icon("arrow-up-right", 16., fg.opacity(row_opacity)))
        });
    div()
        .id(("command-row", ordinal))
        .relative()
        .py(px(5.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .when(selected, |d| {
            d.child(shapes::decoration(shapes::Decoration {
                radius: Corners::all(px(12.5)),
                fill: Some(t.ghost_hover),
                ..Default::default()
            }))
        })
        .child(
            div().size(px(16.)).flex_none().children(
                item.icon
                    .map(|name| icon(name, 16., fg.opacity(row_opacity))),
            ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(line)
                .when_some(snippet, |d, snippet| {
                    // The original wraps every char in its own span: each starts where the
                    // previous span's LayoutUnit-ceiled width ends. `text-overflow: ellipsis`
                    // then drops the spans that leave no room for the `…` in `text_width`.
                    let marks = highlight(&snippet, query, false);
                    let ellipsis = span_widths("…", 12., window)[0];
                    let mut x = 0.;
                    let kept: Vec<_> = snippet
                        .chars()
                        .zip(marks)
                        .zip(span_widths(&snippet, 12., window))
                        .take_while(|&(_, w)| {
                            x += w;
                            x + ellipsis <= text_width
                        })
                        .collect();
                    let cut = kept.len() < snippet.chars().count();
                    let span = |s: String, w: f32, c: Hsla| {
                        div().flex_none().w(px(w)).child(text(s, 12., 16., 430., c))
                    };
                    d.child(
                        div()
                            .pt(px(2.))
                            .flex()
                            .overflow_hidden()
                            .opacity(row_opacity)
                            .children(kept.into_iter().map(|((c, m), w)| {
                                span(c.to_string(), w, if m { fg } else { dim })
                            }))
                            .when(cut, |d| d.child(span("…".into(), ellipsis, dim))),
                    )
                }),
        )
}

/// Width of each char of `s` (430 weight, `size` px) inside the run shaped whole, ceiled to
/// Blink's LayoutUnit (1/64 device px) as an inline span's width is.
fn span_widths(s: &str, size: f32, window: &Window) -> Vec<f32> {
    let mut font = window.text_style().font();
    font.weight = FontWeight(430.);
    let run = TextRun {
        len: s.len(),
        font,
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(s.to_string().into(), px(size), &[run], None);
    let unit = window.scale_factor() * 64.;
    let ends = s.char_indices().skip(1).map(|(i, _)| i).chain([s.len()]);
    s.char_indices()
        .zip(ends)
        .map(|((start, _), end)| {
            let w = f32::from(line.x_for_index(end) - line.x_for_index(start));
            (w * unit).ceil() / unit
        })
        .collect()
}

/// Home with the sidebar `Search` command menu open and `math` typed.
pub fn build(window: &mut Window, cx: &mut App) -> AnyView {
    let sidebar = home_sidebar(cx, None);
    let main = super::home::home_main(window, cx);
    let shell = cx.new(|_| Shell::new(sidebar, main));
    let menu = cx.new(|cx| CommandMenu::new("math", cx));
    let focus = menu.read(cx).focus.clone();
    focus.focus(window, cx);
    cx.new(|_| Overlaid {
        base: shell.into(),
        top: menu.into(),
    })
    .into()
}

/// A view drawn over another, filling the window (the command menu over the shell).
struct Overlaid {
    base: AnyView,
    top: AnyView,
}

impl Render for Overlaid {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .child(self.base.clone())
            .child(self.top.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layered_matches_skia() {
        let t = Theme::dark();
        let red = |c: Hsla| (Rgba::from(c).r * 255.).round() as u8;
        let selected = blit(t.ghost_hover, t.surface_elevated_secondary);
        // refs/sidebar/dark@1280x832.png: `kbd` fill on the selected row 76, on the others 57.
        assert_eq!(red(layered(t.text_alpha_10, &[0.8, 1.], selected)), 76);
        assert_eq!(
            red(layered(
                t.text_alpha_10,
                &[0.8, 0.75],
                t.surface_elevated_secondary
            )),
            57
        );
    }

    #[test]
    fn math_query_matches_like_the_original() {
        // refs/sidebar: `math` keeps the subtract chat and `Manage scheduled tasks` only.
        assert!(fuzzy_match(
            "Manage scheduled tasks Create or manage scheduled tasks from the current page",
            "math"
        ));
        assert!(!fuzzy_match(
            "Clear all unreads Mark all chats and scheduled task updates as read",
            "math"
        ));
        assert!(!fuzzy_match("Computer history ", "math"));
        let marks = highlight("Manage scheduled tasks", "math", true);
        let lit: String = "Manage scheduled tasks"
            .chars()
            .zip(marks)
            .filter_map(|(c, m)| m.then_some(c))
            .collect();
        assert_eq!(lit, "Mat");
    }
}
